use serde_json::Value;
use yo_backend::transport::JsonMessagePeer;
use yo_core::{
    ActivityDocument, ActivityKind, ActivityOutcome, ActivityPlan, ActivityUpdate, BackendEvent,
    BackendFailure, PlanStep, PlanStepStatus,
};

use super::super::{
    super::state::Backend,
    snapshots::{proposed_plan_snapshot, reasoning_snapshot},
};
use crate::protocol;

impl<P: JsonMessagePeer> Backend<P> {
    pub(super) fn terminal_interaction(
        &mut self,
        params: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        self.validate_thread(params)?;
        let wire_turn = protocol::string_at(params, &["turnId"])?;
        let binding = self.wire_turns.get(wire_turn).copied().ok_or_else(|| {
            protocol::protocol_failure("terminal interaction targets an unknown turn")
        })?;
        if binding.finished {
            return Ok(None);
        }
        let item_id = protocol::string_at(params, &["itemId"])?;
        let (owner, command) = self.terminal_commands.get(item_id).ok_or_else(|| {
            protocol::protocol_failure("terminal interaction targets an unknown command")
        })?;
        if *owner != binding.turn {
            return Err(protocol::protocol_failure(
                "terminal interaction changed turn",
            ));
        }
        let process_id = protocol::string_at(params, &["processId"])?;
        let input = protocol::string_at(params, &["stdin"])?;
        let mut source = format!("Process: {process_id}");
        if let Some(command) = command {
            source.push_str(&format!("\nCommand: {command}"));
        }
        if !input.is_empty() {
            source.push_str(&format!("\nInput:\n{input}"));
        }
        // 더 긴 펜스를 사용해 터미널 입력에 Markdown 펜스가 있어도 리터럴로 유지합니다.
        let longest = source.split(|c| c != '`').map(str::len).max().unwrap_or(0);
        let fence = "`".repeat(longest.saturating_add(1).max(3));
        let snapshot = ActivityDocument {
            title: if input.is_empty() {
                "Waited for background terminal"
            } else {
                "Terminal input sent"
            }
            .to_owned(),
            markdown: format!("{fence}text\n{source}\n{fence}"),
        }
        .to_snapshot()
        .ok_or_else(|| {
            protocol::protocol_failure("terminal interaction exceeds output profile limit")
        })?;
        let activity = self.next_activity(binding.turn)?;
        self.pending_events
            .push_back(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(snapshot),
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

    pub(super) fn proposed_plan_delta(
        &mut self,
        params: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        let Some(BackendEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextDelta(delta),
        }) = self.item_delta(params)?
        else {
            unreachable!("validated delta")
        };
        let item_id = protocol::string_at(params, &["itemId"])?;
        let source = self
            .items
            .get_mut(item_id)
            .and_then(|binding| binding.proposed_plan.as_mut())
            .ok_or_else(|| protocol::protocol_failure("plan delta targets a non-plan item"))?;
        let mut next = source.clone();
        next.push_str(&delta);
        let snapshot = proposed_plan_snapshot(&next)?;
        *source = next;
        Ok(Some(BackendEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextSnapshot(snapshot),
        }))
    }

    pub(super) fn reasoning_summary_delta(
        &mut self,
        params: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        // 보존된 요약을 변경하기 전에 항목/스레드/턴 검증을 재사용합니다.
        let Some(BackendEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextDelta(delta),
        }) = self.item_delta(params)?
        else {
            unreachable!("item_delta returns an update or a protocol error");
        };
        let index = params
            .get("summaryIndex")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                protocol::protocol_failure("Codex summaryIndex must be a non-negative integer")
            })?;
        let item_id = protocol::string_at(params, &["itemId"])?;
        let summary = self
            .items
            .get_mut(item_id)
            .and_then(|binding| binding.public_summary.as_mut())
            .ok_or_else(|| {
                protocol::protocol_failure("summary delta targets a non-reasoning item")
            })?;
        // 희소 인덱스는 자리표시자 파트를 할당하면 안 됩니다. 스냅샷은 별도의 공개 요약 파트
        // 델타가 교차해도 파트 순서를 보존합니다.
        summary.entry(index).or_default().push_str(&delta);
        let text = summary
            .values()
            .filter(|part| !part.is_empty())
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n\n");
        Ok(Some(BackendEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextSnapshot(reasoning_snapshot(text).ok_or_else(|| {
                protocol::protocol_failure(
                    "Codex public reasoning summary exceeds output profile limit",
                )
            })?),
        }))
    }

    pub(super) fn plan_updated(
        &mut self,
        params: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        self.validate_thread(params)?;
        let wire_turn = protocol::string_at(params, &["turnId"])?;
        let binding = self
            .wire_turns
            .get(wire_turn)
            .copied()
            .ok_or_else(|| protocol::protocol_failure("plan targets an unknown turn"))?;
        if binding.finished {
            return Ok(None);
        }
        let steps = params
            .get("plan")
            .and_then(Value::as_array)
            .ok_or_else(|| protocol::protocol_failure("plan steps must be an array"))?;
        let mut plan_steps = Vec::with_capacity(steps.len());
        for entry in steps {
            let text = protocol::string_at(entry, &["step"])?.to_owned();
            let status = match protocol::string_at(entry, &["status"])? {
                "completed" => PlanStepStatus::Completed,
                "inProgress" => PlanStepStatus::InProgress,
                "pending" => PlanStepStatus::Pending,
                _ => {
                    return Err(protocol::protocol_failure(
                        "plan step has an unknown status",
                    ));
                },
            };
            plan_steps.push(PlanStep { text, status });
        }
        let explanation = match params.get("explanation") {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => Some(text.clone()),
            _ => return Err(protocol::protocol_failure("plan explanation must be text")),
        };
        let text = ActivityPlan {
            explanation,
            steps: plan_steps,
        }
        .to_snapshot()
        .ok_or_else(|| protocol::protocol_failure("plan exceeds output profile limit"))?;
        if let Some(&activity) = self.plans.get(&binding.turn) {
            return Ok(Some(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(text),
            }));
        }
        let activity = self.next_activity(binding.turn)?;
        self.plans.insert(binding.turn, activity);
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
}
