//! Opt-in diagnostic for the Codex dynamic-tool to Yo hidden-input boundary.

use serde_json::{Value, json};
use yo_backend::transport::JsonMessagePeer;
use yo_core::{BackendFailure, BackendFailureKind};

use super::state::{Backend, InputQuestions};
use crate::protocol;

pub(super) const TOOL_NAME: &str = "yo_secret_entry_probe";

pub(super) fn wire_version_supported(user_agent: &str) -> bool {
    user_agent
        .split_whitespace()
        .next()
        .and_then(|part| part.strip_prefix("yo/"))
        == Some("0.155.1")
}

pub(super) fn tool_spec() -> Value {
    json!({
        "type": "function",
        "name": TOOL_NAME,
        "description": "Diagnostic only: ask the user to enter a SAMPLE secret in Yo's hidden editor. Yo discards the value and returns only a fixed status. Never request a real password, token, or credential with this tool; the model cannot use the entered value.",
        "inputSchema": {
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }
    })
}

pub(super) fn parse_request<P: JsonMessagePeer>(
    backend: &mut Backend<P>,
    params: &Value,
) -> Result<InputQuestions, BackendFailure> {
    if !backend.secret_probe_enabled || backend.read_only_review {
        return Err(BackendFailure::new(
            BackendFailureKind::Unsupported,
            "Codex secret-entry probe is not enabled for this Session",
        ));
    }
    if !wire_version_supported(backend.backend_version.as_deref().unwrap_or_default()) {
        return Err(BackendFailure::new(
            BackendFailureKind::Unsupported,
            "the secret-entry probe requires the reviewed Codex 0.155.1 dynamic-tool wire",
        ));
    }
    if params
        .get("namespace")
        .is_some_and(|value| !value.is_null())
        || protocol::string_at(params, &["tool"])? != TOOL_NAME
    {
        return Err(protocol::protocol_failure("unknown Codex dynamic tool"));
    }
    let call_id = protocol::string_at(params, &["callId"])?;
    let wire_turn = protocol::string_at(params, &["turnId"])?;
    if call_id.is_empty()
        || !backend.items.get(call_id).is_some_and(|item| {
            item.dynamic_tool_call
                .as_ref()
                .is_some_and(|call| call.tool == TOOL_NAME && call.arguments == params["arguments"])
                && backend.wire_turns.get(wire_turn).is_some_and(|turn| {
                    item.activity.turn() == turn.turn && !turn.finished && !turn.interrupted
                })
        })
    {
        return Err(protocol::protocol_failure(
            "Codex dynamic tool call does not match an active item",
        ));
    }
    let args = params
        .get("arguments")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            protocol::protocol_failure("Codex dynamic tool arguments must be an object")
        })?;
    if !args.is_empty() {
        return Err(protocol::protocol_failure(
            "Codex secret-entry probe takes no arguments",
        ));
    }
    let mut parsed = InputQuestions::parse(&json!({"questions": [{
        "id": "sample-secret",
        "header": "Sample input test",
        "question": "Enter a made-up sample value only. Never enter a real password, token, or credential. Yo will discard the sample without sending it to Codex or the model.",
        "isSecret": true,
        "options": []
    }]}))?;
    parsed.probe_only = true;
    // Consume the started call before publishing a prompt. A later failure is
    // fail-closed: Codex cannot replay the same call to open another editor.
    backend
        .items
        .get_mut(call_id)
        .expect("validated active dynamic tool item")
        .dynamic_tool_call
        .take();
    Ok(parsed)
}
