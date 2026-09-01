//! Shared concise Chat projection for live and archived Session records.

use std::collections::HashMap;

use yo_core::{
    ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentCommand, AgentEvent,
    ContextCheckpointObservation, ContextPolicyChanged, ContextPressureDecision,
    ContextPressureObservation, TranscriptRecord, TurnOutcome,
};

use super::state::StateError;
use crate::transcript::{TranscriptItemId, TranscriptPhase, TranscriptState};

#[derive(Debug, Default)]
pub(super) struct ChatProjection {
    transcript: TranscriptState,
    publication_cursor: PublicationCursor,
    next_item_id: u64,
    activities: HashMap<ActivityRef, ActivityPresentation>,
    context_policy: Option<ContextPolicyChanged>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct PublicationCursor {
    boundary: Option<PublicationBoundary>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PublicationBoundary {
    item: TranscriptItemId,
    final_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PublicationCandidate {
    expected: PublicationCursor,
    range: std::ops::Range<usize>,
    boundary: PublicationBoundary,
}

impl PublicationCandidate {
    pub(super) fn range(&self) -> std::ops::Range<usize> {
        self.range.clone()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActivityPresentation {
    item: TranscriptItemId,
    kind: ActivityKind,
    has_payload: bool,
    visible: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ChatProjectionChange {
    Unchanged,
    VisibleItem(TranscriptItemId),
}

impl ChatProjection {
    pub(super) fn new() -> Self {
        Self {
            next_item_id: 1,
            ..Self::default()
        }
    }

    pub(super) fn transcript(&self) -> &TranscriptState {
        &self.transcript
    }

    pub(super) fn publication_candidate(&self) -> Option<PublicationCandidate> {
        let start = self.published_item_count();
        let end = self.transcript.items()[start..]
            .iter()
            .take_while(|item| item.phase() == TranscriptPhase::Final)
            .count()
            + start;
        if end == start {
            return None;
        }
        let last = self.transcript.items().get(end.checked_sub(1)?)?;
        Some(PublicationCandidate {
            expected: self.publication_cursor,
            range: start..end,
            boundary: PublicationBoundary {
                item: last.id(),
                final_revision: last.revision(),
            },
        })
    }

    pub(super) fn published_item_count(&self) -> usize {
        let Some(boundary) = self.publication_cursor.boundary else {
            return 0;
        };
        self.transcript
            .items()
            .iter()
            .position(|item| {
                item.id() == boundary.item
                    && item.revision() == boundary.final_revision
                    && item.phase() == TranscriptPhase::Final
            })
            .map(|index| index + 1)
            .expect("the publication cursor must name an immutable Final transcript item")
    }

    pub(super) fn acknowledge_publication(&mut self, candidate: &PublicationCandidate) -> bool {
        if self.publication_cursor != candidate.expected {
            return false;
        }
        let Some(last) = self
            .transcript
            .items()
            .get(candidate.range.end.saturating_sub(1))
        else {
            return false;
        };
        if last.id() != candidate.boundary.item
            || last.revision() != candidate.boundary.final_revision
            || last.phase() != TranscriptPhase::Final
        {
            return false;
        }
        self.publication_cursor.boundary = Some(candidate.boundary);
        true
    }

    pub(super) fn observe_record(
        &mut self,
        record: &TranscriptRecord,
    ) -> Result<ChatProjectionChange, StateError> {
        match record {
            TranscriptRecord::CommandCommitted(
                AgentCommand::StartTurn { input, .. } | AgentCommand::SteerTurn { input, .. },
            ) => {
                let id = self.next_transcript_id()?;
                self.transcript
                    .push_user(id, input.as_str().to_owned())
                    .map_err(StateError::Transcript)?;
                Ok(ChatProjectionChange::VisibleItem(id))
            },
            TranscriptRecord::CommandCommitted(_) => Ok(ChatProjectionChange::Unchanged),
            TranscriptRecord::EventCommitted(event) => self.observe_event(event),
            TranscriptRecord::ContextPolicyChanged(policy) => {
                self.context_policy = Some(policy.clone());
                Ok(ChatProjectionChange::Unchanged)
            },
            TranscriptRecord::ContextCheckpointCommitted(checkpoint) => {
                let id = self.push_notice(format_context_checkpoint(
                    *checkpoint,
                    self.context_policy.as_ref(),
                ))?;
                Ok(ChatProjectionChange::VisibleItem(id))
            },
        }
    }

    fn observe_event(&mut self, event: &AgentEvent) -> Result<ChatProjectionChange, StateError> {
        match event {
            AgentEvent::SessionCreated { .. } | AgentEvent::TurnStarted { .. } => {
                Ok(ChatProjectionChange::Unchanged)
            },
            AgentEvent::ActivityStarted { activity, kind } => {
                let (item, visible) = self.start_activity(*activity, *kind)?;
                Ok(if visible {
                    ChatProjectionChange::VisibleItem(item)
                } else {
                    ChatProjectionChange::Unchanged
                })
            },
            AgentEvent::ActivityUpdated { activity, update } => {
                self.update_activity(*activity, update)
            },
            AgentEvent::ActivityFinished { activity, outcome } => {
                let (item, visible) = self.finish_activity(*activity, outcome)?;
                Ok(if visible {
                    ChatProjectionChange::VisibleItem(item)
                } else {
                    ChatProjectionChange::Unchanged
                })
            },
            AgentEvent::TurnFinished { outcome, .. } => match outcome {
                TurnOutcome::Completed => Ok(ChatProjectionChange::Unchanged),
                TurnOutcome::Interrupted => self
                    .push_notice("Turn interrupted".to_owned())
                    .map(ChatProjectionChange::VisibleItem),
                TurnOutcome::Failed(failure) => self
                    .push_notice(format!("Turn failed: {}", failure.message()))
                    .map(ChatProjectionChange::VisibleItem),
            },
        }
    }

    fn update_activity(
        &mut self,
        activity: ActivityRef,
        update: &ActivityUpdate,
    ) -> Result<ChatProjectionChange, StateError> {
        let Some(presentation) = self.activities.get_mut(&activity) else {
            return Err(StateError::UnknownActivity(activity));
        };
        match update {
            ActivityUpdate::TextDelta(text) => {
                if text.is_empty() {
                    return Ok(ChatProjectionChange::Unchanged);
                }
                if !presentation.has_payload && activity_label(presentation.kind).is_some() {
                    self.transcript
                        .append_text(presentation.item, "\n")
                        .map_err(StateError::Transcript)?;
                }
                self.transcript
                    .append_text(presentation.item, text)
                    .map_err(StateError::Transcript)?;
                presentation.has_payload = true;
                presentation.visible = true;
                Ok(ChatProjectionChange::VisibleItem(presentation.item))
            },
            ActivityUpdate::TextSnapshot(text) => {
                let text = project_snapshot(presentation.kind, text.clone());
                let visible = !text.is_empty();
                let changed = self
                    .transcript
                    .replace_text_changed(presentation.item, text)
                    .map_err(StateError::Transcript)?;
                presentation.has_payload = true;
                presentation.visible = visible;
                Ok(if changed {
                    ChatProjectionChange::VisibleItem(presentation.item)
                } else {
                    ChatProjectionChange::Unchanged
                })
            },
        }
    }

    fn start_activity(
        &mut self,
        activity: ActivityRef,
        kind: ActivityKind,
    ) -> Result<(TranscriptItemId, bool), StateError> {
        let id = self.next_transcript_id()?;
        self.transcript
            .start_assistant(id)
            .map_err(StateError::Transcript)?;
        let label = activity_label(kind);
        if let Some(label) = label {
            self.transcript
                .append_text(id, label)
                .map_err(StateError::Transcript)?;
        }
        self.activities.insert(
            activity,
            ActivityPresentation {
                item: id,
                kind,
                has_payload: false,
                visible: label.is_some(),
            },
        );
        Ok((id, label.is_some()))
    }

    fn finish_activity(
        &mut self,
        activity: ActivityRef,
        outcome: &ActivityOutcome,
    ) -> Result<(TranscriptItemId, bool), StateError> {
        let Some(presentation) = self.activities.remove(&activity) else {
            return Err(StateError::UnknownActivity(activity));
        };
        let id = presentation.item;
        let visible = match outcome {
            ActivityOutcome::Completed => !presentation.visible,
            ActivityOutcome::Interrupted => self
                .transcript
                .append_text(id, "\nInterrupted")
                .map(|()| true)
                .map_err(StateError::Transcript)?,
            ActivityOutcome::Failed(failure) => self
                .transcript
                .append_text(id, &format!("\nFailed: {}", failure.message()))
                .map(|()| true)
                .map_err(StateError::Transcript)?,
        };
        self.transcript
            .finalize(id)
            .map_err(StateError::Transcript)?;
        Ok((id, visible))
    }

    pub(super) fn push_notice(&mut self, text: String) -> Result<TranscriptItemId, StateError> {
        let id = self.next_transcript_id()?;
        self.transcript
            .start_assistant(id)
            .map_err(StateError::Transcript)?;
        self.transcript
            .append_text(id, &text)
            .map_err(StateError::Transcript)?;
        self.transcript
            .finalize(id)
            .map_err(StateError::Transcript)?;
        Ok(id)
    }

    fn next_transcript_id(&mut self) -> Result<TranscriptItemId, StateError> {
        let id = TranscriptItemId::new(self.next_item_id);
        self.next_item_id = self
            .next_item_id
            .checked_add(1)
            .ok_or(StateError::ItemIdOverflow)?;
        Ok(id)
    }

    #[cfg(test)]
    pub(super) fn set_next_item_id(&mut self, value: u64) {
        self.next_item_id = value;
    }
}

fn format_context_checkpoint(
    checkpoint: ContextCheckpointObservation,
    policy: Option<&ContextPolicyChanged>,
) -> String {
    let retained_budget = policy
        .filter(|policy| policy.policy_revision() == checkpoint.policy_revision())
        .map_or_else(
            || "unknown".to_owned(),
            |policy| match (
                policy.retained_raw_percent(),
                policy.retained_raw_max_tokens(),
            ) {
                (Some(percent), Some(tokens)) => format!("{percent}% · max {tokens} tokens"),
                (Some(percent), None) => format!("{percent}%"),
                (None, Some(tokens)) => format!("max {tokens} tokens"),
                (None, None) => "none".to_owned(),
            },
        );
    let mut losses = vec![format!(
        "visible-prefix:{}",
        checkpoint.visible_prefix_loss_count()
    )];
    if checkpoint.provider_private_loss_count() > 0 {
        losses.push(format!(
            "provider-private:{}",
            checkpoint.provider_private_loss_count()
        ));
    }
    format!(
        "Context checkpoint committed\n{} → {} tokens (limit {})\ncontext epoch {} → {} · source {}..{}\nretained raw: {} group(s) (budget {retained_budget}) · artifact receipts: {} · losses: {}",
        checkpoint.input_tokens_before(),
        checkpoint.input_tokens_after(),
        checkpoint.input_token_limit(),
        checkpoint.previous_context_epoch(),
        checkpoint.successor_context_epoch(),
        checkpoint.source_anchor_sequence().get(),
        checkpoint.source_journal_boundary().get(),
        checkpoint.retained_group_count(),
        checkpoint.artifact_receipt_count(),
        losses.join(", "),
    )
}

fn project_snapshot(kind: ActivityKind, text: String) -> String {
    if kind == ActivityKind::ModelWork
        && let Some(pressure) = project_context_pressure(&text)
    {
        return pressure;
    }
    let Some(label) = activity_label(kind) else {
        return text;
    };
    if text.is_empty() {
        label.to_owned()
    } else {
        format!("{label}\n{text}")
    }
}

fn project_context_pressure(text: &str) -> Option<String> {
    let observation = ContextPressureObservation::from_snapshot_json(text)?;
    let input_tokens = observation.input_tokens();
    let input_token_limit = observation.input_token_limit();
    let trigger_percent = observation.trigger_percent();
    let decision = match observation.decision() {
        ContextPressureDecision::Admit => "near compaction threshold",
        ContextPressureDecision::Compact => "compacting before the next request",
        ContextPressureDecision::Reject => "context limit reached",
    };
    let used_percent = if input_token_limit == 0 {
        100
    } else {
        u128::from(input_tokens)
            .saturating_mul(100)
            .checked_div(u128::from(input_token_limit))?
            .min(100) as u64
    };
    Some(format!(
        "Context pressure\n{used_percent}% used ({input_tokens} / {input_token_limit} tokens) · {decision} · compacts at {trigger_percent}%"
    ))
}

const fn activity_label(kind: ActivityKind) -> Option<&'static str> {
    match kind {
        ActivityKind::ModelWork => Some("Thinking…"),
        ActivityKind::AgentMessage => None,
        ActivityKind::ToolCall => Some("Running tool…"),
        ActivityKind::ToolResult => Some("Tool result"),
        ActivityKind::FileChange => Some("File change observed"),
        ActivityKind::ApprovalRequest { .. } => {
            Some("Approval required: enter `y` to approve or `n` to decline.")
        },
        ActivityKind::ApprovalResponse { .. } => Some("Approval response sent"),
        ActivityKind::UserInputRequest { .. } => Some("Agent requested input"),
        ActivityKind::UserInputResponse { .. } => Some("Input response sent"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // context pressure receipt는 durable typed JSON을 유지하되 Chat에는 model reasoning처럼
    // 원문을 노출하지 않고 사람이 읽을 수 있는 pressure 상태로 투영합니다.
    #[test]
    fn projects_context_pressure_without_exposing_raw_telemetry() {
        let projected = project_snapshot(
            ActivityKind::ModelWork,
            ContextPressureObservation::new(86, 100, 85, 90, ContextPressureDecision::Admit)
                .unwrap()
                .to_snapshot_json(),
        );

        assert_eq!(
            projected,
            "Context pressure\n86% used (86 / 100 tokens) · near compaction threshold · compacts at 90%"
        );
        assert!(!projected.contains("yo.context-pressure"));
        assert!(!projected.contains("Thinking"));
    }
}
