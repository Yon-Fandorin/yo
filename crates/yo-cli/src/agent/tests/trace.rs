use std::{
    thread,
    time::{Duration, Instant},
};

use yo_core::{
    AgentCommand, AgentIntent, BackendBindingEvidence, BackendCommandEvidence, BackendEvent,
    BackendIdentity, BackendOutcomeEvidence, BackendRequestEvidence, BackendScriptStep,
    RequestTraceRecord, ScriptedBackend, UserInput,
};
use yo_tui::{AgentConnection, AgentPoll};

use super::support::{NeverTerminated, session_id, turn};
use crate::agent::TuiAgentConnection;

// CLI adapter는 worker 알림 하나에서 Transcript와 payload-free Request trace를 함께
// 끝까지 배출하여, TUI가 backend나 저장소 내부 타입을 직접 읽지 않게 합니다.
#[test]
fn exposes_live_request_trace_through_the_frontend_connection() {
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::CreateSession {
                session_id: session_id(),
            },
            evidence: BackendCommandEvidence::BindingOpened(BackendBindingEvidence::new(
                "codex-app-server",
                "test",
                BackendIdentity::new("binding/v1", "binding"),
                BackendIdentity::new("model/v1", "model"),
                BackendIdentity::new("session/v1", "session"),
                yo_core::ContinuationStrategy::BackendManagedState,
            )),
        },
        BackendScriptStep::AcceptCommandWithEvidence {
            command: AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::new("inspect"),
            },
            evidence: BackendCommandEvidence::RequestAccepted(BackendRequestEvidence::new(
                "payload/v1",
                BackendIdentity::new("exchange/v1", "exchange"),
                BackendIdentity::new("request/v1", "request"),
            )),
        },
        BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
            turn: turn(),
            evidence: BackendOutcomeEvidence::with_identity(BackendIdentity::new(
                "outcome/v1",
                "outcome",
            )),
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut termination = NeverTerminated;
    let mut connection = TuiAgentConnection::start(backend, session_id(), &mut termination)
        .unwrap()
        .unwrap();
    connection
        .dispatch(AgentIntent::submit("inspect".to_owned()).unwrap())
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut trace = Vec::new();
    while trace.len() < 5 {
        match connection.poll().unwrap() {
            AgentPoll::RequestTrace(entry) => trace.push(entry),
            AgentPoll::Closed => panic!("connection closed before Request trace was drained"),
            _ => thread::yield_now(),
        }
        assert!(
            Instant::now() < deadline,
            "live Request trace was not delivered"
        );
    }
    assert!(
        trace
            .windows(2)
            .all(|pair| pair[0].sequence() < pair[1].sequence())
    );
    assert!(matches!(
        trace[0].record(),
        RequestTraceRecord::BindingOpened { .. }
    ));
    assert!(matches!(
        trace[1].record(),
        RequestTraceRecord::ExchangeObserved { .. }
    ));
    assert!(matches!(
        trace[2].record(),
        RequestTraceRecord::RequestAccepted { .. }
    ));
    assert!(matches!(
        trace[3].record(),
        RequestTraceRecord::ResumableOutcome { .. }
    ));
    assert!(matches!(
        trace[4].record(),
        RequestTraceRecord::ContinuationAnchor { .. }
    ));
    connection.shutdown().unwrap();
}
