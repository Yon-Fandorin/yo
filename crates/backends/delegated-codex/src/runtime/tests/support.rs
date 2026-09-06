use std::{num::NonZeroU64, time::Duration};

use serde_json::{Value, json};
use yo_core::{AccountId, ActivityId, ActivityRef, SessionId, TurnId, TurnRef};

use super::super::Backend;
use crate::client::AppServerClient;
pub(super) use crate::test_support::{FakePeer, Sent, initialize_response};

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
    backend_with_profile(later_messages, false)
}

pub(super) fn backend_with_profile(
    later_messages: impl IntoIterator<Item = Value>,
    read_only_review: bool,
) -> (Backend<FakePeer>, Sent) {
    let messages = [
        vec![initialize_response(1, "0.146.0")],
        later_messages.into_iter().collect(),
    ]
    .concat();
    let (peer, sent) = FakePeer::new(messages);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));
    let initialize = client.initialize().unwrap();
    let mut backend =
        Backend::new_uninitialized(client, "/workspace".into(), read_only_review, None);
    backend.initialized = true;
    backend.backend_version = Some(initialize.user_agent);
    backend.account = Some(AccountId::new("account-test").unwrap());
    (backend, sent)
}
