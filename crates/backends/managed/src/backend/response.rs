//! Ordinary connector observations and semantic round completion.

use std::collections::BTreeMap;

use serde_json::json;
use yo_backend::validate_provider_private_replay_sequence;
use yo_core::{
    ActivityKind, ActivityOutcome, ActivityRef, BackendEvent, BackendFailure, BackendFailureKind,
    CacheReadInputTokens, Failure, ModelConnectorEvent, ModelConnectorTerminal, ModelReplayItem,
    ModelReplayRole, ReasoningChannel, ReplayProfile, ToolValidationFailure,
    provider_private_schema,
};

use super::{
    CallActivity, NativeModelBackend, PendingCall, TurnState, failure, map_connector_cleanup,
    tools::{durable_tool_validation_message, tool_validation_failure},
};

impl NativeModelBackend {
    fn assistant_activity(
        &mut self,
        state: &mut TurnState,
        output_index: usize,
    ) -> Result<ActivityRef, BackendFailure> {
        if let Some(activity) = state.assistant_activities.get(&output_index) {
            return Ok(*activity);
        }
        let activity = self.next_activity(state.turn)?;
        state.assistant_activities.insert(output_index, activity);
        self.events.push_back(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::AgentMessage,
        });
        Ok(activity)
    }

    fn apply_visible_delta(
        &mut self,
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
        let activity = self.assistant_activity(state, output_index)?;
        let target = if refusal {
            &mut state.round_refusals
        } else {
            &mut state.round_messages
        };
        target.entry(key).or_default().push_str(&delta);
        self.events.push_back(BackendEvent::ActivityUpdated {
            activity,
            update: yo_core::ActivityUpdate::TextDelta(delta),
        });
        Ok(())
    }

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
            } => {
                self.apply_visible_delta(state, output_index, content_index, delta, false)?;
            },
            ModelConnectorEvent::RefusalDelta {
                output_index,
                content_index,
                delta,
                ..
            } => {
                self.apply_visible_delta(state, output_index, content_index, delta, true)?;
            },
            ModelConnectorEvent::MessageDone {
                output_index,
                item_id: _,
            } => {
                if state.round_replay.contains_key(&output_index)
                    || !state.round_message_items.insert(output_index)
                {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "model output index completed more than one semantic item",
                    ));
                }
                self.assistant_activity(state, output_index)?;
            },
            ModelConnectorEvent::ReasoningDelta {
                output_index,
                part_index,
                channel,
                delta,
                ..
            } => {
                if channel == ReasoningChannel::Summary {
                    let key = (output_index, part_index);
                    let activity = if let Some(activity) = state.reasoning_activities.get(&key) {
                        *activity
                    } else {
                        let activity = self.next_activity(state.turn)?;
                        state.reasoning_activities.insert(key, activity);
                        self.events.push_back(BackendEvent::ActivityStarted {
                            activity,
                            kind: ActivityKind::ModelWork,
                        });
                        activity
                    };
                    self.events.push_back(BackendEvent::ActivityUpdated {
                        activity,
                        update: yo_core::ActivityUpdate::TextDelta(delta),
                    });
                }
            },
            ModelConnectorEvent::FunctionCallStarted {
                output_index,
                item_id,
                call_id,
                name,
            } => {
                if state.call_activities.contains_key(&item_id)
                    || !state.seen_call_ids.insert(call_id.clone())
                {
                    let message = "duplicate function item or call identity";
                    let activity = self.next_activity(state.turn)?;
                    self.queue_activity_text(
                        activity,
                        ActivityKind::ToolCall,
                        format!("{name} {call_id}"),
                        Some(ActivityOutcome::Failed(tool_validation_failure(
                            ToolValidationFailure::DuplicateIdentity,
                            message,
                        ))),
                    );
                    self.fail_turn(state, message.to_owned());
                    return Ok(());
                }
                let activity = self.next_activity(state.turn)?;
                state.call_activities.insert(
                    item_id,
                    CallActivity {
                        activity,
                        output_index,
                        call_id,
                        name: name.clone(),
                    },
                );
                self.queue_activity_text(activity, ActivityKind::ToolCall, name, None);
            },
            ModelConnectorEvent::FunctionArgumentsDelta { .. } => {},
            ModelConnectorEvent::FunctionCallDone {
                output_index,
                item_id,
                call_id,
                name,
                arguments,
            } => {
                let started = state.call_activities.remove(&item_id).ok_or_else(|| {
                    failure(
                        BackendFailureKind::Protocol,
                        "completed function call was not started",
                    )
                })?;
                if started.output_index != output_index
                    || started.call_id != call_id
                    || started.name != name
                {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "completed function call does not match its start identity",
                    ));
                }
                let activity = started.activity;
                match self.registry.validate_call(
                    call_id.clone(),
                    &name,
                    &arguments,
                    self.config.maximum_tool_argument_bytes,
                ) {
                    Ok(call) => {
                        let admitted_arguments = match self
                            .semantic_admission
                            .as_ref()
                            .expect("a non-empty registry requires semantic admission")
                            .admit_arguments(call.definition(), &arguments)
                        {
                            Ok(admitted)
                                if admitted.len() <= self.config.maximum_tool_argument_bytes
                                    && serde_json::from_str::<serde_json::Value>(&admitted)
                                        .is_ok() =>
                            {
                                admitted
                            },
                            Ok(_) => {
                                let message = "semantic admission returned invalid or oversized argument JSON";
                                self.fail_tool_admission(activity, call_id, name, message);
                                self.fail_turn(state, message.to_owned());
                                return Ok(());
                            },
                            Err(_) => {
                                let message = "tool argument semantic admission was rejected";
                                self.fail_tool_admission(activity, call_id, name, message);
                                self.fail_turn(state, message.to_owned());
                                return Ok(());
                            },
                        };
                        self.events.push_back(BackendEvent::ActivityUpdated {
                            activity,
                            update: yo_core::ActivityUpdate::TextSnapshot(
                                json!({
                                    "call_id": call_id,
                                    "name": name,
                                    "arguments": admitted_arguments,
                                })
                                .to_string(),
                            ),
                        });
                        if !self.tool_host.is_available(call.definition().id()) {
                            let message = "tool is unavailable on the selected execution host";
                            self.events.push_back(BackendEvent::ActivityFinished {
                                activity,
                                outcome: ActivityOutcome::Failed(tool_validation_failure(
                                    ToolValidationFailure::Unavailable,
                                    message,
                                )),
                            });
                            self.fail_turn(state, message.to_owned());
                            return Ok(());
                        }
                        self.events.push_back(BackendEvent::ActivityFinished {
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
                        self.events.push_back(BackendEvent::ActivityUpdated {
                            activity,
                            update: yo_core::ActivityUpdate::TextSnapshot(
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
                        self.events.push_back(BackendEvent::ActivityFinished {
                            activity,
                            outcome: ActivityOutcome::Failed(tool_validation_failure(
                                kind, message,
                            )),
                        });
                        self.fail_turn(state, message.to_owned());
                    },
                }
            },
            ModelConnectorEvent::ProviderPrivateAssistant {
                output_index,
                envelope,
                visible_projection,
            } => {
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
                if Self::last_visible_round_output_index(state)
                    .is_some_and(|last| output_index <= last)
                {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "provider-private assistant must follow every visible output in its group",
                    ));
                }
                if provider_private_schema(self.replay_profile) != Some(envelope.schema()) {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "provider-private assistant schema differs from its replay profile",
                    ));
                }
                let current_projection = self.current_visible_round_projection(state)?;
                Self::validate_provider_private_visible_projection(&current_projection)?;
                if current_projection != visible_projection {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "provider-private assistant projection differs from semantic replay",
                    ));
                }
                let item = ModelReplayItem::ProviderPrivateAssistant { envelope };
                self.ensure_replay_capacity_with_round_item(state, Some((output_index, &item)))?;
                state.round_replay.insert(output_index, item);
            },
            ModelConnectorEvent::Terminal {
                response_id,
                status,
                usage,
            } => {
                if !state.call_activities.is_empty() {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "model terminal arrived with an incomplete function call",
                    ));
                }
                if state.response_id.as_deref() != Some(response_id.as_str()) {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "model terminal identity does not match the created response",
                    ));
                }
                if let Some(mut stream) = state.stream.take() {
                    stream.shutdown().map_err(map_connector_cleanup)?;
                }
                *self.shared_stop.response.lock().map_err(|_| {
                    failure(BackendFailureKind::Cleanup, "native stop state is poisoned")
                })? = None;
                self.ensure_replay_capacity_with_round_item(state, None)?;
                let attribution = self.next_activity(state.turn)?;
                let cache_read_input_tokens = match &usage.cache_read_input_tokens {
                    CacheReadInputTokens::Reported {
                        tokens,
                        source_profile,
                    } => json!({
                        "availability": "reported",
                        "tokens": tokens,
                        "source_profile": source_profile.as_str(),
                    }),
                    CacheReadInputTokens::Absent { source_profile } => json!({
                        "availability": "absent",
                        "source_profile": source_profile.as_str(),
                    }),
                    CacheReadInputTokens::Unsupported => json!({
                        "availability": "unsupported",
                    }),
                };
                self.queue_activity_text(
                    attribution,
                    ActivityKind::ModelWork,
                    json!({
                        "schema": "yo.model-usage-receipt/v1",
                        "response_id": response_id,
                        "round": state.round,
                        "provider": self.binding.provider_id().as_str(),
                        "account": self.binding.account_id().as_str(),
                        "model": self.binding.model_id().as_str(),
                        "connector": self.binding.connector_id().as_str(),
                        "api_dialect": self.binding.api_dialect().as_str(),
                        "base_url": self.binding.endpoint().as_str(),
                        "usage": {
                            "input_tokens": usage.input_tokens,
                            "output_tokens": usage.output_tokens,
                            "total_tokens": usage.total_tokens,
                            "reasoning_tokens": usage.reasoning_tokens,
                        },
                        "cache_read_input_tokens": cache_read_input_tokens,
                    })
                    .to_string(),
                    Some(ActivityOutcome::Completed),
                );
                for activity in state
                    .assistant_activities
                    .values()
                    .chain(state.reasoning_activities.values())
                {
                    self.events.push_back(BackendEvent::ActivityFinished {
                        activity: *activity,
                        outcome: terminal_activity_outcome(&status),
                    });
                }
                let mut messages = BTreeMap::<usize, String>::new();
                for ((output_index, _), content) in std::mem::take(&mut state.round_messages) {
                    messages.entry(output_index).or_default().push_str(&content);
                }
                let mut refusals = BTreeMap::<usize, String>::new();
                for ((output_index, _), refusal) in std::mem::take(&mut state.round_refusals) {
                    refusals.entry(output_index).or_default().push_str(&refusal);
                }
                for output_index in std::mem::take(&mut state.round_message_items) {
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
                if matches!(&status, ModelConnectorTerminal::Completed)
                    && self.replay_profile == ReplayProfile::ProviderPrivateLocalPlaintext
                {
                    validate_provider_private_replay_sequence(
                        &state.round_replay.values().cloned().collect::<Vec<_>>(),
                        provider_private_schema(self.replay_profile)
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
                    .extend(std::mem::take(&mut state.round_replay).into_values());
                match status {
                    ModelConnectorTerminal::Completed if state.pending_calls.is_empty() => {
                        if completed_round_has_assistant {
                            self.observe_model_request(
                                state.turn,
                                yo_core::ModelRequestOutcome::Succeeded,
                            );
                            self.complete_turn(state)?;
                        } else {
                            self.observe_model_request(
                                state.turn,
                                yo_core::ModelRequestOutcome::Failed(
                                    yo_core::ModelRequestFailureKind::Protocol,
                                ),
                            );
                            self.fail_turn(
                                state,
                                "completed model response did not contain a final assistant message"
                                    .to_owned(),
                            );
                        }
                    },
                    ModelConnectorTerminal::Completed => {
                        self.observe_model_request(
                            state.turn,
                            yo_core::ModelRequestOutcome::Succeeded,
                        );
                        self.advance_tool_queue(state)?;
                    },
                    ModelConnectorTerminal::Incomplete {
                        reason,
                        request_failure,
                    } => {
                        self.observe_model_request(
                            state.turn,
                            yo_core::ModelRequestOutcome::Failed(request_failure),
                        );
                        self.fail_turn(
                            state,
                            format!(
                                "model response was incomplete: {}",
                                reason.unwrap_or_else(|| "unknown reason".to_owned())
                            ),
                        );
                    },
                    ModelConnectorTerminal::Failed {
                        code,
                        request_failure,
                    } => {
                        self.observe_model_request(
                            state.turn,
                            yo_core::ModelRequestOutcome::Failed(request_failure),
                        );
                        self.fail_turn(
                            state,
                            format!(
                                "model response failed: {}",
                                code.unwrap_or_else(|| "unknown code".to_owned())
                            ),
                        );
                    },
                }
            },
        }
        Ok(())
    }
}

fn terminal_activity_outcome(status: &ModelConnectorTerminal) -> ActivityOutcome {
    match status {
        ModelConnectorTerminal::Completed => ActivityOutcome::Completed,
        ModelConnectorTerminal::Incomplete { .. } | ModelConnectorTerminal::Failed { .. } => {
            ActivityOutcome::Failed(Failure::new("model response did not complete"))
        },
    }
}
