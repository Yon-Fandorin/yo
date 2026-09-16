use std::{
    collections::VecDeque,
    io,
    num::NonZeroU64,
    task::{Context, Poll},
    thread,
    time::{Duration, Instant},
};

use serde_json::json;
use yo_core::{
    ActivityApproval, ActivityDocument, ActivityId, ActivityKind, ActivityNotice, ActivityOutcome,
    ActivityQuestion, ActivityRef, ActivityRequestRef, ActivityUpdate, AgentCommand,
    AgentControlOutcome, AgentEvent, ApprovalChoice, ApprovalDecision, Failure, NoticeLevel,
    RequestId, SessionId, SubmissionOutcome, SubmissionRejection, SubmissionRejectionKind,
    ToolOutput, TranscriptRecord, TurnId, TurnOutcome, TurnRef,
};

use super::{
    super::TuiDocument,
    media::media_response,
    scenario::{approval_profile, interview_prompt, response},
};
use crate::runner::{
    AgentAction, AgentConnection, AgentPoll, DispatchOutcome, PendingDispatch, TuiStatusLine,
};

#[derive(Debug)]
pub(in crate::runner) struct TestAgent {
    ready: VecDeque<AgentPoll>,
    stream: VecDeque<AgentEvent>,
    active: Option<ActivityRef>,
    session: SessionId,
    turns: u64,
    next: Instant,
    armed: bool,
    request: Option<PreviewRequest>,
    interview_drafts: [Option<(Option<u32>, String)>; 2],
    interview_sequence: u64,
}

#[derive(Debug)]
enum PreviewRequest {
    Approval(ActivityRequestRef, PreviewApproval),
    Interview {
        request: ActivityRequestRef,
        first_answer: Option<String>,
        first: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PreviewApproval {
    Plain,
    Scopes,
    FileChange,
}

impl PreviewApproval {
    fn profile(self) -> Option<ActivityApproval> {
        match self {
            Self::Plain => None,
            Self::Scopes => Some(approval_profile()),
            Self::FileChange => Some(ActivityApproval {
                related_change: Some(2),
                plain_text: "Review the proposed file changes with /changes.\nOffline preview: no files or permissions are changed.".into(),
                choices: vec![
                    ApprovalChoice { label:"Approve request".into(), description:"Accept this offline proposal".into(), enabled:true },
                    ApprovalChoice { label:"Decline".into(), description:"Leave the proposal unapplied".into(), enabled:true },
                ],
                decline_choice:Some(2),
            }),
        }
    }
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
            request: None,
            interview_drafts: [None, None],
            interview_sequence: 100,
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

    fn ask(&mut self, activity: ActivityRef, kind: ActivityKind, text: &str) {
        self.active = Some(activity);
        self.event(AgentEvent::ActivityStarted { activity, kind });
        self.event(AgentEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextSnapshot(text.to_owned()),
        });
    }

    fn next_interview_activity(&mut self, turn: TurnRef) -> Result<ActivityRef, io::Error> {
        self.interview_sequence = self
            .interview_sequence
            .checked_add(1)
            .ok_or_else(|| io::Error::other("preview question identity exhausted"))?;
        Ok(ActivityRef::new(
            turn,
            ActivityId::new(NonZeroU64::new(self.interview_sequence).unwrap()),
        ))
    }

    fn show_interview(
        &mut self,
        turn: TurnRef,
        first: bool,
        first_answer: Option<String>,
    ) -> Result<(), io::Error> {
        let activity = self.next_interview_activity(turn)?;
        let request_id = RequestId::new(activity.activity_id().get());
        let mut question = ActivityQuestion::from_snapshot(&interview_prompt(first))
            .expect("structured preview question");
        question.previous_question = !first;
        if let Some((choice, draft)) = &self.interview_drafts[usize::from(!first)] {
            question.draft_choice = *choice;
            question.draft = Some(draft.clone());
        }
        let text = question
            .to_snapshot()
            .ok_or_else(|| io::Error::other("preview draft exceeds question display limit"))?;
        self.request = Some(PreviewRequest::Interview {
            request: ActivityRequestRef::new(activity, request_id),
            first_answer,
            first,
        });
        self.ask(
            activity,
            ActivityKind::UserInputRequest { request_id },
            &text,
        );
        Ok(())
    }

    fn answer_interview(
        &mut self,
        request: ActivityRequestRef,
        input: String,
        choice: Option<u32>,
    ) -> Result<(), io::Error> {
        let Some(PreviewRequest::Interview {
            request: current,
            first,
            first_answer,
        }) = &self.request
        else {
            return Ok(());
        };
        if *current != request {
            return Ok(());
        }
        let first = *first;
        let first_answer = first_answer.clone();
        let question = ActivityQuestion::from_snapshot(&interview_prompt(first))
            .expect("structured preview question");
        let resolved = match choice {
            Some(choice) => {
                let selected = choice
                    .checked_sub(1)
                    .and_then(|index| question.choices.get(index as usize))
                    .ok_or_else(|| io::Error::other("preview choice is unavailable"))?;
                if input.trim().is_empty() {
                    selected.label.clone()
                } else {
                    format!("{}\nNote: {}", selected.label, input)
                }
            },
            None => input
                .trim()
                .parse::<usize>()
                .ok()
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| question.choices.get(index))
                .map_or(input.clone(), |choice| choice.label.clone()),
        };
        self.interview_drafts[usize::from(!first)] = Some((choice, input));
        let receipt = self.next_interview_activity(request.activity().turn())?;
        self.event(AgentEvent::ActivityStarted {
            activity: receipt,
            kind: ActivityKind::UserInputResponse {
                request_id: request.request_id(),
            },
        });
        self.event(AgentEvent::ActivityUpdated {
            activity: receipt,
            update: ActivityUpdate::TextSnapshot(format!(
                "Question {} of 2\nAnswer: {resolved}\n\nOffline preview · {}",
                if first { 1 } else { 2 },
                if first {
                    "one question remains."
                } else {
                    "both answers recorded."
                }
            )),
        });
        self.event(AgentEvent::ActivityFinished {
            activity: receipt,
            outcome: ActivityOutcome::Completed,
        });
        if first {
            self.event(AgentEvent::ActivityFinished {
                activity: request.activity(),
                outcome: ActivityOutcome::Completed,
            });
            self.show_interview(request.activity().turn(), false, Some(resolved))?;
        } else {
            self.finish_answer(request, format!("## Preferences recorded\n\nPriority: {}\n\nDensity: {resolved}\n\nBoth answers stayed in this offline preview.", first_answer.expect("first question answered")));
        }
        Ok(())
    }

    fn finish_answer(&mut self, request: ActivityRequestRef, text: String) {
        self.request = None;
        self.active = None;
        self.event(AgentEvent::ActivityFinished {
            activity: request.activity(),
            outcome: ActivityOutcome::Completed,
        });
        let activity = ActivityRef::new(
            request.activity().turn(),
            ActivityId::new(NonZeroU64::new(10).unwrap()),
        );
        self.event(AgentEvent::ActivityStarted {
            activity,
            kind: ActivityKind::AgentMessage,
        });
        self.event(AgentEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextSnapshot(text),
        });
        self.event(AgentEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed,
        });
        self.event(AgentEvent::TurnFinished {
            turn: activity.turn(),
            outcome: TurnOutcome::Completed,
        });
    }

    fn interrupt(&mut self) {
        let interrupted_request = self.request.take();
        if let Some(activity) = self.active.take() {
            self.stream.clear();
            self.event(AgentEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Interrupted,
            });
            if let Some(PreviewRequest::Interview { first_answer, .. }) = interrupted_request {
                let recorded = usize::from(first_answer.is_some());
                let notice = ActivityNotice {
                    level: NoticeLevel::Warning,
                    title: "Interview incomplete".to_owned(),
                    message: format!("{recorded}/2 answers recorded · {} unanswered\nOffline preview · submission incomplete.\n1. {}: What should the chat make easiest?\n2. Unanswered: How much detail should stay visible?", 2 - recorded, if recorded == 1 { "Recorded" } else { "Unanswered" }),
                }.to_snapshot().expect("bounded offline summary");
                let summary = ActivityRef::new(
                    activity.turn(),
                    ActivityId::new(NonZeroU64::new(13).unwrap()),
                );
                self.event(AgentEvent::ActivityStarted {
                    activity: summary,
                    kind: ActivityKind::ModelWork,
                });
                self.event(AgentEvent::ActivityUpdated {
                    activity: summary,
                    update: ActivityUpdate::TextSnapshot(notice),
                });
                self.event(AgentEvent::ActivityFinished {
                    activity: summary,
                    outcome: ActivityOutcome::Completed,
                });
            }
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
                if input.trim() == "session-document" {
                    let document = ActivityDocument { title: "Workspace guide".into(), markdown: "This **host document** did not start a model turn.\n\n| Action | Shortcut |\n| --- | --- |\n| Inspect output | `/output` |\n| Review changes | `/changes` |\n\n```rust\nuse std::path::Path;\n```".into() };
                    self.ready.push_back(AgentPoll::Document(TuiDocument::new(document).expect("bounded session document").with_expanded(true)));
                    return Ok(DispatchOutcome::Queued);
                }
                if matches!(input.trim(), "status" | "status-update" | "status-clear") {
                    let status = match input.trim() {
                        "status" => TuiStatusLine::new([
                            ("branch", "Demo branch: chat-ui"),
                            ("checks", "Demo checks: 18 passed"),
                            ("worker", "Demo worker: idle"),
                        ]).expect("bounded offline status"),
                        "status-update" => TuiStatusLine::new([
                            ("checks", "Demo checks: 20 passed"),
                            ("worker", "Demo worker: ready"),
                        ]).expect("bounded offline status"),
                        _ => TuiStatusLine::default(),
                    };
                    self.ready.push_back(AgentPoll::StatusLine(status));
                    return Ok(DispatchOutcome::Queued);
                }
                let notice = match input.trim() {
                    "warning" => Some(("Codex configuration warning", "Preview only: an unsupported setting was ignored.\nFile: config.toml\nThis session notice does not start a model turn.")),
                    "deprecation" => Some(("Codex deprecation notice", "Preview only: a legacy setting will be removed.\nDetails: migrate to the replacement setting before upgrading.\nFile: config.toml")),
                    "approval-warning" => Some(("Codex approval warning", "Preview only: automatic approval review is unavailable.\nRead the request scope before choosing an action.\nNo command was executed and no permission was changed.")),
                    _ => None,
                };
                if let Some((title, message)) = notice {
                    self.ready.push_back(AgentPoll::Notice(ActivityNotice {
                        title: title.to_owned(),
                        message: message.to_owned(),
                        level: NoticeLevel::Warning,
                    }));
                    return Ok(DispatchOutcome::Queued);
                }
                self.turns += 1;
                let turn = TurnRef::new(self.session, TurnId::new(NonZeroU64::new(self.turns).unwrap()));
                let activity = ActivityRef::new(turn, ActivityId::new(NonZeroU64::MIN));
                self.ready.push_back(AgentPoll::Record(TranscriptRecord::CommandCommitted(
                    AgentCommand::StartTurn { turn, input: submission.into_input() })));
                self.event(AgentEvent::TurnStarted { turn });
                if input.trim() == "proposed-plan" {
                    self.active=Some(activity);
                    self.event(AgentEvent::ActivityStarted{activity,kind:ActivityKind::ModelWork});
                    self.event(AgentEvent::ActivityUpdated{activity,update:ActivityUpdate::TextSnapshot(ActivityDocument{title:"Proposed plan".to_owned(),markdown:"## Draft\n\nInspecting the current implementation…".to_owned()}.to_snapshot().expect("bounded preview document"))});
                    self.stream.push_back(AgentEvent::ActivityUpdated{activity,update:ActivityUpdate::TextSnapshot(response("proposed-plan").1)});
                    self.stream.push_back(AgentEvent::ActivityFinished{activity,outcome:ActivityOutcome::Completed});
                    self.stream.push_back(AgentEvent::TurnFinished{turn,outcome:TurnOutcome::Completed});
                    self.next=Instant::now()+Duration::from_secs(1);
                    self.armed=false;
                    return Ok(DispatchOutcome::Queued);
                }
                if input.trim() == "shell-progress" {
                    self.active = Some(activity);
                    self.event(AgentEvent::ActivityStarted {activity,kind:ActivityKind::ToolCall});
                    let output = ToolOutput {tool:"run_command".to_owned(),server:Some("preview".to_owned()),arguments:Some(json!({"command":"cargo test --workspace"})),result:Some(json!({"content":[{"type":"text","text":json!({"stdout":"First output while the command is running.\nOffline fixture: no command was executed.","stderr":""}).to_string()}],"progress":true})),content_items:None,error:None,plain_text:"Offline command progress fixture".to_owned()};
                    self.event(AgentEvent::ActivityUpdated {activity,update:ActivityUpdate::TextSnapshot(output.to_snapshot().expect("bounded progress fixture"))});
                    self.stream.push_back(AgentEvent::ActivityUpdated {activity,update:ActivityUpdate::TextSnapshot(response("shell").1)});
                    self.stream.push_back(AgentEvent::ActivityFinished {activity,outcome:ActivityOutcome::Completed});
                    self.stream.push_back(AgentEvent::TurnFinished {turn,outcome:TurnOutcome::Completed});
                    self.next=Instant::now()+Duration::from_secs(1);
                    self.armed=false;
                    return Ok(DispatchOutcome::Queued);
                }
                if input.trim() == "compaction" {
                    self.active = Some(activity);
                    self.event(AgentEvent::ActivityStarted { activity, kind: ActivityKind::ModelWork });
                    self.event(AgentEvent::ActivityUpdated { activity, update: ActivityUpdate::TextSnapshot(ActivityNotice {
                        title: "Compacting context".to_owned(),
                        message: "Preview only: context compaction in progress.".to_owned(),
                        level: NoticeLevel::Info,
                    }.to_snapshot().expect("bounded preview notice")) });
                    self.stream.push_back(AgentEvent::ActivityUpdated { activity, update: ActivityUpdate::TextSnapshot(response("compaction").1) });
                    self.stream.push_back(AgentEvent::ActivityFinished { activity, outcome: ActivityOutcome::Completed });
                    self.stream.push_back(AgentEvent::TurnFinished { turn, outcome: TurnOutcome::Completed });
                    self.next = Instant::now() + Duration::from_secs(1);
                    self.armed = false;
                    return Ok(DispatchOutcome::Queued);
                }
                if matches!(input.trim(), "approval" | "approval-scopes" | "approval-diff") {
                    let request_id = RequestId::new(NonZeroU64::MIN);
                    let kind = match input.trim() {
                        "approval-scopes" => PreviewApproval::Scopes,
                        "approval-diff" => PreviewApproval::FileChange,
                        _ => PreviewApproval::Plain,
                    };
                    if kind == PreviewApproval::FileChange {
                        for (id, path) in [(2, "src/proposed.rs"), (3, "src/unrelated.rs")] {
                            let change = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(id).unwrap()));
                            self.event(AgentEvent::ActivityStarted { activity:change, kind:ActivityKind::FileChange });
                            self.event(AgentEvent::ActivityUpdated { activity:change, update:ActivityUpdate::TextSnapshot(format!("update: {path}\n@@ -1 +1 @@\n-let ready = false;\n+let ready = true;")) });
                            self.event(AgentEvent::ActivityFinished { activity:change, outcome:ActivityOutcome::Completed });
                        }
                    }
                    self.request = Some(PreviewRequest::Approval(ActivityRequestRef::new(activity, request_id), kind));
                    if let Some(profile) = kind.profile() {
                        self.ask(activity, ActivityKind::ApprovalRequest { request_id }, &profile.to_snapshot().expect("bounded preview approval"));
                        return Ok(DispatchOutcome::Queued);
                    }
                    self.ask(activity, ActivityKind::ApprovalRequest { request_id },
                        "Run the layout checks?\n\nCommand: cargo test --locked -p yo-tui\nWorking directory: /workspace/yo\nAdditional permissions: {\"network\":{\"enabled\":false}}\nScope: requested command\nReason: verify rendering and keyboard interactions.\n\nPreview only: no command will run.");
                    return Ok(DispatchOutcome::Queued);
                }
                if input.trim() == "interview" {
                    let request_id = RequestId::new(NonZeroU64::MIN);
                    self.request = Some(PreviewRequest::Interview { request: ActivityRequestRef::new(activity, request_id), first_answer: None, first: true });
                    self.interview_drafts = [None, None];
                    self.interview_sequence = 100;
                    self.ask(activity, ActivityKind::UserInputRequest { request_id },
                        &interview_prompt(true));
                    return Ok(DispatchOutcome::Queued);
                }
                if let Some((kind, response)) = media_response(input.trim()).map(|text| (ActivityKind::AgentMessage, text))
                    .or_else(|| matches!(input.trim(), "agent-tasks" | "file-links" | "usage" | "plan" | "reasoning" | "agent-reasoning" | "tool-diff" | "mcp-image" | "retry" | "reroute" | "turn-diff" | "summary" | "branch" | "terminal-wait" | "terminal-input" | "turn-duration" | "file-read" | "file-write" | "files-read" | "file-edit" | "shell" | "codex-shell" | "shell-tail" | "shell-truncated" | "shell-retained" | "files-list" | "files-find" | "content-search" | "resource-link" | "embedded-resource").then(|| response(input.trim()))) {
                    self.event(AgentEvent::ActivityStarted { activity, kind });
                    self.event(AgentEvent::ActivityUpdated { activity, update: ActivityUpdate::TextSnapshot(response) });
                    self.event(AgentEvent::ActivityFinished { activity, outcome: ActivityOutcome::Completed });
                    self.event(AgentEvent::TurnFinished { turn, outcome: TurnOutcome::Completed });
                    return Ok(DispatchOutcome::Queued);
                }
                let (kind, response) = response(&input);
                self.event(AgentEvent::ActivityStarted { activity, kind });
                self.active = Some(activity);
                for chunk in response.split_inclusive(' ') {
                    self.stream.push_back(AgentEvent::ActivityUpdated { activity,
                        update: ActivityUpdate::TextDelta(chunk.to_owned()) });
                }
                let failed = matches!(input.trim(), "error" | "mcp-failure");
                self.stream.push_back(AgentEvent::ActivityFinished { activity, outcome: if failed {
                    ActivityOutcome::Failed(Failure::new("Simulated tool failure; no command was executed."))
                } else { ActivityOutcome::Completed } });
                self.stream.push_back(AgentEvent::TurnFinished { turn, outcome: if failed {
                    TurnOutcome::Failed(Failure::new("Preview failure. Send another message to recover."))
                } else { TurnOutcome::Completed } });
                self.next = Instant::now();
            },
            AgentAction::RespondToApproval { request, decision } => {
                if let Some(PreviewRequest::Approval(current, kind)) = self.request
                    && current == request
                {
                    let (decision_text, declined) = match decision {
                        ApprovalDecision::Approved if kind == PreviewApproval::Plain => ("approved".to_owned(), false),
                        ApprovalDecision::Declined if kind == PreviewApproval::Plain => ("declined".to_owned(), true),
                        ApprovalDecision::Offered(choice) if kind != PreviewApproval::Plain => {
                            let profile = kind.profile().expect("structured preview approval");
                            let selected = choice.checked_sub(1).and_then(|index| profile.choices.get(index as usize))
                                .filter(|choice| choice.enabled).ok_or_else(|| io::Error::other("unsupported preview approval choice"))?;
                            (format!("{}\n{}", selected.label, selected.description), profile.decline_choice == Some(choice))
                        },
                        _ => return Err(io::Error::other("approval response does not match preview choices")),
                    };
                    let receipt = ActivityRef::new(request.activity().turn(), ActivityId::new(NonZeroU64::new(14).unwrap()));
                    self.event(AgentEvent::ActivityStarted { activity: receipt, kind: ActivityKind::ApprovalResponse { request_id: request.request_id() } });
                    self.event(AgentEvent::ActivityUpdated { activity: receipt, update: ActivityUpdate::TextSnapshot(format!("Decision: {decision_text}")) });
                    self.event(AgentEvent::ActivityFinished { activity: receipt, outcome: ActivityOutcome::Completed });
                    let text = if declined {
                        "## Declined\n\nThe action was not run. You can continue the conversation."
                    } else {
                        "## Approval recorded\n\nThe decision stayed in this offline preview. No command was executed. No permission or policy was changed."
                    };
                    self.finish_answer(request, text.to_owned());
                }
            },
            AgentAction::PreviousQuestion { request, choice, draft } => {
                let Some(PreviewRequest::Interview { request: current, first: false, first_answer }) = &self.request else {
                    return Err(io::Error::other("previous preview question is unavailable"));
                };
                if *current != request || choice.is_some_and(|choice| choice == 0 || choice > 2) {
                    return Err(io::Error::other("previous preview question or choice is stale"));
                }
                let first_answer = first_answer.clone();
                self.interview_drafts[1] = Some((choice, draft));
                self.event(AgentEvent::ActivityFinished { activity: request.activity(), outcome: ActivityOutcome::Completed });
                self.show_interview(request.activity().turn(), true, first_answer)?;
            },
            AgentAction::RespondToQuestion { request, choice, notes } => {
                self.answer_interview(request, notes, Some(choice))?;
            },
            AgentAction::RespondToUserInput { request, input } => {
                self.answer_interview(request, input, None)?;
            },
            AgentAction::Interrupt => self.interrupt(),
            _ => self.ready.push_back(AgentPoll::Control(AgentControlOutcome::ContextCompactionRejected {
                detail: "This offline test agent supports chat, tools, long-tools, error, long, markdown, tables, diff, approval, interview, and interruption only.".to_owned() })),
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
            thread::spawn(move || {
                thread::sleep(delay);
                waker.wake();
            });
        }
        Poll::Pending
    }
}

#[cfg(test)]
mod tests;
