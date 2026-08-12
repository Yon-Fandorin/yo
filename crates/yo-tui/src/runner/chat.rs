//! Shared concise Chat projection for live and archived Session records.

use std::collections::HashMap;

use yo_core::{
    ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentCommand, AgentEvent,
    TranscriptRecord, TurnOutcome,
};

use super::state::StateError;
use crate::transcript::{TranscriptItemId, TranscriptPhase, TranscriptState};

#[derive(Debug, Default)]
pub(super) struct ChatProjection {
    transcript: TranscriptState,
    publication_cursor: PublicationCursor,
    next_item_id: u64,
    activities: HashMap<ActivityRef, ActivityPresentation>,
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

fn project_snapshot(kind: ActivityKind, text: String) -> String {
    let Some(label) = activity_label(kind) else {
        return text;
    };
    if text.is_empty() {
        label.to_owned()
    } else {
        format!("{label}\n{text}")
    }
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
