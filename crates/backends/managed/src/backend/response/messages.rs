//! 어시스턴트, 거부, 추론 메시지 관찰.

use yo_core::{
    ActivityKind, ActivityRef, ActivityUpdate, BackendEvent, BackendFailure, BackendFailureKind,
    ReasoningChannel,
};

use super::super::{NativeModelBackend, TurnState, failure};

pub(super) fn text_delta(
    backend: &mut NativeModelBackend,
    state: &mut TurnState,
    output_index: usize,
    content_index: usize,
    delta: String,
) -> Result<(), BackendFailure> {
    backend.apply_visible_delta(state, output_index, content_index, delta, false)
}

pub(super) fn refusal_delta(
    backend: &mut NativeModelBackend,
    state: &mut TurnState,
    output_index: usize,
    content_index: usize,
    delta: String,
) -> Result<(), BackendFailure> {
    backend.apply_visible_delta(state, output_index, content_index, delta, true)
}

pub(super) fn message_done(
    backend: &mut NativeModelBackend,
    state: &mut TurnState,
    output_index: usize,
) -> Result<(), BackendFailure> {
    if state.round_replay.contains_key(&output_index)
        || !state.round_message_items.insert(output_index)
    {
        return Err(failure(
            BackendFailureKind::Protocol,
            "model output index completed more than one semantic item",
        ));
    }
    assistant_activity(backend, state, output_index)?;
    Ok(())
}

pub(super) fn reasoning_delta(
    backend: &mut NativeModelBackend,
    state: &mut TurnState,
    output_index: usize,
    part_index: usize,
    channel: ReasoningChannel,
    delta: String,
) -> Result<(), BackendFailure> {
    if channel == ReasoningChannel::Summary {
        let key = (output_index, part_index);
        let activity = if let Some(activity) = state.reasoning_activities.get(&key) {
            *activity
        } else {
            let activity = backend.next_activity(state.turn)?;
            state.reasoning_activities.insert(key, activity);
            backend.events.push_back(BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::ModelWork,
            });
            activity
        };
        backend.events.push_back(BackendEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextDelta(delta),
        });
    }
    Ok(())
}

fn assistant_activity(
    backend: &mut NativeModelBackend,
    state: &mut TurnState,
    output_index: usize,
) -> Result<ActivityRef, BackendFailure> {
    if let Some(activity) = state.assistant_activities.get(&output_index) {
        return Ok(*activity);
    }
    let activity = backend.next_activity(state.turn)?;
    state.assistant_activities.insert(output_index, activity);
    backend.events.push_back(BackendEvent::ActivityStarted {
        activity,
        kind: ActivityKind::AgentMessage,
    });
    Ok(activity)
}

fn apply_visible_delta(
    backend: &mut NativeModelBackend,
    state: &mut TurnState,
    output_index: usize,
    content_index: usize,
    delta: String,
    refusal: bool,
) -> Result<(), BackendFailure> {
    let key = (output_index, content_index);
    if state.round_replay.contains_key(&output_index) {
        return Err(failure(
            BackendFailureKind::Protocol,
            "model output index changed semantic item kind",
        ));
    }
    let activity = assistant_activity(backend, state, output_index)?;
    let target = if refusal {
        &mut state.round_refusals
    } else {
        &mut state.round_messages
    };
    target.entry(key).or_default().push_str(&delta);
    backend.events.push_back(BackendEvent::ActivityUpdated {
        activity,
        update: ActivityUpdate::TextDelta(delta),
    });
    Ok(())
}

impl NativeModelBackend {
    fn apply_visible_delta(
        &mut self,
        state: &mut TurnState,
        output_index: usize,
        content_index: usize,
        delta: String,
        refusal: bool,
    ) -> Result<(), BackendFailure> {
        apply_visible_delta(self, state, output_index, content_index, delta, refusal)
    }
}
