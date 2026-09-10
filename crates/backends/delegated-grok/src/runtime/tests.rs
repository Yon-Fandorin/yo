//! Runtime test support shared by lifecycle, event, and session coverage.

mod events;
mod lifecycle;
mod session;

use std::{cell::RefCell, collections::VecDeque, rc::Rc, time::Duration};

use serde_json::{Value, json};
use yo_backend::transport::JsonMessagePeer;
use yo_core::{
    ActivityKind, ActivityOutcome, ActivityResponse, AgentCommand, ApprovalDecision,
    BackendBindingEvidence, BackendCommandEvidence, BackendEvent, BackendFailure,
    BackendFailureKind, BackendIdentity, BackendPoll, ContinuationStrategy, SessionId, TurnOutcome,
    TurnRef, UserInput,
};

use super::{Backend, GrokBackend};
use crate::{client::AcpClient, config::GrokBackendConfig, transport::PeerPoll};

#[derive(Clone)]
struct Sent(Rc<RefCell<Vec<Value>>>);

struct FakePeer {
    incoming: VecDeque<Result<PeerPoll, BackendFailure>>,
    sent: Sent,
}

impl FakePeer {
    fn new(messages: impl IntoIterator<Item = Value>) -> (Self, Sent) {
        let sent = Sent(Rc::new(RefCell::new(Vec::new())));
        (
            Self {
                incoming: messages
                    .into_iter()
                    .map(|message| Ok(PeerPoll::Message(message)))
                    .collect(),
                sent: sent.clone(),
            },
            sent,
        )
    }
}

impl JsonMessagePeer for FakePeer {
    fn stop_handle(&self) -> yo_core::BackendStopHandle {
        yo_core::BackendStopHandle::no_op()
    }

    fn send(&mut self, message: &Value) -> Result<(), BackendFailure> {
        self.sent.0.borrow_mut().push(message.clone());
        Ok(())
    }

    fn receive(&mut self, _timeout: Duration) -> Result<PeerPoll, BackendFailure> {
        self.incoming.pop_front().unwrap_or(Ok(PeerPoll::Closed))
    }

    fn try_receive(&mut self) -> Result<PeerPoll, BackendFailure> {
        self.incoming.pop_front().unwrap_or(Ok(PeerPoll::Pending))
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        Ok(())
    }
}

fn initialize_response(id: u64, auth_methods: &[&str], load_session: bool) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "protocolVersion": 1,
            "agentCapabilities": { "loadSession": load_session },
            "authMethods": auth_methods
                .iter()
                .map(|method| json!({ "id": method, "name": method }))
                .collect::<Vec<_>>(),
            "agentInfo": { "name": "grok", "version": "1.0.5" }
        }
    })
}

fn response(id: u64, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: u64, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

fn skills_reload_ack(reloaded: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": "skills-reload",
        "result": { "result": { "reloaded": reloaded } }
    })
}

fn session(value: u64) -> SessionId {
    let uuid = uuid::Uuid::from_u128(0x0189_0f00_0000_7000_8000_0000_0000_0000 | u128::from(value));
    SessionId::from_uuid(uuid).expect("the test Session fixture is a UUIDv7")
}

fn turn(session_id: SessionId, value: u64) -> TurnRef {
    TurnRef::new(
        session_id,
        yo_core::TurnId::new(std::num::NonZeroU64::new(value).unwrap()),
    )
}

fn backend(later_messages: impl IntoIterator<Item = Value>) -> (Backend<FakePeer>, Sent) {
    backend_with_profile(later_messages, false)
}

fn backend_with_profile(
    later_messages: impl IntoIterator<Item = Value>,
    read_only_review: bool,
) -> (Backend<FakePeer>, Sent) {
    let messages = [
        vec![
            initialize_response(1, &["cached_token", "grok.com"], true),
            response(2, json!({})),
        ],
        later_messages.into_iter().collect(),
    ]
    .concat();
    let (peer, sent) = FakePeer::new(messages);
    (
        Backend::new_uninitialized(
            AcpClient::new(peer, Duration::from_secs(1)),
            "/workspace".to_owned(),
            read_only_review,
        ),
        sent,
    )
}

fn session_update(kind: &str, mut update: Value) -> Value {
    update["sessionUpdate"] = Value::String(kind.to_owned());
    json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": { "sessionId": "grok-session-a", "update": update }
    })
}

fn text_update(kind: &str, text: &str) -> Value {
    session_update(kind, json!({ "content": { "type": "text", "text": text } }))
}

fn tool_call(tool_id: &str, status: &str, title: Option<&str>) -> Value {
    let mut update = json!({ "toolCallId": tool_id, "status": status });
    if let Some(title) = title {
        update["title"] = Value::String(title.to_owned());
    }
    session_update("tool_call", update)
}

fn tool_result(tool_id: &str, text: &str) -> Value {
    session_update(
        "tool_call_update",
        json!({
            "toolCallId": tool_id,
            "name": "terminal",
            "content": [{
                "type": "content",
                "content": { "type": "text", "text": text }
            }],
            "status": "completed"
        }),
    )
}

fn tool_raw_input_update(tool_id: &str, raw_input: Value) -> Value {
    session_update(
        "tool_call_update",
        json!({
            "toolCallId": tool_id,
            "rawInput": raw_input,
            "status": "in_progress"
        }),
    )
}

fn permission_request(id: &str, title: Option<&str>) -> Value {
    let mut params = json!({
        "sessionId": "grok-session-a",
        "options": [
            { "optionId": "once", "kind": "allow_once" },
            { "optionId": "reject", "kind": "reject_once" }
        ]
    });
    if let Some(title) = title {
        params["toolCall"] = json!({ "title": title });
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "session/request_permission",
        "params": params
    })
}

fn expect_activity_started(
    backend: &mut Backend<FakePeer>,
    expected: ActivityKind,
) -> yo_core::ActivityRef {
    match backend.poll_event().unwrap() {
        BackendPoll::Event(BackendEvent::ActivityStarted { activity, kind }) => {
            assert_eq!(kind, expected);
            activity
        },
        other => panic!("expected {expected:?} ActivityStarted, got {other:?}"),
    }
}

fn expect_activity_update(backend: &mut Backend<FakePeer>, activity: yo_core::ActivityRef) {
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityUpdated { activity: observed, .. })
            if observed == activity
    ));
}

fn create_session(backend: &mut Backend<FakePeer>, session_id: SessionId) {
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
}

fn resume_binding(grok_session: &str) -> BackendBindingEvidence {
    BackendBindingEvidence::new(
        "grok-build-acp",
        "grok/recorded",
        BackendIdentity::new(
            "grok.acp/session-binding/v1",
            json!({ "sessionId": grok_session }).to_string(),
        ),
        BackendIdentity::new("grok.build/model-selection/v1", "backend-managed"),
        BackendIdentity::new("grok.acp/session-locator/v1", grok_session),
        ContinuationStrategy::BackendManagedState,
    )
}

fn read_only_resume_binding(grok_session: &str) -> BackendBindingEvidence {
    BackendBindingEvidence::new(
        "grok-build-acp",
        "grok/recorded",
        BackendIdentity::new(
            "grok.acp/session-binding/v1alpha1",
            json!({
                "executionProfile": "yo.delegated-review-execution/v1alpha1",
                "sessionId": grok_session,
            })
            .to_string(),
        ),
        BackendIdentity::new("grok.build/model-selection/v1", "backend-managed"),
        BackendIdentity::new("grok.acp/session-locator/v1", grok_session),
        ContinuationStrategy::BackendManagedState,
    )
}

// 새 Session은 ACP v1 초기화 뒤 cached_token만 인증하고 session/new를 호출하며,
// 반환된 Grok Session ID를 backend-managed Continuation 신원으로 보존합니다.
