//! Semantic replay capacity and provider-private visible projection checks.

use std::collections::{BTreeMap, BTreeSet};

use yo_core::{
    BackendFailure, BackendFailureKind, ModelConnectorInputItem, ModelConnectorInputRole,
    ModelReplayDelta, ModelReplayItem, ModelReplayRole,
};

use super::{NativeModelBackend, TurnState, failure};

impl NativeModelBackend {
    pub(super) fn current_visible_round_projection(
        &self,
        state: &TurnState,
    ) -> Result<Vec<ModelReplayItem>, BackendFailure> {
        let mut messages = BTreeMap::<usize, String>::new();
        for ((output_index, _), content) in &state.round_messages {
            messages.entry(*output_index).or_default().push_str(content);
        }
        let mut refusals = BTreeMap::<usize, String>::new();
        for ((output_index, _), refusal) in &state.round_refusals {
            refusals.entry(*output_index).or_default().push_str(refusal);
        }
        if messages
            .keys()
            .chain(refusals.keys())
            .any(|output_index| !state.round_message_items.contains(output_index))
        {
            return Err(failure(
                BackendFailureKind::Protocol,
                "provider-private assistant preceded completion of its visible message",
            ));
        }
        let message_outputs = state
            .round_message_items
            .iter()
            .copied()
            .chain(messages.keys().copied())
            .chain(refusals.keys().copied())
            .collect::<BTreeSet<_>>();
        if message_outputs
            .iter()
            .any(|output_index| state.round_replay.contains_key(output_index))
        {
            return Err(failure(
                BackendFailureKind::Protocol,
                "model output index changed semantic item kind",
            ));
        }
        let output_indices = state
            .round_replay
            .keys()
            .copied()
            .chain(message_outputs.iter().copied())
            .collect::<BTreeSet<_>>();
        output_indices
            .into_iter()
            .map(|output_index| {
                if let Some(item) = state.round_replay.get(&output_index) {
                    if matches!(item, ModelReplayItem::ProviderPrivateAssistant { .. }) {
                        return Err(failure(
                            BackendFailureKind::Protocol,
                            "provider-private replay appeared inside its visible projection",
                        ));
                    }
                    return Ok(item.clone());
                }
                Ok(ModelReplayItem::Message {
                    role: ModelReplayRole::Assistant,
                    content: messages.remove(&output_index).unwrap_or_default(),
                    refusal: refusals.remove(&output_index),
                })
            })
            .collect()
    }

    pub(super) fn validate_provider_private_visible_projection(
        projection: &[ModelReplayItem],
    ) -> Result<(), BackendFailure> {
        let Some((first, rest)) = projection.split_first() else {
            return Err(failure(
                BackendFailureKind::Protocol,
                "provider-private assistant has no completed visible assistant group",
            ));
        };
        if !matches!(
            first,
            ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                refusal: None,
                ..
            }
        ) || rest
            .iter()
            .any(|item| !matches!(item, ModelReplayItem::FunctionCall { .. }))
        {
            return Err(failure(
                BackendFailureKind::Protocol,
                "provider-private assistant projection must be one assistant message followed by its calls",
            ));
        }
        Ok(())
    }

    pub(super) fn last_visible_round_output_index(state: &TurnState) -> Option<usize> {
        state
            .round_replay
            .keys()
            .copied()
            .chain(state.round_message_items.iter().copied())
            .chain(
                state
                    .round_messages
                    .keys()
                    .map(|(output_index, _)| *output_index),
            )
            .chain(
                state
                    .round_refusals
                    .keys()
                    .map(|(output_index, _)| *output_index),
            )
            .max()
    }

    fn prospective_replay_delta_encoded_len(
        &self,
        state: &TurnState,
        extra: Option<(usize, &ModelReplayItem)>,
    ) -> Result<usize, BackendFailure> {
        if extra
            .as_ref()
            .is_some_and(|(output_index, _)| state.round_replay.contains_key(output_index))
        {
            return Err(failure(
                BackendFailureKind::Protocol,
                "model output index was completed more than once",
            ));
        }

        let mut messages = BTreeMap::<usize, String>::new();
        for ((output_index, _), content) in &state.round_messages {
            messages.entry(*output_index).or_default().push_str(content);
        }
        let mut refusals = BTreeMap::<usize, String>::new();
        for ((output_index, _), refusal) in &state.round_refusals {
            refusals.entry(*output_index).or_default().push_str(refusal);
        }
        let message_outputs = state
            .round_message_items
            .iter()
            .copied()
            .chain(messages.keys().copied())
            .chain(refusals.keys().copied())
            .collect::<BTreeSet<_>>();
        let mut completed_messages = BTreeMap::new();
        for output_index in message_outputs {
            let message = ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                content: messages.remove(&output_index).unwrap_or_default(),
                refusal: refusals.remove(&output_index),
            };
            if state.round_replay.contains_key(&output_index)
                || extra
                    .as_ref()
                    .is_some_and(|(extra_index, _)| *extra_index == output_index)
            {
                return Err(failure(
                    BackendFailureKind::Protocol,
                    "model output index changed semantic item kind",
                ));
            }
            completed_messages.insert(output_index, message);
        }

        let output_indices = state
            .round_replay
            .keys()
            .copied()
            .chain(extra.as_ref().map(|(output_index, _)| *output_index))
            .chain(completed_messages.keys().copied())
            .collect::<BTreeSet<_>>();
        let round_items = output_indices.into_iter().map(|output_index| {
            state
                .round_replay
                .get(&output_index)
                .or_else(|| {
                    extra.as_ref().and_then(|(extra_index, item)| {
                        (*extra_index == output_index).then_some(*item)
                    })
                })
                .or_else(|| completed_messages.get(&output_index))
                .expect("every prospective output index has one replay item")
        });
        ModelReplayDelta::prospective_encoded_len(
            self.replay.contract().is_none().then_some(&self.contract),
            state.delta.iter().chain(round_items),
        )
        .ok_or_else(|| {
            failure(
                BackendFailureKind::ContextExhausted,
                "model replay item capacity exceeded before durable retention",
            )
        })
    }

    pub(super) fn ensure_replay_capacity_with_round_item(
        &self,
        state: &TurnState,
        extra: Option<(usize, &ModelReplayItem)>,
    ) -> Result<(), BackendFailure> {
        if self.prospective_replay_delta_encoded_len(state, extra)?
            <= ModelReplayDelta::MAX_ENCODED_BYTES
        {
            Ok(())
        } else {
            Err(failure(
                BackendFailureKind::ContextExhausted,
                "model replay delta capacity exceeded before durable retention",
            ))
        }
    }

    pub(super) fn ensure_accumulated_replay_capacity(
        &self,
        state: &TurnState,
        extra: Option<&ModelReplayItem>,
    ) -> Result<(), BackendFailure> {
        let encoded_bytes = ModelReplayDelta::prospective_encoded_len(
            self.replay.contract().is_none().then_some(&self.contract),
            state.delta.iter().chain(extra),
        )
        .ok_or_else(|| {
            failure(
                BackendFailureKind::ContextExhausted,
                "model replay item capacity exceeded before durable retention",
            )
        })?;
        if encoded_bytes <= ModelReplayDelta::MAX_ENCODED_BYTES {
            Ok(())
        } else {
            Err(failure(
                BackendFailureKind::ContextExhausted,
                "model replay delta capacity exceeded before durable retention",
            ))
        }
    }

    pub(super) fn ensure_pending_replay_capacity(
        &self,
        state: &TurnState,
    ) -> Result<(), BackendFailure> {
        if state.delta.is_empty() {
            return Ok(());
        }
        let delta = ModelReplayDelta::new(
            self.replay
                .contract()
                .is_none()
                .then(|| self.contract.clone()),
            state.delta.clone(),
        );
        let mut replay = self.replay.clone();
        replay.apply(&delta).map_err(|message| {
            let kind = if is_replay_capacity_error(message) {
                BackendFailureKind::ContextExhausted
            } else {
                BackendFailureKind::Turn
            };
            failure(kind, message)
        })
    }
}

pub(super) fn is_replay_capacity_error(message: &str) -> bool {
    matches!(
        message,
        "model replay delta is invalid or exceeds its bounds"
            | "model replay item limit exceeded"
            | "model replay prefix byte limit exceeded"
    )
}

pub(super) fn replay_input(item: &ModelReplayItem) -> ModelConnectorInputItem {
    match item {
        ModelReplayItem::Message {
            role,
            content,
            refusal,
        } => ModelConnectorInputItem::Message {
            role: match role {
                ModelReplayRole::System => ModelConnectorInputRole::System,
                ModelReplayRole::Developer => ModelConnectorInputRole::Developer,
                ModelReplayRole::User => ModelConnectorInputRole::User,
                ModelReplayRole::Assistant => ModelConnectorInputRole::Assistant,
            },
            content: content.clone(),
            refusal: refusal.clone(),
        },
        ModelReplayItem::FunctionCall {
            call_id,
            name,
            arguments,
        } => ModelConnectorInputItem::FunctionCall {
            call_id: call_id.clone(),
            name: name.clone(),
            arguments: arguments.clone(),
        },
        ModelReplayItem::FunctionCallOutput { call_id, output } => {
            ModelConnectorInputItem::FunctionCallOutput {
                call_id: call_id.clone(),
                output: output.clone(),
            }
        },
        ModelReplayItem::ProviderPrivateAssistant { envelope } => {
            ModelConnectorInputItem::ProviderPrivateAssistant {
                envelope: envelope.clone(),
            }
        },
    }
}

#[cfg(test)]
mod tests;
