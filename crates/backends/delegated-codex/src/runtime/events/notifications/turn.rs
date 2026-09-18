use std::iter;

use serde_json::Value;
use yo_backend::transport::JsonMessagePeer;
use yo_core::{
    ActivityKind, ActivityNotice, ActivityOutcome, ActivityUpdate, BackendEvent, BackendFailure,
    BackendFailureKind, BackendOutcomeEvidence, Failure, NoticeLevel, ToolOutput, TurnOutcome,
};

use super::super::{
    super::state::{Backend, RequestKind},
    requests::wire_key,
};
use crate::protocol;

impl<P: JsonMessagePeer> Backend<P> {
    pub(super) fn turn_completed(
        &mut self,
        params: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        self.validate_thread(params)?;
        let wire_turn = protocol::string_at(params, &["turn", "id"])?;
        let turn = self
            .wire_turns
            .get(wire_turn)
            .map(|binding| binding.turn)
            .ok_or_else(|| {
                protocol::protocol_failure(format!("unknown completed Codex Turn `{wire_turn}`"))
            })?;
        if self
            .wire_turns
            .get(wire_turn)
            .is_some_and(|binding| binding.finished)
        {
            return Ok(None);
        }
        let duration_ms = match params.pointer("/turn/durationMs") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_i64()
                    .filter(|duration| *duration >= 0)
                    .ok_or_else(|| {
                        protocol::protocol_failure(
                            "Codex turn durationMs must be a non-negative int64",
                        )
                    })? as u64,
            ),
        };
        let status = protocol::string_at(params, &["turn", "status"])?;
        let outcome = match status {
            "completed" => TurnOutcome::Completed,
            "interrupted" => TurnOutcome::Interrupted,
            "failed" => {
                let message = self.turn_errors.remove(wire_turn).unwrap_or_else(|| {
                    params
                        .pointer("/turn/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("Codex Turn failed")
                        .to_owned()
                });
                TurnOutcome::Failed(Failure::new(message))
            },
            other => {
                return Err(protocol::protocol_failure(format!(
                    "Codex Turn completed with invalid status `{other}`"
                )));
            },
        };
        let outcome = if self.secret_dispatch_attempted_for_turn(turn) {
            match outcome {
                TurnOutcome::Failed(_) => TurnOutcome::Failed(Failure::new(
                    "secret input delivery failed with an unknown outcome",
                )),
                outcome => outcome,
            }
        } else {
            outcome
        };
        let turn_finished = if outcome == TurnOutcome::Completed {
            BackendEvent::ResumableTurnFinished {
                turn,
                evidence: BackendOutcomeEvidence::without_identity(),
            }
        } else {
            BackendEvent::TurnFinished {
                turn,
                outcome: outcome.clone(),
            }
        };
        let mut completion_events = Vec::new();
        if let Some(duration_ms) = duration_ms {
            let activity = self.next_activity(turn)?;
            let seconds = duration_ms / 1000;
            let elapsed = if seconds >= 60 {
                format!(
                    "{}m {}.{:03}s",
                    seconds / 60,
                    seconds % 60,
                    duration_ms % 1000
                )
            } else {
                format!("{}.{:03}s", seconds, duration_ms % 1000)
            };
            let notice = ActivityNotice {
                level: if status == "failed" {
                    NoticeLevel::Warning
                } else {
                    NoticeLevel::Info
                },
                title: format!("Turn {status}"),
                message: format!("Duration: {elapsed} ({duration_ms} ms, reported by Codex)"),
            }
            .to_snapshot()
            .expect("bounded duration notice");
            completion_events.extend([
                BackendEvent::ActivityStarted {
                    activity,
                    kind: ActivityKind::ModelWork,
                },
                BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(notice),
                },
                BackendEvent::ActivityFinished {
                    activity,
                    outcome: ActivityOutcome::Completed,
                },
            ]);
        }
        self.wire_turns
            .get_mut(wire_turn)
            .expect("completed turn was resolved above")
            .finished = true;
        self.terminal_commands
            .retain(|_, (owner, _)| *owner != turn);
        self.file_changes.retain(|(owner, _), _| *owner != turn);
        let plan = self.plans.remove(&turn);
        let diff = self.turn_diffs.remove(&turn);
        if outcome != TurnOutcome::Interrupted {
            let mut interviews = self
                .requests
                .iter()
                .filter(|(_, binding)| {
                    binding.request_activity.turn() == turn
                        && matches!(binding.kind, RequestKind::Input(_))
                })
                .map(|(request, binding)| (*request, binding.request_activity))
                .collect::<Vec<_>>();
            interviews.sort_unstable_by_key(|(_, activity)| *activity);
            for (request, activity) in interviews {
                let summaries = self.interview_summary_events(
                    request,
                    if status == "failed" {
                        "Turn failed."
                    } else {
                        "Turn ended."
                    },
                )?;
                let binding = self
                    .requests
                    .remove(&request)
                    .expect("collected interview exists");
                self.wire_requests.remove(&wire_key(&binding.wire_id)?);
                let request_outcome = if binding.responded {
                    ActivityOutcome::Completed
                } else {
                    match &outcome {
                        TurnOutcome::Failed(failure) => ActivityOutcome::Failed(failure.clone()),
                        _ => ActivityOutcome::Interrupted,
                    }
                };
                self.pending_events
                    .push_back(BackendEvent::ActivityFinished {
                        activity,
                        outcome: request_outcome,
                    });
                self.pending_events.extend(summaries);
            }
            for activity in plan.into_iter().chain(diff) {
                let outcome = match &outcome {
                    TurnOutcome::Completed => ActivityOutcome::Completed,
                    TurnOutcome::Failed(failure) => ActivityOutcome::Failed(failure.clone()),
                    TurnOutcome::Interrupted => unreachable!("handled below"),
                };
                self.pending_events
                    .push_back(BackendEvent::ActivityFinished { activity, outcome });
            }
            self.pending_events.extend(completion_events);
            self.pending_events.push_back(turn_finished);
            return Ok(self.pending_events.pop_front());
        }
        self.wire_turns
            .get_mut(wire_turn)
            .expect("the completed Codex Turn binding was resolved above")
            .interrupted = true;

        let interrupted_items = self
            .items
            .iter()
            .filter(|(_, binding)| binding.activity.turn() == turn)
            .map(|(item_id, binding)| (item_id.clone(), binding.activity))
            .collect::<Vec<_>>();
        let mut interrupted_requests = self
            .requests
            .iter()
            .filter(|(_, binding)| binding.request_activity.turn() == turn)
            .map(|(request, binding)| (*request, binding.request_activity))
            .collect::<Vec<_>>();

        let mut interrupted_activities =
            Vec::with_capacity(interrupted_items.len() + interrupted_requests.len());
        interrupted_activities.extend(plan);
        interrupted_activities.extend(diff);
        for (item_id, activity) in interrupted_items {
            self.items.remove(&item_id);
            interrupted_activities.push(activity);
        }
        interrupted_requests.sort_unstable_by_key(|(_, activity)| *activity);
        for (request, activity) in interrupted_requests {
            completion_events.extend(self.interview_summary_events(request, "Turn interrupted.")?);
            let binding = self
                .requests
                .remove(&request)
                .expect("collected request binding must still exist");
            let key = wire_key(&binding.wire_id)?;
            self.wire_requests.remove(&key);
            interrupted_activities.push(activity);
        }
        interrupted_activities.sort_unstable();

        let mut terminal_events = interrupted_activities
            .into_iter()
            .map(|activity| BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Interrupted,
            })
            .chain(completion_events)
            .chain(iter::once(turn_finished));
        let first = terminal_events
            .next()
            .expect("an interrupted Turn always has its own terminal event");
        self.pending_events.extend(terminal_events);
        Ok(Some(first))
    }

    pub(super) fn turn_diff_updated(
        &mut self,
        params: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        self.validate_thread(params)?;
        let wire_turn = protocol::string_at(params, &["turnId"])?;
        let binding = self
            .wire_turns
            .get(wire_turn)
            .copied()
            .ok_or_else(|| protocol::protocol_failure("diff targets an unknown turn"))?;
        if binding.finished {
            return Ok(None);
        }
        let diff = protocol::string_at(params, &["diff"])?;
        let text = format!(
            "Turn aggregate diff\n{}",
            if diff.is_empty() {
                "No remaining changes reported for this turn."
            } else {
                diff
            }
        );
        if text.len() > ToolOutput::MAX_SNAPSHOT_BYTES {
            return Err(protocol::protocol_failure(
                "turn diff exceeds output presentation limit",
            ));
        }
        if let Some(&activity) = self.turn_diffs.get(&binding.turn) {
            return Ok(Some(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(text),
            }));
        }
        let activity = self.next_activity(binding.turn)?;
        self.turn_diffs.insert(binding.turn, activity);
        self.pending_events
            .push_back(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(text),
            });
        Ok(Some(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::FileChange,
        }))
    }

    pub(super) fn model_rerouted(
        &mut self,
        params: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        self.validate_thread(params)?;
        let wire_turn = protocol::string_at(params, &["turnId"])?;
        let binding =
            self.wire_turns.get(wire_turn).copied().ok_or_else(|| {
                protocol::protocol_failure("model reroute targets an unknown turn")
            })?;
        if binding.finished {
            return Ok(None);
        }
        let from = protocol::string_at(params, &["fromModel"])?;
        let to = protocol::string_at(params, &["toModel"])?;
        let reason = protocol::string_at(params, &["reason"])?;
        let message = if self.secret_dispatch_attempted() {
            "Codex reported a model change; details are redacted after secret input.".to_owned()
        } else {
            format!("Codex reported a model change.\nFrom: {from}\nTo: {to}\nReason: {reason}")
        };
        let notice = ActivityNotice {
            title: "Model rerouted".to_owned(),
            message,
            level: NoticeLevel::Warning,
        }
        .to_snapshot()
        .ok_or_else(|| {
            protocol::protocol_failure("model reroute notice exceeds the presentation bound")
        })?;
        let activity = self.next_activity(binding.turn)?;
        self.pending_events
            .push_back(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(notice),
            });
        self.pending_events
            .push_back(BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            });
        Ok(Some(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ModelWork,
        }))
    }

    pub(super) fn record_turn_error(
        &mut self,
        params: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        self.validate_thread(params)?;
        let message = protocol::string_at(params, &["error", "message"])?;
        let Some(wire_turn) = params.get("turnId").and_then(Value::as_str) else {
            let message = if self.secret_dispatch_attempted() {
                "secret input delivery failed with an unknown outcome"
            } else {
                message
            };
            return Err(BackendFailure::new(BackendFailureKind::Turn, message));
        };
        let binding = self.wire_turns.get(wire_turn).copied().ok_or_else(|| {
            protocol::protocol_failure(format!("error targets unknown Codex Turn `{wire_turn}`"))
        })?;
        if binding.finished {
            return Ok(None);
        }
        let message = if self.secret_dispatch_attempted_for_turn(binding.turn) {
            "secret input delivery failed with an unknown outcome".to_owned()
        } else {
            message.to_owned()
        };
        let will_retry = match params.get("willRetry") {
            None => false,
            Some(Value::Bool(value)) => *value,
            Some(_) => {
                return Err(protocol::protocol_failure(
                    "Codex willRetry must be boolean",
                ));
            },
        };
        if will_retry {
            // 일시적 오류는 이후의 최종 실패 사유가 되면 안 됩니다.
            self.turn_errors.remove(wire_turn);
            let activity = self.next_activity(binding.turn)?;
            let notice = ActivityNotice {
                title: "Retry announced".to_owned(),
                message: format!("{message}\nCodex will retry."),
                level: NoticeLevel::Warning,
            };
            let text = notice
                .to_snapshot()
                .unwrap_or_else(|| format!("{}\n{}", notice.title, notice.message));
            self.pending_events
                .push_back(BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(text),
                });
            self.pending_events
                .push_back(BackendEvent::ActivityFinished {
                    activity,
                    outcome: ActivityOutcome::Completed,
                });
            return Ok(Some(BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::ModelWork,
            }));
        }
        self.turn_errors.insert(wire_turn.to_owned(), message);
        Ok(None)
    }
}
