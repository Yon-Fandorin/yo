use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

use yo_core::{ActivityNotice, NoticeLevel};
use yo_tui::{AgentAction, AgentConnection, AgentPoll, DispatchOutcome, PendingDispatch};

use super::{CodexWarningCollector, MAX_CODEX_COMPATIBILITY_WARNINGS};
use crate::interaction::diagnostic::CliDiagnostic;

// 같은 warning은 한 번만 남기고, 서로 다른 warning은 관측된 순서로 publication합니다.
#[test]
fn codex_warning_collector_deduplicates_and_preserves_observation_order() {
    let collector = CodexWarningCollector::default();
    collector.observe_message("first".to_owned());
    collector.observe_message("first".to_owned());
    collector.observe_message("second".to_owned());

    let diagnostics = collector.take_pending_diagnostics();
    assert_eq!(
        diagnostics
            .iter()
            .map(CliDiagnostic::message)
            .collect::<Vec<_>>(),
        vec!["first", "second"]
    );
    assert!(collector.take_pending_diagnostics().is_empty());
}

// 상한을 넘는 warning은 메모리와 stderr 모두 bounded하게 한 개의 suppression 진단으로
// 접습니다.
#[test]
fn codex_warning_collector_suppresses_distinct_overflow_once() {
    let collector = CodexWarningCollector::default();
    for index in 0..=MAX_CODEX_COMPATIBILITY_WARNINGS {
        collector.observe_message(format!("warning {index}"));
    }

    let diagnostics = collector.take_pending_diagnostics();
    assert_eq!(diagnostics.len(), MAX_CODEX_COMPATIBILITY_WARNINGS + 1);
    assert_eq!(
        diagnostics.last().map(CliDiagnostic::message),
        Some("additional Codex warnings were suppressed after 32 distinct warnings")
    );
    assert!(collector.take_pending_diagnostics().is_empty());
}

// stdout publication이 실패하면 이후에 도착한 Codex warning도 publication하지 않습니다.
#[test]
fn codex_warning_collector_discard_blocks_late_observations() {
    let collector = CodexWarningCollector::default();
    collector.observe_message("already pending".to_owned());
    collector.discard_pending();
    collector.observe_message("arrived after stdout failure".to_owned());

    assert!(collector.take_pending_diagnostics().is_empty());
}

struct QuietAgent;

impl AgentConnection for QuietAgent {
    type Error = io::Error;

    fn dispatch(&mut self, _: AgentAction) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Queued)
    }

    fn retry(&mut self, _: PendingDispatch) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Queued)
    }

    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        Ok(AgentPoll::Pending)
    }

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<()> {
        Poll::Pending
    }
}

#[derive(Default)]
struct WarningWake(AtomicUsize);

impl Wake for WarningWake {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

// 유휴 화면을 깨운 경고는 제목을 유지해 한 번만 표시하고 종료 후 stderr에 중복하지 않는다.
#[test]
fn session_notice_wakes_idle_frontend_and_publishes_once() {
    let collector = CodexWarningCollector::default();
    let mut agent = QuietAgent;
    let wake = Arc::new(WarningWake::default());
    let waker = Waker::from(wake.clone());
    let mut context = Context::from_waker(&waker);
    let mut pending = None;
    let mut connection = collector.connection(&mut agent, &mut pending);
    assert!(connection.poll_ready(&mut context).is_pending());
    let notice = ActivityNotice {
        title: "Codex configuration warning".to_owned(),
        message: "Unsupported setting".to_owned(),
        level: NoticeLevel::Warning,
    };
    collector.observe_warning(
        "configuration: Unsupported setting".to_owned(),
        notice.clone(),
    );
    assert_eq!(wake.0.load(Ordering::SeqCst), 1);
    assert!(connection.poll_ready(&mut context).is_ready());
    assert_eq!(connection.poll().unwrap(), AgentPoll::Notice(notice));
    assert_eq!(connection.poll().unwrap(), AgentPoll::Pending);
    assert!(collector.take_pending_diagnostics().is_empty());
    assert!(connection.poll_ready(&mut context).is_pending());
    drop(connection);
    assert!(collector.state.lock().unwrap().waker.is_none());
}

// stderr에서 먼저 알린 경고는 TUI 진입 후 되풀이하지 않고 초과 안내도 한 번만 표시한다.
#[test]
fn session_notice_shares_publication_cursor_and_bounds_overflow() {
    let collector = CodexWarningCollector::default();
    collector.observe_message("before TUI".to_owned());
    assert_eq!(collector.take_pending_diagnostics().len(), 1);
    let mut agent = QuietAgent;
    let mut pending = None;
    let mut connection = collector.connection(&mut agent, &mut pending);
    assert_eq!(connection.poll().unwrap(), AgentPoll::Pending);
    for index in 1..=MAX_CODEX_COMPATIBILITY_WARNINGS + 1 {
        collector.observe_message(format!("warning {index}"));
    }
    for _ in 1..MAX_CODEX_COMPATIBILITY_WARNINGS {
        assert!(matches!(connection.poll().unwrap(), AgentPoll::Notice(_)));
    }
    let AgentPoll::Notice(suppression) = connection.poll().unwrap() else {
        panic!("missing suppression notice");
    };
    assert!(suppression.message.contains("after 32 distinct warnings"));
    assert_eq!(connection.poll().unwrap(), AgentPoll::Pending);
    assert!(collector.take_pending_diagnostics().is_empty());
}

// 같은 수신에서 경고와 종료가 함께 도착해도 경고를 먼저 보여 주며, 화면 재진입 뒤에도
// 원래 결과를 한 번만 전달하고 이미 소비한 agent 이벤트를 다시 읽지 않는다.
#[test]
fn warnings_precede_poll_results_and_survive_frontend_reentry() {
    use yo_core::{AgentEvent, TranscriptRecord};

    struct WarningAgent {
        collector: CodexWarningCollector,
        result: Option<Result<AgentPoll, io::Error>>,
        polls: usize,
    }
    impl AgentConnection for WarningAgent {
        type Error = io::Error;
        fn dispatch(&mut self, _: AgentAction) -> Result<DispatchOutcome, Self::Error> {
            Ok(DispatchOutcome::Queued)
        }
        fn retry(&mut self, _: PendingDispatch) -> Result<DispatchOutcome, Self::Error> {
            Ok(DispatchOutcome::Queued)
        }
        fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
            self.polls += 1;
            self.collector.observe_message("first warning".into());
            self.collector.observe_message("second warning".into());
            self.result.take().expect("result consumed twice")
        }
        fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<()> {
            Poll::Pending
        }
    }
    for kind in 0..4 {
        let collector = CodexWarningCollector::default();
        let record = TranscriptRecord::EventCommitted(AgentEvent::SessionCreated {
            session_id: "01890f00-0000-7000-8000-000000000001".parse().unwrap(),
        });
        let mut agent = WarningAgent {
            collector: collector.clone(),
            result: Some(match kind {
                0 => Ok(AgentPoll::Pending),
                1 => Ok(AgentPoll::Closed),
                2 => Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "original read failure",
                )),
                _ => Ok(AgentPoll::Record(record.clone())),
            }),
            polls: 0,
        };
        let mut pending = None;
        {
            let mut connection = collector.connection(&mut agent, &mut pending);
            let AgentPoll::Notice(first) = connection.poll().unwrap() else {
                panic!("first warning was hidden by the poll result");
            };
            assert_eq!(first.message, "first warning");
        }
        assert!(pending.is_some());
        {
            let mut connection = collector.connection(&mut agent, &mut pending);
            let waker = Waker::noop();
            let mut context = Context::from_waker(waker);
            assert!(connection.poll_ready(&mut context).is_ready());
            let AgentPoll::Notice(second) = connection.poll().unwrap() else {
                panic!("second warning missing after reentry");
            };
            assert_eq!(second.message, "second warning");
            assert!(connection.poll_ready(&mut context).is_ready());
            let result = connection.poll();
            match kind {
                0 => assert_eq!(result.unwrap(), AgentPoll::Pending),
                1 => assert_eq!(result.unwrap(), AgentPoll::Closed),
                2 => {
                    let failure = result.unwrap_err();
                    assert_eq!(failure.kind(), io::ErrorKind::BrokenPipe);
                    assert_eq!(failure.to_string(), "original read failure");
                },
                _ => assert_eq!(result.unwrap(), AgentPoll::Record(record)),
            }
            assert!(connection.poll_ready(&mut context).is_pending());
        }
        assert!(pending.is_none());
        assert_eq!(agent.polls, 1);
        assert!(collector.take_pending_diagnostics().is_empty());
    }
}
