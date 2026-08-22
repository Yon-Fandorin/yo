use std::{cell::RefCell, collections::VecDeque, num::NonZeroU64, rc::Rc, time::Duration};

use serde_json::{Value, json};
use yo_backend::transport::JsonMessagePeer;
use yo_core::{ActivityId, ActivityRef, BackendFailure, SessionId, TurnId, TurnRef};

use super::super::{Backend, client::AppServerClient, transport::PeerPoll};

#[derive(Clone)]
pub(super) struct Sent(pub(super) Rc<RefCell<Vec<Value>>>);

pub(super) struct FakePeer {
    incoming: VecDeque<Result<PeerPoll, BackendFailure>>,
    sent: Sent,
}

impl FakePeer {
    pub(super) fn new(incoming: impl IntoIterator<Item = Value>) -> (Self, Sent) {
        let sent = Sent(Rc::new(RefCell::new(Vec::new())));
        (
            Self {
                incoming: incoming
                    .into_iter()
                    .map(|value| Ok(PeerPoll::Message(value)))
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

pub(super) fn initialize_response(id: u64, version: &str) -> Value {
    json!({
        "id": id,
        "result": {
            "userAgent": format!("codex_cli_rs/{version} (test)"),
            "platformFamily": "unix",
            "platformOs": "linux",
            "codexHome": "/tmp/codex-test"
        }
    })
}

pub(super) fn thread_start_response(id: u64, thread_id: &str) -> Value {
    json!({
        "id": id,
        "result": {
            "thread": { "id": thread_id, "sessionId": thread_id },
            "model": "gpt-test",
            "modelProvider": "openai"
        }
    })
}

pub(super) fn id(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

pub(super) fn submission(value: u8) -> yo_core::SubmissionId {
    yo_core::SubmissionId::from_uuid(uuid::Builder::from_random_bytes([value; 16]).into_uuid())
        .expect("the test submission fixture is a UUIDv4")
}

pub(super) fn session(value: u64) -> SessionId {
    let uuid = uuid::Uuid::from_u128(0x0189_0f00_0000_7000_8000_0000_0000_0000 | u128::from(value));
    SessionId::from_uuid(uuid).expect("the test Session fixture is a UUIDv7")
}

pub(super) fn turn(session_id: SessionId, value: u64) -> TurnRef {
    TurnRef::new(session_id, TurnId::new(id(value)))
}

pub(super) fn activity(turn: TurnRef, value: u64) -> ActivityRef {
    ActivityRef::new(turn, ActivityId::new(id(value)))
}

pub(super) fn backend(
    later_messages: impl IntoIterator<Item = Value>,
) -> (Backend<FakePeer>, Sent) {
    let messages = [
        vec![initialize_response(1, "0.146.0")],
        later_messages.into_iter().collect(),
    ]
    .concat();
    let (peer, sent) = FakePeer::new(messages);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));
    let initialize = client.initialize().unwrap();
    let mut backend = Backend::new_uninitialized(client, "/workspace".into());
    backend.initialized = true;
    backend.backend_version = Some(initialize.user_agent);
    (backend, sent)
}
