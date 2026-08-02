//! Read-only terminal-independent projections of one durable Session history.

use std::fmt;

use yo_core::session_repository::{
    StoredSessionContinuity, StoredSessionHistory, StoredSessionRecovery,
};

use super::{chat::ChatProjection, view::format_archival_record};
use crate::appearance::{AppearanceCandidate, AppearanceState, GlyphProfile};

/// The durable history projection selected by a non-interactive caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArchivedSessionView {
    Chat,
    Transcript,
}

/// Failure to build a read-only projection from already validated history.
#[derive(Debug)]
pub struct ArchivedProjectionError {
    detail: String,
}

impl fmt::Display for ArchivedProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for ArchivedProjectionError {}

/// Projects durable semantic history without acquiring a terminal or starting a backend.
pub fn project_archived_session(
    history: &StoredSessionHistory,
    view: ArchivedSessionView,
    glyph_profile: GlyphProfile,
) -> Result<String, ArchivedProjectionError> {
    match view {
        ArchivedSessionView::Chat => project_chat(history.records(), glyph_profile),
        ArchivedSessionView::Transcript => Ok(project_transcript(history)),
    }
}

fn project_chat(
    records: &[yo_core::TranscriptRecord],
    glyph_profile: GlyphProfile,
) -> Result<String, ArchivedProjectionError> {
    let mut chat = ChatProjection::new();
    for record in records {
        chat.observe_record(record)
            .map_err(|error| ArchivedProjectionError {
                detail: format!("projecting stored Chat history failed: {error:?}"),
            })?;
    }
    let appearance = AppearanceState::new(AppearanceCandidate::for_profile(glyph_profile))
        .expect("built-in appearance profiles must remain valid");
    chat.transcript()
        .plain_output(appearance.pin().snapshot().transcript_config())
        .map(|output| output.unwrap_or_default())
        .map_err(|error| ArchivedProjectionError {
            detail: format!("measuring stored Chat history failed: {error:?}"),
        })
}

fn project_transcript(history: &StoredSessionHistory) -> String {
    project_transcript_parts(
        history.descriptor(),
        history.journal_cutoff(),
        history.recovery(),
        history.continuity(),
        history.discovery_consistent(),
        history.records(),
    )
}

fn project_transcript_parts(
    descriptor: &yo_core::SessionDescriptor,
    journal_cutoff: Option<yo_core::JournalSequence>,
    recovery: StoredSessionRecovery,
    continuity: StoredSessionContinuity,
    discovery_consistent: bool,
    records: &[yo_core::TranscriptRecord],
) -> String {
    let cutoff = journal_cutoff.map_or_else(
        || "descriptor-only".to_owned(),
        |value| value.get().to_string(),
    );
    let recovery = match recovery {
        StoredSessionRecovery::NotRequired => "not-required",
        StoredSessionRecovery::Interrupted => "interrupted",
    };
    let continuity = match continuity {
        StoredSessionContinuity::NotObservable => "not-observable",
    };
    let discovery = if discovery_consistent {
        "consistent"
    } else {
        "mismatch"
    };
    let mut output = format!(
        "Stored Session diagnostic\n\
         session={}\n\
         workspace={}\n\
         journal_cutoff={cutoff}\n\
         message_recovery={recovery}\n\
         durability_continuity={continuity}\n\
         discovery={discovery}",
        descriptor.session_id(),
        descriptor.workspace_path(),
    );
    for (index, record) in records.iter().enumerate() {
        output.push_str("\n\n");
        output.push_str(&format_archival_record(index, record));
    }
    output
}

#[cfg(test)]
mod tests;
