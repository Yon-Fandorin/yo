mod notifications;
mod requests;
mod snapshots;

use serde_json::Value;
use yo_backend::transport::JsonMessagePeer;
use yo_core::{ApprovalChoice, BackendFailure, BackendPoll};

use super::state::Backend;

pub(super) fn approval_choice(value: &Value, command: bool) -> Option<ApprovalChoice> {
    requests::approval_choice(value, command)
}

pub(super) fn wire_key(value: &Value) -> Result<String, BackendFailure> {
    requests::wire_key(value)
}

pub(super) fn poll_client_message<P: JsonMessagePeer>(
    backend: &mut Backend<P>,
) -> Result<BackendPoll, BackendFailure> {
    backend.poll_client_message()
}
