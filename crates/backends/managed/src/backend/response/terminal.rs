//! 터미널 상태, 재생 완료, 실패 결과.

use std::{collections::BTreeMap, mem};

use yo_backend::validate_provider_private_replay_sequence;
use yo_core::{
    ActivityOutcome, BackendEvent, BackendFailure, BackendFailureKind, Failure,
    ModelConnectorTerminal, ModelReplayItem, ModelReplayRole, ModelRequestFailureKind,
    ModelRequestOutcome, ProviderPrivateReplayEnvelope, ReplayProfile,
};

use super::{
    super::{NativeModelBackend, TurnState, failure, map_connector_cleanup},
    usage,
};

pub(super) fn provider_private_assistant(
    backend: &mut NativeModelBackend,
    state: &mut TurnState,
    output_index: usize,
    envelope: ProviderPrivateReplayEnvelope,
    visible_projection: Vec<ModelReplayItem>,
) -> Result<(), BackendFailure> {
    if state.round_replay.contains_key(&output_index) {
        return Err(failure(
            BackendFailureKind::Protocol,
            "provider-private assistant reused a model output index",
        ));
    }
    if !state.call_activities.is_empty() {
        return Err(failure(
            BackendFailureKind::Protocol,
            "provider-private assistant preceded completion of its function calls",
        ));
    }
    if NativeModelBackend::last_visible_round_output_index(state)
        .is_some_and(|last| output_index <= last)
    {
        return Err(failure(
            BackendFailureKind::Protocol,
            "provider-private assistant must follow every visible output in its group",
        ));
    }
    if yo_core::provider_private_schema(backend.replay_profile) != Some(envelope.schema()) {
        return Err(failure(
            BackendFailureKind::Protocol,
            "provider-private assistant schema differs from its replay profile",
        ));
    }
    let current_projection = backend.current_visible_round_projection(state)?;
    NativeModelBackend::validate_provider_private_visible_projection(&current_projection)?;
    if current_projection != visible_projection {
        return Err(failure(
            BackendFailureKind::Protocol,
            "provider-private assistant projection differs from semantic replay",
        ));
    }
    let item = ModelReplayItem::ProviderPrivateAssistant { envelope };
    backend.ensure_replay_capacity_with_round_item(state, Some((output_index, &item)))?;
    state.round_replay.insert(output_index, item);
    Ok(())
}

pub(super) fn terminal(
    backend: &mut NativeModelBackend,
    state: &mut TurnState,
    response_id: String,
    status: ModelConnectorTerminal,
    usage_values: yo_core::ModelConnectorUsage,
) -> Result<(), BackendFailure> {
    if state.response_id.as_deref() != Some(response_id.as_str()) {
        return Err(failure(
            BackendFailureKind::Protocol,
            "model terminal identity does not match the created response",
        ));
    }
    if matches!(status, ModelConnectorTerminal::Completed) && !state.call_activities.is_empty() {
        return Err(failure(
            BackendFailureKind::Protocol,
            "model terminal arrived with an incomplete function call",
        ));
    }
    if let Some(mut stream) = state.stream.take() {
        stream.shutdown().map_err(map_connector_cleanup)?;
    }
    *backend
        .shared_stop
        .response
        .lock()
        .map_err(|_| failure(BackendFailureKind::Cleanup, "native stop state is poisoned"))? = None;
    backend.ensure_replay_capacity_with_round_item(state, None)?;
    usage::record(backend, state, &response_id, &usage_values)?;

    let terminal_failure = match &status {
        ModelConnectorTerminal::Completed => None,
        ModelConnectorTerminal::Incomplete {
            reason,
            request_failure,
        } => Some((
            *request_failure,
            format!(
                "model response was incomplete: {}",
                reason.as_deref().unwrap_or("unknown reason")
            ),
        )),
        ModelConnectorTerminal::Failed {
            code,
            request_failure,
        } => Some((
            *request_failure,
            format!(
                "model response failed: {}",
                code.as_deref().unwrap_or("unknown code")
            ),
        )),
    };
    if let Some((kind, message)) = terminal_failure {
        backend.observe_model_request(state.turn, ModelRequestOutcome::Failed(kind));
        if kind == ModelRequestFailureKind::ResponseLimit {
            let notice = backend.next_activity(state.turn)?;
            backend.queue_activity_text(
                notice,
                yo_core::ActivityKind::ModelWork,
                yo_core::ActivityNotice {
                    title: "Response limit reached".to_owned(),
                    message: "The response stopped before completion.\nPartial answer text is retained. Unfinished tool calls were not executed.".to_owned(),
                    level: yo_core::NoticeLevel::Warning,
                }
                .to_snapshot()
                .expect("bounded static limit notice"),
                Some(ActivityOutcome::Completed),
            );
        }
        // 실패한 라운드는 표시용 증거이며 재생이나 실행 가능한 호출이 아닙니다.
        backend.fail_turn(state, message);
        return Ok(());
    }
    for activity in state
        .assistant_activities
        .values()
        .chain(state.reasoning_activities.values())
    {
        backend.events.push_back(BackendEvent::ActivityFinished {
            activity: *activity,
            outcome: terminal_activity_outcome(&status),
        });
    }
    finalize_messages(state)?;
    if matches!(&status, ModelConnectorTerminal::Completed)
        && backend.replay_profile == ReplayProfile::ProviderPrivateLocalPlaintext
    {
        validate_provider_private_replay_sequence(
            &state.round_replay.values().cloned().collect::<Vec<_>>(),
            yo_core::provider_private_schema(backend.replay_profile)
                .expect("the provider-private profile has an exact schema"),
        )
        .map_err(|detail| failure(BackendFailureKind::Protocol, detail))?;
    }
    let completed_round_has_assistant = state.round_replay.values().any(|item| {
        matches!(
            item,
            ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                ..
            }
        )
    });
    state
        .delta
        .extend(mem::take(&mut state.round_replay).into_values());
    match status {
        ModelConnectorTerminal::Completed if state.pending_calls.is_empty() => {
            if completed_round_has_assistant {
                backend.observe_model_request(state.turn, ModelRequestOutcome::Succeeded);
                backend.complete_turn(state)?;
            } else {
                backend.observe_model_request(
                    state.turn,
                    ModelRequestOutcome::Failed(ModelRequestFailureKind::Protocol),
                );
                backend.fail_turn(
                    state,
                    "completed model response did not contain a final assistant message".to_owned(),
                );
            }
        },
        ModelConnectorTerminal::Completed => {
            backend.observe_model_request(state.turn, ModelRequestOutcome::Succeeded);
            backend.advance_tool_queue(state)?;
        },
        ModelConnectorTerminal::Incomplete { .. } | ModelConnectorTerminal::Failed { .. } => {
            unreachable!("failed terminals leave before replay preparation")
        },
    }
    Ok(())
}

fn finalize_messages(state: &mut TurnState) -> Result<(), BackendFailure> {
    let mut messages = BTreeMap::<usize, String>::new();
    for ((output_index, _), content) in mem::take(&mut state.round_messages) {
        messages.entry(output_index).or_default().push_str(&content);
    }
    let mut refusals = BTreeMap::<usize, String>::new();
    for ((output_index, _), refusal) in mem::take(&mut state.round_refusals) {
        refusals.entry(output_index).or_default().push_str(&refusal);
    }
    for output_index in mem::take(&mut state.round_message_items) {
        let content = messages.remove(&output_index).unwrap_or_default();
        let refusal = refusals.remove(&output_index);
        if state
            .round_replay
            .insert(
                output_index,
                ModelReplayItem::Message {
                    role: ModelReplayRole::Assistant,
                    content,
                    refusal,
                },
            )
            .is_some()
        {
            return Err(failure(
                BackendFailureKind::Protocol,
                "model output index changed semantic item kind",
            ));
        }
    }
    if !messages.is_empty() || !refusals.is_empty() {
        return Err(failure(
            BackendFailureKind::Protocol,
            "model message text completed without its message output item",
        ));
    }
    Ok(())
}

fn terminal_activity_outcome(status: &ModelConnectorTerminal) -> ActivityOutcome {
    match status {
        ModelConnectorTerminal::Completed => ActivityOutcome::Completed,
        ModelConnectorTerminal::Incomplete { .. } | ModelConnectorTerminal::Failed { .. } => {
            ActivityOutcome::Failed(Failure::new("model response did not complete"))
        },
    }
}
