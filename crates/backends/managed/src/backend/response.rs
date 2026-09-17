//! 일반 커넥터 관찰과 의미 있는 라운드 완료 처리.

mod messages;
mod terminal;
mod tools;
mod usage;

use yo_core::{BackendFailure, BackendFailureKind, ModelConnectorEvent, ModelReplayItem};

use super::{NativeModelBackend, TurnState, failure};

impl NativeModelBackend {
    pub(super) fn apply_response_event(
        &mut self,
        state: &mut TurnState,
        event: ModelConnectorEvent,
    ) -> Result<(), BackendFailure> {
        if state
            .round_replay
            .values()
            .any(|item| matches!(item, ModelReplayItem::ProviderPrivateAssistant { .. }))
            && !matches!(&event, ModelConnectorEvent::Terminal { .. })
        {
            return Err(failure(
                BackendFailureKind::Protocol,
                "semantic model output arrived after provider-private replay was sealed",
            ));
        }
        match event {
            ModelConnectorEvent::ResponseCreated { response_id } => {
                state.response_id = Some(response_id);
            },
            ModelConnectorEvent::TextDelta {
                output_index,
                content_index,
                delta,
                ..
            } => messages::text_delta(self, state, output_index, content_index, delta)?,
            ModelConnectorEvent::RefusalDelta {
                output_index,
                content_index,
                delta,
                ..
            } => messages::refusal_delta(self, state, output_index, content_index, delta)?,
            ModelConnectorEvent::MessageDone {
                output_index,
                item_id: _,
            } => messages::message_done(self, state, output_index)?,
            ModelConnectorEvent::ReasoningDelta {
                output_index,
                part_index,
                channel,
                delta,
                ..
            } => messages::reasoning_delta(self, state, output_index, part_index, channel, delta)?,
            ModelConnectorEvent::FunctionCallStarted {
                output_index,
                item_id,
                call_id,
                name,
            } => tools::function_call_started(self, state, output_index, item_id, call_id, name)?,
            ModelConnectorEvent::FunctionArgumentsDelta { .. } => {},
            ModelConnectorEvent::FunctionCallDone {
                output_index,
                item_id,
                call_id,
                name,
                arguments,
            } => tools::function_call_done(
                self,
                state,
                output_index,
                item_id,
                call_id,
                name,
                arguments,
            )?,
            ModelConnectorEvent::ProviderPrivateAssistant {
                output_index,
                envelope,
                visible_projection,
            } => terminal::provider_private_assistant(
                self,
                state,
                output_index,
                envelope,
                visible_projection,
            )?,
            ModelConnectorEvent::Terminal {
                response_id,
                status,
                usage,
            } => terminal::terminal(self, state, response_id, status, usage)?,
        }
        Ok(())
    }
}
