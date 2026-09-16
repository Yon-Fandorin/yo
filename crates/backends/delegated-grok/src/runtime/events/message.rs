use serde_json::Value;
use yo_core::{
    ActivityKind, ActivityOutcome, ActivityPlan, ActivityReasoning, ActivityUpdate, BackendEvent,
    BackendFailure, MessageContent, PlanStep, PlanStepStatus, ToolOutput,
};

use super::super::state::{
    Backend, MessageBinding, MessageChannel, MessageKey, optional_identifier,
};
use crate::{protocol, transport::JsonPeer};

impl<P: JsonPeer> Backend<P> {
    pub(super) fn plan_update(
        &mut self,
        update: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        let turn = self.active_turn()?;
        let entries = update
            .get("entries")
            .and_then(Value::as_array)
            .ok_or_else(|| protocol::protocol_failure("Grok ACP plan has no entries array"))?;
        let mut steps = Vec::new();
        let mut bytes = 0_usize;
        for entry in entries {
            let content = protocol::string_at(entry, &["content"])?;
            let priority = protocol::string_at(entry, &["priority"])?;
            if !matches!(priority, "high" | "medium" | "low") {
                return Err(protocol::protocol_failure(
                    "Grok ACP plan priority is unsupported",
                ));
            }
            let status = match protocol::string_at(entry, &["status"])? {
                "pending" => PlanStepStatus::Pending,
                "in_progress" => PlanStepStatus::InProgress,
                "completed" => PlanStepStatus::Completed,
                _ => {
                    return Err(protocol::protocol_failure(
                        "Grok ACP plan status is unsupported",
                    ));
                },
            };
            bytes = bytes
                .checked_add(content.len())
                .and_then(|size| size.checked_add(priority.len() + 3))
                .filter(|size| *size <= ToolOutput::MAX_SNAPSHOT_BYTES)
                .ok_or_else(|| {
                    protocol::protocol_failure("Grok ACP plan exceeds presentation limit")
                })?;
            steps.push(PlanStep {
                text: format!("[{priority}] {content}"),
                status,
            });
        }
        let text = ActivityPlan {
            explanation: None,
            steps,
        }
        .to_snapshot()
        .ok_or_else(|| protocol::protocol_failure("Grok ACP plan exceeds presentation limit"))?;
        let key = MessageKey {
            channel: MessageChannel::Plan,
            message_id: None,
        };
        if let Some(binding) = self.messages.get(&key) {
            return Ok(Some(BackendEvent::ActivityUpdated {
                activity: binding.activity,
                update: ActivityUpdate::TextSnapshot(text),
            }));
        }
        self.ensure_activity_capacity()?;
        let activity = self.next_activity(turn)?;
        self.messages.insert(
            key,
            MessageBinding {
                activity,
                reasoning: None,
            },
        );
        self.pending_events
            .push_back(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(text),
            });
        Ok(Some(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ModelWork,
        }))
    }

    pub(super) fn message_chunk(
        &mut self,
        update: &Value,
        channel: MessageChannel,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        let turn = self.active_turn()?;
        let content = update
            .get("content")
            .ok_or_else(|| protocol::protocol_failure("Grok ACP message chunk has no content"))?;
        let key = MessageKey {
            channel,
            message_id: optional_identifier(update, "messageId")?,
        };
        if content.get("type").and_then(Value::as_str) != Some("text") {
            self.ensure_activity_capacity()?;
            let activity = self.next_activity(turn)?;
            // 콘텐츠 블록은 같은 메시지의 텍스트 스트림을 나누어 이미지·리소스 뒤의 텍스트 순서를
            // 보존합니다.
            if let Some(binding) = self.messages.remove(&key) {
                self.pending_events
                    .push_back(BackendEvent::ActivityFinished {
                        activity: binding.activity,
                        outcome: ActivityOutcome::Completed,
                    });
            }
            let source = if channel == MessageChannel::Agent {
                MessageContent {
                    block: content.clone(),
                }
                .to_snapshot()
            } else {
                Some(reasoning_snapshot(content.clone())?)
            }
            .unwrap_or_else(|| format!("{content:#}"));
            self.pending_events.extend([
                BackendEvent::ActivityStarted {
                    activity,
                    kind: match channel {
                        MessageChannel::Agent => ActivityKind::AgentMessage,
                        MessageChannel::Thought | MessageChannel::Plan => ActivityKind::ModelWork,
                    },
                },
                BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(source),
                },
                BackendEvent::ActivityFinished {
                    activity,
                    outcome: ActivityOutcome::Completed,
                },
            ]);
            return Ok(self.pending_events.pop_front());
        }
        let text = protocol::string_at(content, &["text"])?;
        if channel == MessageChannel::Thought {
            if let Some(binding) = self.messages.get_mut(&key) {
                let accumulated = binding
                    .reasoning
                    .as_mut()
                    .expect("thought stream owns text");
                if accumulated.len().saturating_add(text.len()) > ToolOutput::MAX_SNAPSHOT_BYTES {
                    return Err(protocol::protocol_failure(
                        "Grok reasoning exceeds presentation limit",
                    ));
                }
                accumulated.push_str(text);
                return Ok(Some(BackendEvent::ActivityUpdated {
                    activity: binding.activity,
                    update: ActivityUpdate::TextSnapshot(reasoning_snapshot(Value::String(
                        accumulated.clone(),
                    ))?),
                }));
            }
            let snapshot = reasoning_snapshot(Value::String(text.to_owned()))?;
            self.ensure_activity_capacity()?;
            let activity = self.next_activity(turn)?;
            self.messages.insert(
                key,
                MessageBinding {
                    activity,
                    reasoning: Some(text.to_owned()),
                },
            );
            self.pending_events
                .push_back(BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(snapshot),
                });
            return Ok(Some(BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::ModelWork,
            }));
        }
        let existing = self.messages.get(&key).map(|binding| binding.activity);
        let activity = match existing {
            Some(activity) => activity,
            None => {
                self.ensure_activity_capacity()?;
                let activity = self.next_activity(turn)?;
                self.messages.insert(
                    key,
                    MessageBinding {
                        activity,
                        reasoning: None,
                    },
                );
                self.pending_events
                    .push_back(BackendEvent::ActivityUpdated {
                        activity,
                        update: ActivityUpdate::TextDelta(text.to_owned()),
                    });
                return Ok(Some(BackendEvent::ActivityStarted {
                    activity,
                    kind: match channel {
                        MessageChannel::Agent => ActivityKind::AgentMessage,
                        MessageChannel::Thought | MessageChannel::Plan => ActivityKind::ModelWork,
                    },
                }));
            },
        };
        Ok(Some(BackendEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextDelta(text.to_owned()),
        }))
    }
    pub(super) fn finish_anonymous_messages(&mut self) {
        let keys = self
            .messages
            .keys()
            .filter(|key| key.message_id.is_none() && key.channel != MessageChannel::Plan)
            .cloned()
            .collect::<Vec<_>>();
        let mut activities = keys
            .into_iter()
            .filter_map(|key| self.messages.remove(&key))
            .map(|binding| binding.activity)
            .collect::<Vec<_>>();
        activities.sort_unstable();
        self.pending_events
            .extend(
                activities
                    .into_iter()
                    .map(|activity| BackendEvent::ActivityFinished {
                        activity,
                        outcome: ActivityOutcome::Completed,
                    }),
            );
    }
}

fn reasoning_snapshot(content: Value) -> Result<String, BackendFailure> {
    ActivityReasoning { content }
        .to_snapshot()
        .ok_or_else(|| protocol::protocol_failure("Grok reasoning exceeds presentation limit"))
}
