//! 함수 호출 식별자, 승인, 도구 스냅샷.

use serde_json::json;
use yo_core::{
    ActivityKind, ActivityOutcome, ActivityUpdate, BackendEvent, BackendFailure,
    BackendFailureKind, ModelReplayItem, ToolOutput, ToolValidationFailure,
};

use super::super::{
    CallActivity, NativeModelBackend, PendingCall, TurnState, failure,
    tools::{durable_tool_validation_message, tool_validation_failure},
};

pub(super) fn function_call_started(
    backend: &mut NativeModelBackend,
    state: &mut TurnState,
    output_index: usize,
    item_id: String,
    call_id: String,
    name: String,
) -> Result<(), BackendFailure> {
    if state.call_activities.contains_key(&item_id) || !state.seen_call_ids.insert(call_id.clone())
    {
        let message = "duplicate function item or call identity";
        let activity = backend.next_activity(state.turn)?;
        backend.queue_activity_text(
            activity,
            ActivityKind::ToolCall,
            format!("{name} {call_id}"),
            Some(ActivityOutcome::Failed(tool_validation_failure(
                ToolValidationFailure::DuplicateIdentity,
                message,
            ))),
        );
        backend.fail_turn(state, message.to_owned());
        return Ok(());
    }
    let activity = backend.next_activity(state.turn)?;
    state.call_activities.insert(
        item_id,
        CallActivity {
            activity,
            output_index,
            call_id,
            name: name.clone(),
        },
    );
    backend.queue_activity_text(activity, ActivityKind::ToolCall, name, None);
    Ok(())
}

pub(super) fn function_call_done(
    backend: &mut NativeModelBackend,
    state: &mut TurnState,
    output_index: usize,
    item_id: String,
    call_id: String,
    name: String,
    arguments: String,
) -> Result<(), BackendFailure> {
    let started = state.call_activities.remove(&item_id).ok_or_else(|| {
        failure(
            BackendFailureKind::Protocol,
            "completed function call was not started",
        )
    })?;
    if started.output_index != output_index || started.call_id != call_id || started.name != name {
        return Err(failure(
            BackendFailureKind::Protocol,
            "completed function call does not match its start identity",
        ));
    }
    let activity = started.activity;
    match backend.registry.validate_call(
        call_id.clone(),
        &name,
        &arguments,
        backend.config.maximum_tool_argument_bytes,
    ) {
        Ok(call) => {
            let admitted_arguments = match backend
                .semantic_admission
                .as_ref()
                .expect("a non-empty registry requires semantic admission")
                .admit_arguments(call.definition(), &arguments)
            {
                Ok(admitted)
                    if admitted.len() <= backend.config.maximum_tool_argument_bytes
                        && serde_json::from_str::<serde_json::Value>(&admitted).is_ok() =>
                {
                    admitted
                },
                Ok(_) => {
                    let message = "semantic admission returned invalid or oversized argument JSON";
                    backend.fail_tool_admission(activity, call_id, name, message);
                    backend.fail_turn(state, message.to_owned());
                    return Ok(());
                },
                Err(_) => {
                    let message = "tool argument semantic admission was rejected";
                    backend.fail_tool_admission(activity, call_id, name, message);
                    backend.fail_turn(state, message.to_owned());
                    return Ok(());
                },
            };
            backend.events.push_back(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(
                    ToolOutput {
                        tool: name.clone(),
                        server: None,
                        arguments: admitted_arguments.parse().ok(),
                        result: None,
                        content_items: None,
                        error: None,
                        plain_text: format!("{name} · {call_id}\nArguments:\n{admitted_arguments}"),
                    }
                    .to_snapshot()
                    .unwrap_or_else(|| {
                        json!({
                            "call_id": call_id,
                            "name": name,
                            "arguments": admitted_arguments,
                        })
                        .to_string()
                    }),
                ),
            });
            if !backend.tool_host.is_available(call.definition().id()) {
                let message = "tool is unavailable on the selected execution host";
                backend.events.push_back(BackendEvent::ActivityFinished {
                    activity,
                    outcome: ActivityOutcome::Failed(tool_validation_failure(
                        ToolValidationFailure::Unavailable,
                        message,
                    )),
                });
                backend.fail_turn(state, message.to_owned());
                return Ok(());
            }
            backend.events.push_back(BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            });
            if state
                .round_replay
                .insert(
                    output_index,
                    ModelReplayItem::FunctionCall {
                        call_id,
                        name,
                        arguments: admitted_arguments,
                    },
                )
                .is_some()
            {
                return Err(failure(
                    BackendFailureKind::Protocol,
                    "model output index was completed more than once",
                ));
            }
            if state
                .pending_calls
                .insert(
                    output_index,
                    PendingCall {
                        call,
                        approval: None,
                    },
                )
                .is_some()
            {
                return Err(failure(
                    BackendFailureKind::Protocol,
                    "model output index declared more than one function call",
                ));
            }
        },
        Err(error) => {
            let kind = error.kind();
            let message = durable_tool_validation_message(kind);
            backend.events.push_back(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(
                    json!({
                        "call_id": call_id,
                        "name": name,
                        "validation_failure": {
                            "code": kind.code(),
                            "message": message,
                        },
                    })
                    .to_string(),
                ),
            });
            backend.events.push_back(BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Failed(tool_validation_failure(kind, message)),
            });
            backend.fail_turn(state, message.to_owned());
        },
    }
    Ok(())
}
