use std::{
    collections::VecDeque,
    io,
    num::NonZeroU64,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use yo_core::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentCommand,
    AgentControlOutcome, AgentEvent, Failure, SessionId, SubmissionOutcome, SubmissionRejection,
    SubmissionRejectionKind, TranscriptRecord, TurnId, TurnOutcome, TurnRef,
};

use crate::runner::{AgentAction, AgentConnection, AgentPoll, DispatchOutcome, PendingDispatch};

#[derive(Debug)]
pub(in crate::runner) struct TestAgent {
    ready: VecDeque<AgentPoll>,
    stream: VecDeque<AgentEvent>,
    active: Option<ActivityRef>,
    session: SessionId,
    turns: u64,
    next: Instant,
    armed: bool,
}

impl TestAgent {
    pub(in crate::runner) fn new() -> Self {
        Self {
            ready: VecDeque::new(),
            stream: VecDeque::new(),
            active: None,
            session: "01890f00-0000-7000-8000-000000000001".parse().unwrap(),
            turns: 0,
            next: Instant::now(),
            armed: false,
        }
    }

    fn event(&mut self, event: AgentEvent) {
        self.ready
            .push_back(AgentPoll::Record(TranscriptRecord::EventCommitted(event)));
    }

    pub(in crate::runner) fn next_deadline(&self) -> Option<Instant> {
        if !self.ready.is_empty() {
            Some(Instant::now())
        } else if !self.stream.is_empty() {
            Some(self.next)
        } else {
            None
        }
    }

    fn interrupt(&mut self) {
        if let Some(activity) = self.active.take() {
            self.stream.clear();
            self.event(AgentEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Interrupted,
            });
            self.event(AgentEvent::TurnFinished {
                turn: activity.turn(),
                outcome: TurnOutcome::Interrupted,
            });
        }
    }
}

impl AgentConnection for TestAgent {
    type Error = io::Error;

    fn dispatch(&mut self, action: AgentAction) -> Result<DispatchOutcome, Self::Error> {
        match action {
            AgentAction::Submit(submission) | AgentAction::Steer { submission, .. } => {
                if self.active.is_some() {
                    return Ok(DispatchOutcome::Rejected { id: submission.id(), rejection: SubmissionRejection::new(
                        SubmissionRejectionKind::Incompatible, "Test agent is busy. Interrupt it, then send again.") });
                }
                let input = submission.input().as_str().to_owned();
                self.ready.push_back(AgentPoll::Submission(SubmissionOutcome::Accepted { id: submission.id() }));
                self.turns += 1;
                let turn = TurnRef::new(self.session, TurnId::new(NonZeroU64::new(self.turns).unwrap()));
                let activity = ActivityRef::new(turn, ActivityId::new(NonZeroU64::MIN));
                self.ready.push_back(AgentPoll::Record(TranscriptRecord::CommandCommitted(
                    AgentCommand::StartTurn { turn, input: submission.into_input() })));
                self.event(AgentEvent::TurnStarted { turn });
                let (kind, response) = response(&input);
                self.event(AgentEvent::ActivityStarted { activity, kind });
                self.active = Some(activity);
                for chunk in response.split_inclusive(' ') {
                    self.stream.push_back(AgentEvent::ActivityUpdated { activity,
                        update: ActivityUpdate::TextDelta(chunk.to_owned()) });
                }
                let failed = input.trim() == "error";
                self.stream.push_back(AgentEvent::ActivityFinished { activity, outcome: if failed {
                    ActivityOutcome::Failed(Failure::new("Simulated tool failure; no command was executed."))
                } else { ActivityOutcome::Completed } });
                self.stream.push_back(AgentEvent::TurnFinished { turn, outcome: if failed {
                    TurnOutcome::Failed(Failure::new("Preview failure. Send another message to recover."))
                } else { TurnOutcome::Completed } });
                self.next = Instant::now();
            },
            AgentAction::Interrupt => self.interrupt(),
            _ => self.ready.push_back(AgentPoll::Control(AgentControlOutcome::ContextCompactionRejected {
                detail: "This offline test agent supports chat, tools, error, long, and interruption only.".to_owned() })),
        }
        Ok(DispatchOutcome::Queued)
    }

    fn retry(&mut self, _: PendingDispatch) -> Result<DispatchOutcome, Self::Error> {
        Err(io::Error::other(
            "preview never creates backpressured commands",
        ))
    }

    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        if let Some(record) = self.ready.pop_front() {
            return Ok(record);
        }
        if Instant::now() >= self.next
            && let Some(event) = self.stream.pop_front()
        {
            if matches!(event, AgentEvent::TurnFinished { .. }) {
                self.active = None;
            }
            self.next = Instant::now() + Duration::from_millis(70);
            self.armed = false;
            return Ok(AgentPoll::Record(TranscriptRecord::EventCommitted(event)));
        }
        Ok(AgentPoll::Pending)
    }

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        if !self.ready.is_empty() || (!self.stream.is_empty() && Instant::now() >= self.next) {
            return Poll::Ready(());
        }
        if !self.stream.is_empty() && !self.armed {
            self.armed = true;
            let delay = self.next.saturating_duration_since(Instant::now());
            let waker = cx.waker().clone();
            std::thread::spawn(move || {
                std::thread::sleep(delay);
                waker.wake();
            });
        }
        Poll::Pending
    }
}

fn response(input: &str) -> (ActivityKind, String) {
    match input.trim() {
        "tools" => (ActivityKind::ToolCall, "[SIMULATED] cargo test\nChecking layout and Unicode wrapping...\nAll preview checks passed. No process or filesystem operation was performed.".into()),
        "error" => (ActivityKind::ToolCall, "[SIMULATED] Checking a missing fixture...\nThis request deliberately fails so you can inspect error presentation.".into()),
        "long" => (ActivityKind::AgentMessage, (1..=40).map(|n| format!("Line {n}: 한글과 English, wide characters and scrolling.\n")).collect()),
        _ => (ActivityKind::AgentMessage, format!("받은 입력: {input}\n\n저는 화면 검증용 테스트 에이전트입니다. 실제 모델 호출 없이 응답을 스트리밍합니다.\n\n계속 입력해 대화를 이어가세요. tools · error · long 을 보내면 해당 UI를 확인할 수 있습니다. Esc로 중단할 수 있습니다.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // idle preview는 polling timer를 요구하지 않는다. 합성 출력이 있을 때만 깨우고,
    // 중단 후 종료 observation을 소비하면 다시 무기한 입력 대기로 돌아간다.
    #[test]
    fn preview_deadlines_exist_only_while_events_are_pending() {
        let mut agent = TestAgent::new();
        assert_eq!(agent.next_deadline(), None);
        agent
            .dispatch(AgentAction::submit("long").unwrap())
            .unwrap();
        assert!(agent.next_deadline().is_some());
        while !agent.ready.is_empty() {
            agent.poll().unwrap();
        }
        agent.next = Instant::now() + Duration::from_secs(1);
        assert_eq!(agent.next_deadline(), Some(agent.next));
        agent.dispatch(AgentAction::Interrupt).unwrap();
        while !agent.ready.is_empty() {
            agent.poll().unwrap();
        }
        assert_eq!(agent.next_deadline(), None);
    }
    // 입력을 수락하고 실제 문자열을 응답에 반영하며, 중단 후 새 대화를 받을 수 있어야 한다.
    #[test]
    fn accepts_interrupts_and_accepts_again() {
        let mut agent = TestAgent::new();
        agent
            .dispatch(AgentAction::submit("hello").unwrap())
            .unwrap();
        assert!(matches!(
            agent.poll().unwrap(),
            AgentPoll::Submission(SubmissionOutcome::Accepted { .. })
        ));
        assert!(
            agent
                .stream
                .iter()
                .any(|event| matches!(event, AgentEvent::ActivityUpdated {
            update: ActivityUpdate::TextDelta(text), .. } if text.contains("hello")))
        );
        agent.dispatch(AgentAction::Interrupt).unwrap();
        assert!(agent.stream.is_empty());
        assert!(agent.active.is_none());
        assert_eq!(
            agent
                .dispatch(AgentAction::submit("again").unwrap())
                .unwrap(),
            DispatchOutcome::Queued
        );
    }
    // 작업 중 입력은 조용히 유실시키지 않고 거절하여 frontend의 draft 보존 경로로 돌린다.
    #[test]
    fn busy_input_is_rejected() {
        let mut agent = TestAgent::new();
        agent
            .dispatch(AgentAction::submit("long").unwrap())
            .unwrap();
        assert!(matches!(
            agent
                .dispatch(AgentAction::submit("keep draft").unwrap())
                .unwrap(),
            DispatchOutcome::Rejected { .. }
        ));
    }

    // 모의 실패는 연결을 닫지 않고 Turn을 끝내 다음 입력으로 복구할 수 있어야 한다.
    #[test]
    fn failed_turn_finishes_and_allows_recovery() {
        let mut agent = TestAgent::new();
        agent
            .dispatch(AgentAction::submit("error").unwrap())
            .unwrap();
        let mut failed = false;
        for _ in 0..200 {
            agent.next = Instant::now();
            if matches!(
                agent.poll().unwrap(),
                AgentPoll::Record(TranscriptRecord::EventCommitted(AgentEvent::TurnFinished {
                    outcome: TurnOutcome::Failed(_),
                    ..
                }))
            ) {
                failed = true;
                break;
            }
        }
        assert!(failed);
        assert!(agent.active.is_none());
        assert_eq!(
            agent
                .dispatch(AgentAction::submit("recover").unwrap())
                .unwrap(),
            DispatchOutcome::Queued
        );
    }
}
