//! Read-only terminal-independent projections of one durable Session history.

pub(in crate::runner) mod request;
pub(in crate::runner) mod usage;

use std::{fmt, num::NonZeroUsize};

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
    Request,
}

/// How much record payload an archived Transcript exposes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ArchivedContentPolicy {
    /// Exposes only the payload type and its UTF-8 byte length.
    None,
    /// Exposes deterministic, UTF-8-safe bounded payload previews.
    Preview,
    /// Preserves the complete legacy Transcript rendering.
    #[default]
    Full,
}

/// Selection and content bounds for an archived Transcript projection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ArchivedProjectionOptions {
    limit: Option<NonZeroUsize>,
    content: ArchivedContentPolicy,
}

impl ArchivedProjectionOptions {
    /// Creates archived Transcript projection options.
    #[must_use]
    pub const fn new(limit: Option<NonZeroUsize>, content: ArchivedContentPolicy) -> Self {
        Self { limit, content }
    }

    /// Returns the maximum number of newest semantic Transcript records to render.
    #[must_use]
    pub const fn limit(self) -> Option<NonZeroUsize> {
        self.limit
    }

    /// Returns the selected record content exposure policy.
    #[must_use]
    pub const fn content(self) -> ArchivedContentPolicy {
        self.content
    }
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
    project_archived_session_with_options(
        history,
        view,
        glyph_profile,
        ArchivedProjectionOptions::default(),
    )
}

/// Projects durable history with explicit Transcript selection and content bounds.
///
/// Non-default options are rejected for every other view: arbitrary Chat tails can
/// omit lifecycle context, while other diagnostics have their own completeness
/// contracts and typed projections.
pub fn project_archived_session_with_options(
    history: &StoredSessionHistory,
    view: ArchivedSessionView,
    glyph_profile: GlyphProfile,
    options: ArchivedProjectionOptions,
) -> Result<String, ArchivedProjectionError> {
    if view != ArchivedSessionView::Transcript && options != ArchivedProjectionOptions::default() {
        return Err(ArchivedProjectionError {
            detail: format!(
                "archived projection bounds are supported only for Transcript, not {view:?}"
            ),
        });
    }
    match view {
        ArchivedSessionView::Chat => project_chat(history.records(), glyph_profile),
        ArchivedSessionView::Transcript => Ok(project_transcript(history, options)),
        ArchivedSessionView::Request => Ok(request::project(history)),
    }
}

/// Projects the typed Usage report for one durable Session history.
pub fn project_archived_usage(
    history: &StoredSessionHistory,
    glyph_profile: GlyphProfile,
) -> Result<String, ArchivedProjectionError> {
    usage::project(history, glyph_profile)
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

fn project_transcript(
    history: &StoredSessionHistory,
    options: ArchivedProjectionOptions,
) -> String {
    project_transcript_parts_with_options(
        history.descriptor(),
        history.journal_cutoff(),
        history.recovery(),
        history.continuity(),
        history.discovery_consistent(),
        history.records(),
        options,
    )
}

#[cfg(test)]
fn project_transcript_parts(
    descriptor: &yo_core::SessionDescriptor,
    journal_cutoff: Option<yo_core::JournalSequence>,
    recovery: StoredSessionRecovery,
    continuity: StoredSessionContinuity,
    discovery_consistent: bool,
    records: &[yo_core::TranscriptRecord],
) -> String {
    project_transcript_parts_with_options(
        descriptor,
        journal_cutoff,
        recovery,
        continuity,
        discovery_consistent,
        records,
        ArchivedProjectionOptions::default(),
    )
}

fn project_transcript_parts_with_options(
    descriptor: &yo_core::SessionDescriptor,
    journal_cutoff: Option<yo_core::JournalSequence>,
    recovery: StoredSessionRecovery,
    continuity: StoredSessionContinuity,
    discovery_consistent: bool,
    records: &[yo_core::TranscriptRecord],
    options: ArchivedProjectionOptions,
) -> String {
    let cutoff = cutoff_text(journal_cutoff);
    let recovery = recovery_text(recovery);
    let continuity = continuity_text(continuity);
    let discovery = discovery_text(discovery_consistent);
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
    let first_record = options
        .limit()
        .map_or(0, |limit| records.len().saturating_sub(limit.get()));
    let selected_records = &records[first_record..];
    for (offset, record) in selected_records.iter().enumerate() {
        output.push_str("\n\n");
        output.push_str(&format_archival_record(
            first_record + offset,
            record,
            options.content(),
        ));
    }
    output
}

fn cutoff_text(journal_cutoff: Option<yo_core::JournalSequence>) -> String {
    journal_cutoff.map_or_else(
        || "descriptor-only".to_owned(),
        |value| value.get().to_string(),
    )
}

const fn recovery_text(recovery: StoredSessionRecovery) -> &'static str {
    match recovery {
        StoredSessionRecovery::NotRequired => "not-required",
        StoredSessionRecovery::Interrupted => "interrupted",
    }
}

const fn continuity_text(continuity: StoredSessionContinuity) -> &'static str {
    match continuity {
        StoredSessionContinuity::NotObservable => "not-observable",
    }
}

const fn discovery_text(discovery_consistent: bool) -> &'static str {
    if discovery_consistent {
        "consistent"
    } else {
        "mismatch"
    }
}

#[cfg(test)]
mod tests;
