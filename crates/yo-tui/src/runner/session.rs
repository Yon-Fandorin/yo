use std::{collections::BTreeMap, error::Error, fmt, num::NonZeroU16, sync::Arc};

use yo_core::{
    ActivityDocument, ActivityNotice, ImagePreparationHost, NoticeLevel, SessionId,
    session_repository::{
        ContinuationEligibility, InheritedHistorySource, InheritedSessionHistory,
        SessionTreeAncestry, SessionTreePlaceholder, StoredSessionForkCatalog,
        StoredSessionForkSourceKind, StoredSessionTree,
    },
};

use super::{
    ForkPickerToken, FrameRateLimit, PendingDispatch, PresentationMode, SkillReferenceConnection,
    WorkspaceReferenceConnection, archival::ArchivedProjectionError, state::TuiState,
};
#[cfg(test)]
use crate::appearance::AppearancePin;
use crate::{
    AssistantRenderer, DocumentRenderer, LinkResolver, OutputPreferences, PromptTemplates, Theme,
    ThemeOverrides, ToolRenderer,
    appearance::{
        AppearanceCandidate, AppearanceCommitError, AppearanceRevision, AppearanceState,
        ColorCapability, GlyphProfile, MotionPreference,
    },
    overlay::{AcceptanceReceipt, OverlayInstanceToken, PanelSnapshot, SelectionEntry, SlotError},
    text::flow::flow_text,
    transcript::TranscriptMeasureError,
};

/// Terminal-independent state retained across terminal ownership generations.
///
/// A process host can release and reacquire the terminal while keeping the
/// same session value alive. Terminal modes, presenters, and frame history are
/// deliberately not stored here.
pub struct TuiSession {
    state: TuiState,
    appearance: AppearanceState,
    pending_dispatch: Option<PendingDispatch>,
    pending_control: Option<PendingDispatch>,
    frame_rate_limit: FrameRateLimit,
    workspace_references: Option<Box<dyn WorkspaceReferenceConnection>>,
    skill_references: Option<Box<dyn SkillReferenceConnection>>,
    image_preparation: Option<Box<dyn ImagePreparationHost>>,
    publication_recovery_evidence: PublicationRecoveryEvidence,
}

/// A saved session discovered by the host; eligibility is rechecked before continuation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResumeSessionEntry {
    /// Full durable session identity.
    pub session_id: SessionId,
    /// Discovery evidence, revalidated before the host resumes execution.
    pub eligibility: ContinuationEligibility,
    /// Host-formatted durable update time, or an explicit unknown label.
    pub updated_label: String,
}

/// Host-known labels displayed in the TUI status line.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TuiSessionInfo {
    backend: Option<String>,
    workspace: String,
    startup_resumed: Option<bool>,
}

/// Validated host Markdown document, displayed without a model Turn or journal record.
/// Repeated observations append distinct documents; hosts own deduplication and retention.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TuiDocument {
    snapshot: Arc<str>,
    expanded: Option<bool>,
}

impl TuiDocument {
    /// Validates the existing ActivityDocument display limits before publishing to the TUI.
    /// Returns None for invalid or oversized documents. Construction performs no I/O.
    #[must_use]
    pub fn new(document: ActivityDocument) -> Option<Self> {
        document.to_snapshot().map(|snapshot| Self {
            snapshot: Arc::from(snapshot),
            expanded: None,
        })
    }

    /// Selects this document's initial expansion; omission inherits the current global state.
    /// Users can still toggle the item with Alt+O or reset all items with Ctrl+O.
    #[must_use]
    pub fn with_expanded(mut self, expanded: bool) -> Self {
        self.expanded = Some(expanded);
        self
    }

    pub(super) fn expanded(&self) -> Option<bool> {
        self.expanded
    }

    pub(super) fn snapshot(&self) -> &str {
        &self.snapshot
    }
}

/// Complete ephemeral host status line, ordered by key and separate from conversation history.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TuiStatusLine {
    text: String,
}

/// Invalid or oversized host status snapshot; an existing line can remain unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TuiStatusError {
    /// More than sixteen entries.
    TooManyEntries,
    /// Empty, control-bearing or over-64-byte key.
    InvalidKey,
    /// Repeated key in the same snapshot.
    DuplicateKey,
    /// Raw or escaped entry text exceeds 1024 bytes.
    TextTooLong,
    /// Text cannot be represented as a terminal line.
    InvalidText,
}

impl fmt::Display for TuiStatusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooManyEntries => "host status supports at most 16 entries",
            Self::InvalidKey => {
                "host status keys must be nonempty, control-free and at most 64 bytes"
            },
            Self::DuplicateKey => "host status keys must be unique",
            Self::TextTooLong => "host status entry exceeds 1024 bytes",
            Self::InvalidText => "host status text cannot be displayed",
        })
    }
}

impl Error for TuiStatusError {}

impl TuiStatusLine {
    /// Builds a full replacement from at most 16 unique keys and 1024-byte display values.
    /// Controls are escaped before the display-size bound. Empty values are omitted;
    /// an empty snapshot clears the line. Keys determine ordering and are not displayed.
    pub fn new<K: AsRef<str>, V: AsRef<str>>(
        entries: impl IntoIterator<Item = (K, V)>,
    ) -> Result<Self, TuiStatusError> {
        let mut sorted = BTreeMap::new();
        for (index, (key, value)) in entries.into_iter().enumerate() {
            if index >= 16 {
                return Err(TuiStatusError::TooManyEntries);
            }
            let key = key.as_ref();
            if key.is_empty() || key.len() > 64 || key.chars().any(char::is_control) {
                return Err(TuiStatusError::InvalidKey);
            }
            let value = value.as_ref();
            if value.len() > 1024 {
                return Err(TuiStatusError::TextTooLong);
            }
            let value = single_line_label(value.to_owned());
            if value.len() > 1024 {
                return Err(TuiStatusError::TextTooLong);
            }
            if sorted.insert(key.to_owned(), value).is_some() {
                return Err(TuiStatusError::DuplicateKey);
            }
        }
        let text = sorted
            .values()
            .filter(|value| !value.is_empty())
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" · ");
        let flow = flow_text(&text, NonZeroU16::MAX).map_err(|_| TuiStatusError::InvalidText)?;
        if flow.height > 1 {
            return Err(TuiStatusError::InvalidText);
        }
        Ok(Self { text })
    }

    /// Sanitized display text before terminal-width abbreviation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

/// One exact terminal-publication correction observed during this Session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PublicationRecoveryKind {
    /// Cleared a provably addressable prefix and restarted its publication.
    ReversibleRestart,
    /// Preserved a proven native-history prefix and resumed its exact suffix.
    IrreversibleResume,
    /// Retried only an unbuffered transport flush.
    FlushRetry,
}

/// Bounded environmental evidence for recovered terminal-publication errors.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PublicationRecoveryEvidence {
    reversible_restarts: u64,
    irreversible_resumes: u64,
    flush_retries: u64,
    last: Option<PublicationRecoveryKind>,
}

pub(super) struct SessionParts<'session> {
    pub(super) state: &'session mut TuiState,
    pub(super) appearance: &'session mut AppearanceState,
    pub(super) pending_dispatch: &'session mut Option<PendingDispatch>,
    pub(super) pending_control: &'session mut Option<PendingDispatch>,
    pub(super) frame_rate_limit: FrameRateLimit,
    pub(super) workspace_references: &'session mut Option<Box<dyn WorkspaceReferenceConnection>>,
    pub(super) skill_references: &'session mut Option<Box<dyn SkillReferenceConnection>>,
    pub(super) image_preparation: &'session mut Option<Box<dyn ImagePreparationHost>>,
    pub(super) publication_recovery_evidence: &'session mut PublicationRecoveryEvidence,
}

impl TuiSession {
    /// Connects the execution host's single asynchronous image preparation lane.
    pub fn with_image_preparation(mut self, host: Box<dyn ImagePreparationHost>) -> Self {
        self.image_preparation = Some(host);
        self.state.enable_image_preparation();
        self
    }

    /// Installs child-owned inherited presentation without restoring parent execution state.
    /// Call once during construction, before observing the child Session records.
    /// Unfinished source presentation or duplicate installation returns an error.
    pub fn with_inherited_history(
        mut self,
        history: &InheritedSessionHistory,
    ) -> Result<Self, ArchivedProjectionError> {
        self.state
            .observe_inherited_history(history)
            .map_err(|error| ArchivedProjectionError {
                detail: format!("projecting inherited history failed: {error:?}"),
            })?;
        Ok(self)
    }

    /// Installs host resolution of explicit Markdown links; None restores web-only links.
    /// Resolve file ownership before constructing the callback. Terminal reentry retains it.
    #[must_use]
    pub fn with_link_resolver(mut self, resolver: Option<LinkResolver>) -> Self {
        self.appearance
            .select_link_resolver(resolver)
            .expect("startup appearance revision must be available");
        self
    }

    /// Replaces the ephemeral host status without changing the conversation or provider state.
    /// Returns whether the displayed value changed. Live hosts use AgentPoll::StatusLine.
    pub fn set_status_line(&mut self, status: TuiStatusLine) -> bool {
        self.state.set_status_line(status)
    }

    /// Installs assistant-answer Markdown presentation; None restores the original answer.
    /// Terminal outcome text and source export remain unchanged. Theme changes retain the handle.
    #[must_use]
    pub fn with_assistant_renderer(mut self, renderer: Option<AssistantRenderer>) -> Self {
        self.appearance
            .select_assistant_renderer(renderer)
            .expect("startup appearance revision must be available");
        self
    }

    /// Installs an explicit document-body Markdown renderer; None restores its original body.
    /// Theme changes and terminal reentry preserve this session-local callback.
    #[must_use]
    pub fn with_document_renderer(mut self, renderer: Option<DocumentRenderer>) -> Self {
        self.appearance
            .select_document_renderer(renderer)
            .expect("startup appearance revision must be available");
        self
    }

    /// Installs a tool-body Markdown renderer; `None` restores literal tool output.
    /// Theme changes and terminal reentry preserve this session-local callback.
    #[must_use]
    pub fn with_tool_renderer(mut self, renderer: Option<ToolRenderer>) -> Self {
        self.appearance
            .select_tool_renderer(renderer)
            .expect("startup appearance revision must be available");
        self
    }

    /// Layers semantic colors over the selected theme while retaining layout and terminal
    /// capability fallback.
    #[must_use]
    pub fn with_theme_overrides(mut self, overrides: ThemeOverrides) -> Self {
        self.appearance
            .select_theme_overrides(overrides)
            .expect("theme overrides are valid and startup revision must be available");
        self
    }

    /// Installs validated user prompts for literal insertion through `/prompt`.
    #[must_use]
    pub fn with_prompt_templates(mut self, templates: PromptTemplates) -> Self {
        self.state.set_prompt_templates(templates);
        self
    }

    /// Applies output layout preferences to this session, including its offline preview.
    /// Theme changes preserve these preferences, and terminal reentry retains them.
    #[must_use]
    pub fn with_output_preferences(mut self, preferences: OutputPreferences) -> Self {
        self.appearance
            .select_output_preferences(preferences)
            .expect("output preferences are valid and startup revision must be available");
        self
    }
    /// Creates an empty Rich-glyph TUI session from explicit host appearance facts.
    #[must_use]
    pub fn new(color_capability: ColorCapability, motion_preference: MotionPreference) -> Self {
        Self::with_glyph_profile(GlyphProfile::Rich, color_capability, motion_preference)
    }

    /// Creates an empty TUI session from an explicit profile and host appearance facts.
    #[must_use]
    pub fn with_glyph_profile(
        profile: GlyphProfile,
        color_capability: ColorCapability,
        motion_preference: MotionPreference,
    ) -> Self {
        Self::with_session_info(
            profile,
            TuiSessionInfo::default(),
            color_capability,
            motion_preference,
        )
    }

    /// Creates a session with explicit host labels, glyphs, color, and motion preference.
    #[must_use]
    pub fn with_session_info(
        profile: GlyphProfile,
        info: TuiSessionInfo,
        color_capability: ColorCapability,
        motion_preference: MotionPreference,
    ) -> Self {
        Self {
            state: TuiState::with_session_info(info),
            appearance: AppearanceState::new(
                AppearanceCandidate::for_profile_with_host_preferences(
                    profile,
                    color_capability,
                    motion_preference,
                ),
            )
            .expect("built-in appearance profiles must always be valid"),
            pending_dispatch: None,
            pending_control: None,
            frame_rate_limit: FrameRateLimit::default(),
            workspace_references: None,
            skill_references: None,
            image_preparation: None,
            publication_recovery_evidence: PublicationRecoveryEvidence::default(),
        }
    }

    /// Selects a built-in palette while preserving glyphs, host capabilities, and motion.
    /// Selection belongs to this session and survives terminal suspend and resume.
    #[must_use]
    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.appearance
            .select_theme(theme)
            .expect("built-in themes must be valid and startup revision must be available");
        self
    }

    /// Selects the maximum presentation rate used to coalesce live frame requests.
    #[must_use]
    pub fn with_frame_rate_limit(mut self, limit: FrameRateLimit) -> Self {
        self.frame_rate_limit = limit;
        self
    }

    /// Installs the execution environment's nonblocking workspace provider.
    #[must_use]
    pub fn with_workspace_references(
        mut self,
        connection: impl WorkspaceReferenceConnection + 'static,
    ) -> Self {
        self.workspace_references = Some(Box::new(connection));
        self.state.enable_workspace_references();
        self
    }

    /// Installs the execution environment's nonblocking skill catalog provider.
    #[must_use]
    pub fn with_skill_references(
        mut self,
        connection: impl SkillReferenceConnection + 'static,
    ) -> Self {
        self.skill_references = Some(Box::new(connection));
        self.state.enable_skill_references();
        self
    }

    /// Installs the validated, frontend-neutral model selection controller.
    #[must_use]
    pub fn with_model_selection(mut self, controller: yo_core::ModelSelectionController) -> Self {
        self.state.enable_model_selection(controller);
        self
    }

    /// Shows a host-discovered saved-session picker. Selection is revalidated by the host.
    pub fn show_resume_picker(
        &mut self,
        sessions: Vec<ResumeSessionEntry>,
        has_more: bool,
    ) -> Result<(), String> {
        let mut entries = sessions
            .into_iter()
            .map(|session| {
                let id = session.session_id.to_string();
                if session.eligibility == ContinuationEligibility::Unavailable {
                    SelectionEntry::status(
                        id.clone(),
                        format!("Updated {} · {id} · unavailable", session.updated_label),
                    )
                } else {
                    SelectionEntry::enabled_with_context(
                        id.clone(),
                        format!("Updated {}", session.updated_label),
                        Some(id),
                        Some(
                            if session.eligibility == ContinuationEligibility::Eligible {
                                "Saved session · revalidated before resuming"
                            } else {
                                "Eligibility unknown · checked before resuming"
                            }
                            .to_owned(),
                        ),
                    )
                }
            })
            .collect::<Vec<_>>();
        if has_more {
            entries.push(SelectionEntry::status(
                "more",
                "More sessions: use yo session, then /resume UUID",
            ));
        }
        let panel = PanelSnapshot::new("Resume saved session", entries)
            .map_err(|error| format!("saved-session picker: {error:?}"))?;
        self.state
            .show_resume_picker(panel)
            .map_err(|error| format!("saved-session picker: {error:?}"))
    }

    /// Reports resume preparation failure without discarding the current conversation.
    pub fn report_resume_failure(&mut self, detail: impl Into<String>) {
        self.state.report_resume_failure(detail.into());
    }

    /// Reports cleanup failure after the saved session has been selected.
    pub fn report_resume_cleanup_failure(&mut self, detail: impl Into<String>) {
        self.state.report_resume_cleanup_failure(detail.into());
    }

    pub(super) fn take_resume_session_request(&mut self) -> Option<Option<SessionId>> {
        self.state.take_resume_session_request()
    }

    pub(super) fn take_session_tree_request(&mut self) -> bool {
        self.state.take_session_tree_request()
    }

    /// Reports a failed read-only tree query without changing the selected Session.
    pub fn report_session_tree_failure(&mut self, detail: impl Into<String>) {
        self.state.report_session_tree_failure(detail.into());
    }

    /// Displays the core's validated parent-before-child forest without inferring ancestry.
    pub fn show_session_tree(
        &mut self,
        tree: &StoredSessionTree,
        current: SessionId,
    ) -> Result<(), String> {
        let mut entries = Vec::new();
        for node in tree.nodes() {
            let id = node.session_id().to_string();
            let depth = node.depth();
            let branch = if depth == 0 {
                String::new()
            } else {
                format!("{}+- ", "  ".repeat(depth.min(8) - 1))
            };
            let mut relation = match node.ancestry() {
                SessionTreeAncestry::UnknownLegacy => {
                    "Ancestry unknown · no fork provenance".to_owned()
                },
                SessionTreeAncestry::Uninspected => "Ancestry not inspected".to_owned(),
                SessionTreeAncestry::Invalid { .. } => {
                    "Ancestry unavailable · validation failed".to_owned()
                },
                SessionTreeAncestry::ValidatedFork {
                    parent_session_id,
                    source,
                } => {
                    let source = match source {
                        InheritedHistorySource::Empty => "explicit empty source".to_owned(),
                        InheritedHistorySource::Anchor {
                            record_sequence, ..
                        } => format!("anchor {}", record_sequence.get()),
                        InheritedHistorySource::Checkpoint {
                            record_sequence, ..
                        } => format!("checkpoint {}", record_sequence.get()),
                        InheritedHistorySource::InitialFork {
                            record_sequence, ..
                        } => format!("initial fork {}", record_sequence.get()),
                    };
                    format!("Parent {parent_session_id} · {source}")
                },
            };
            if let Some(parent) = node.parent_session_id()
                && let Some(parent_node) = tree
                    .nodes()
                    .iter()
                    .find(|entry| entry.session_id() == parent)
                && let Some(placeholder) = parent_node.placeholder()
            {
                relation.push_str(match placeholder {
                    SessionTreePlaceholder::MissingAncestor => " · ancestor missing",
                    SessionTreePlaceholder::OutsideWorkspace => " · ancestor outside workspace",
                    SessionTreePlaceholder::Unavailable => " · ancestor unavailable",
                    SessionTreePlaceholder::Uninspected => " · ancestor not inspected",
                });
            }
            let status = match node.placeholder() {
                Some(SessionTreePlaceholder::MissingAncestor) => {
                    Some("Ancestor unavailable · missing")
                },
                Some(SessionTreePlaceholder::OutsideWorkspace) => {
                    Some("Ancestor outside this workspace")
                },
                Some(SessionTreePlaceholder::Unavailable) => {
                    Some("Session unavailable · workspace unverified")
                },
                Some(SessionTreePlaceholder::Uninspected) => {
                    Some("Not inspected · workspace unverified")
                },
                None if node.session_id() == current => Some("Current session"),
                None if node.metadata().is_none_or(|metadata| {
                    metadata.continuation_eligibility() == ContinuationEligibility::Unavailable
                }) =>
                {
                    Some("Continuation unavailable")
                },
                None => None,
            };
            let label = format!("{branch}{id} · depth {depth}");
            if let Some(status) = status {
                entries.push(SelectionEntry::status(
                    id,
                    format!("{label} · {status} · {relation}"),
                ));
            } else {
                entries.push(SelectionEntry::enabled_with_context(
                    id.clone(),
                    label,
                    Some(id),
                    Some(relation),
                ));
            }
        }
        if tree.truncated() {
            entries.push(SelectionEntry::status(
                "tree-limit",
                "Query limit reached · uninspected ancestry remains unknown",
            ));
        }
        if entries.is_empty() {
            entries.push(SelectionEntry::status(
                "tree-empty",
                "No sessions in this workspace",
            ));
        }
        let panel = PanelSnapshot::new("Session tree · read-only", entries)
            .map_err(|error| format!("session tree panel: {error:?}"))?
            .with_wrapped_entries();
        self.state
            .show_resume_picker(panel)
            .map_err(|error| format!("session tree panel: {error:?}"))
    }

    pub(super) fn take_fork_session_request(&mut self) -> bool {
        self.state.take_fork_session_request()
    }

    pub(super) fn take_fork_picker_request(&mut self) -> bool {
        self.state.take_fork_picker_request()
    }

    pub(super) fn take_fork_boundary_request(&mut self) -> Option<(ForkPickerToken, usize)> {
        self.state.take_fork_boundary_request()
    }

    /// Displays the host's frozen newest-first catalog and returns its picker identity.
    /// The host retains the catalog and validates this token before resolving a selected row.
    pub fn show_fork_picker(
        &mut self,
        catalog: &StoredSessionForkCatalog,
    ) -> Result<ForkPickerToken, String> {
        let result = (|| {
            let mut entries = catalog
                .boundaries()
                .iter()
                .enumerate()
                .map(|(index, boundary)| {
                    let source = match boundary.source_kind() {
                        StoredSessionForkSourceKind::Anchor => "Completed turn",
                        StoredSessionForkSourceKind::Checkpoint => "Compacted context",
                        StoredSessionForkSourceKind::InitialFork => "Inherited starting point",
                    };
                    let label = boundary.input_excerpt().map_or_else(
                        || format!("{}. {source}", index + 1),
                        |excerpt| format!("{}. {source} · {excerpt}", index + 1),
                    );
                    SelectionEntry::enabled_with_context(
                        index.to_string(),
                        label,
                        Some(boundary.model_label().to_owned()),
                        Some("Continue from this point; later messages stay in the parent conversation.".to_owned()),
                    )
                })
                .collect::<Vec<_>>();
            if entries.is_empty() {
                entries.push(SelectionEntry::status(
                    "empty",
                    "No historical fork boundaries are available",
                ));
            }
            if catalog.truncated() {
                entries.push(SelectionEntry::status(
                    "truncated",
                    "Older boundaries were omitted from this bounded list",
                ));
            }
            let panel = PanelSnapshot::new("Fork from an earlier point", entries)
                .map_err(|error| format!("fork picker: {error:?}"))?
                .with_wrapped_entries();
            self.state
                .show_fork_picker(panel, catalog.boundaries().len())
        })();
        if result.is_err() {
            self.state.cancel_fork_picker();
        }
        result
    }

    /// Identifies the source Session after a prepared child has been selected.
    pub fn report_fork_started(&mut self, parent: SessionId) {
        self.state.report_fork_started(parent);
    }

    /// Reports fork preparation failure while preserving the selected parent.
    pub fn report_fork_failure(&mut self, detail: impl Into<String>) {
        self.state.report_fork_failure(detail.into());
    }

    /// Reports previous backend cleanup failure after the child was selected.
    pub fn report_fork_cleanup_failure(&mut self, detail: impl Into<String>) {
        self.state.report_fork_cleanup_failure(detail.into());
    }

    pub(super) fn take_new_session_request(&mut self) -> bool {
        self.state.take_new_session_request()
    }

    /// Reports failure to release the previous session after selecting its successor.
    pub fn report_new_session_cleanup_failure(&mut self, detail: impl Into<String>) {
        self.state.report_new_session_cleanup_failure(detail.into());
    }

    /// Reports new-session preparation failure while retaining the current conversation.
    pub fn report_new_session_failure(&mut self, detail: impl Into<String>) {
        self.state.report_new_session_failure(detail.into());
    }

    pub(super) fn take_model_selection(&mut self) -> Option<yo_core::ModelPickerTarget> {
        self.state.take_model_selection()
    }

    /// Reports a host-side model preparation failure without discarding the live Session.
    pub fn report_model_switch_failure(&mut self, detail: impl Into<String>) {
        self.state.report_model_switch_failure(detail.into());
    }

    /// Commits the frontend projection after the core Session has durably changed binding.
    pub fn commit_model_switch(
        &mut self,
        controller: yo_core::ModelSelectionController,
        backend_label: impl Into<String>,
        cleanup_warning: Option<String>,
    ) {
        self.state
            .commit_model_switch(controller, backend_label.into(), cleanup_warning);
    }

    pub(super) fn set_presentation_mode(&mut self, mode: PresentationMode) {
        self.state.set_presentation_mode(mode);
    }

    pub(super) fn session_output(&self) -> Result<Option<String>, TranscriptMeasureError> {
        self.state.session_output(&self.appearance.pin())
    }

    /// Returns recovered publication errors retained as environmental evidence.
    #[must_use]
    pub const fn publication_recovery_evidence(&self) -> PublicationRecoveryEvidence {
        self.publication_recovery_evidence
    }

    pub(super) fn parts_mut(&mut self) -> SessionParts<'_> {
        SessionParts {
            state: &mut self.state,
            appearance: &mut self.appearance,
            pending_dispatch: &mut self.pending_dispatch,
            pending_control: &mut self.pending_control,
            frame_rate_limit: self.frame_rate_limit,
            workspace_references: &mut self.workspace_references,
            skill_references: &mut self.skill_references,
            image_preparation: &mut self.image_preparation,
            publication_recovery_evidence: &mut self.publication_recovery_evidence,
        }
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "prompt providers consume this reserved session seam"
        )
    )]
    pub(crate) fn open_prompt_overlay(
        &mut self,
        snapshot: PanelSnapshot,
    ) -> Result<OverlayInstanceToken, SlotError> {
        self.state.open_overlay(snapshot)
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "prompt providers consume this reserved session seam"
        )
    )]
    pub(crate) fn refresh_prompt_overlay(
        &mut self,
        token: OverlayInstanceToken,
        snapshot: PanelSnapshot,
    ) -> Result<(), SlotError> {
        self.state.refresh_overlay(token, snapshot)
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "prompt providers consume this reserved session seam"
        )
    )]
    pub(crate) fn close_prompt_overlay(
        &mut self,
        token: OverlayInstanceToken,
    ) -> Result<(), SlotError> {
        self.state.close_overlay(token)
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "prompt providers consume this reserved session seam"
        )
    )]
    pub(crate) fn take_prompt_overlay_acceptance(&mut self) -> Option<AcceptanceReceipt> {
        self.state.take_overlay_acceptance()
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the first Slice reserves a crate-private runtime replacement seam"
        )
    )]
    pub(crate) fn commit_appearance(
        &mut self,
        candidate: AppearanceCandidate,
    ) -> Result<AppearanceRevision, AppearanceCommitError> {
        self.appearance.commit(candidate)
    }

    #[cfg(test)]
    pub(crate) fn select_glyph_profile(
        &mut self,
        profile: GlyphProfile,
    ) -> Result<AppearanceRevision, AppearanceCommitError> {
        self.commit_appearance(AppearanceCandidate::for_profile(profile))
    }

    #[cfg(test)]
    pub(super) fn appearance_pin(&self) -> AppearancePin {
        self.appearance.pin()
    }
}

impl PublicationRecoveryEvidence {
    /// Returns how many addressable prefixes were cleared and restarted.
    #[must_use]
    pub const fn reversible_restarts(self) -> u64 {
        self.reversible_restarts
    }

    /// Returns how many proven native-history prefixes resumed from their exact suffix.
    #[must_use]
    pub const fn irreversible_resumes(self) -> u64 {
        self.irreversible_resumes
    }

    /// Returns how many unbuffered flushes were retried without replaying bytes.
    #[must_use]
    pub const fn flush_retries(self) -> u64 {
        self.flush_retries
    }

    /// Returns the most recently observed correction, if any.
    #[must_use]
    pub const fn last(self) -> Option<PublicationRecoveryKind> {
        self.last
    }

    pub(super) fn record(&mut self, recovery: crate::terminal::mode::inline::InlineRecovery) {
        let (counter, kind) = match recovery {
            crate::terminal::mode::inline::InlineRecovery::ReversibleRestart => (
                &mut self.reversible_restarts,
                PublicationRecoveryKind::ReversibleRestart,
            ),
            crate::terminal::mode::inline::InlineRecovery::IrreversibleResume => (
                &mut self.irreversible_resumes,
                PublicationRecoveryKind::IrreversibleResume,
            ),
            crate::terminal::mode::inline::InlineRecovery::FlushRetry => {
                (&mut self.flush_retries, PublicationRecoveryKind::FlushRetry)
            },
        };
        *counter = counter.saturating_add(1);
        self.last = Some(kind);
    }
}

impl TuiSessionInfo {
    /// Creates safe, single-line status labels from host-provided display values.
    #[must_use]
    pub fn new(backend: impl Into<String>, workspace: impl Into<String>) -> Self {
        Self {
            backend: non_empty_label(backend.into()),
            workspace: single_line_label(workspace.into()),
            startup_resumed: None,
        }
    }

    /// Requests one ephemeral startup notice using the host-confirmed session origin.
    /// Omit this option to keep only status-line labels. Existing notice styles apply.
    #[must_use]
    pub fn with_startup_notice(mut self, resumed: bool) -> Self {
        self.startup_resumed = Some(resumed);
        self
    }

    pub(super) fn startup_notice(&self) -> Option<ActivityNotice> {
        let resumed = self.startup_resumed?;
        let bounded = |value: &str| {
            let mut label = value.chars().take(4096).collect::<String>();
            if value.chars().nth(4096).is_some() {
                label.push('…');
            }
            label
        };
        let mut lines = Vec::new();
        if let Some(backend) = self.backend() {
            lines.push(format!("Backend: {}", bounded(backend)));
        }
        if !self.workspace.is_empty() {
            lines.push(format!("Workspace: {}", bounded(&self.workspace)));
        }
        if lines.is_empty() {
            return None;
        }
        Some(ActivityNotice {
            title: if resumed {
                "Session resumed"
            } else {
                "New session"
            }
            .to_owned(),
            message: lines.join("\n"),
            level: NoticeLevel::Info,
        })
    }

    pub(super) fn backend(&self) -> Option<&str> {
        self.backend.as_deref()
    }

    pub(super) fn set_backend(&mut self, backend: String) {
        self.backend = non_empty_label(backend);
    }

    pub(super) fn workspace(&self) -> &str {
        &self.workspace
    }
}

fn non_empty_label(value: String) -> Option<String> {
    let label = single_line_label(value);
    (!label.is_empty()).then_some(label)
}

fn single_line_label(value: String) -> String {
    let mut label = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control() {
            label.extend(character.escape_default());
        } else {
            label.push(character);
        }
    }
    label
}

#[cfg(test)]
mod tests {
    use yo_core::NoticeLevel;

    use super::{FrameRateLimit, TuiSession, TuiSessionInfo};
    use crate::{
        appearance::{ColorCapability, MotionPreference},
        overlay::{PanelSnapshot, SelectionEntry, SlotError},
    };

    fn panel(label: &str) -> PanelSnapshot {
        PanelSnapshot::new(
            "Commands",
            vec![SelectionEntry::enabled("entry", label, None)],
        )
        .unwrap()
    }

    // 외부 backend가 전달한 제어 문자는 status line의 행 구조를 바꾸지 못하고 보이는 표기로 바뀐다.
    #[test]
    fn session_info_escapes_control_characters_into_one_line() {
        let info = TuiSessionInfo::new("co\ndex", "work\tspace");

        assert_eq!(info.backend(), Some("co\\ndex"));
        assert_eq!(info.workspace(), "work\\tspace");
    }

    // 시작 안내는 opt-in이며 재개 여부·원문 제어 표기와 표시 한도의 정확한 경계를 보존한다.
    #[test]
    fn startup_notice_uses_confirmed_origin_and_bounded_labels() {
        assert!(
            TuiSessionInfo::new("native", "~/yo")
                .startup_notice()
                .is_none()
        );
        assert!(
            TuiSessionInfo::default()
                .with_startup_notice(false)
                .startup_notice()
                .is_none()
        );
        for resumed in [false, true] {
            let notice = TuiSessionInfo::new("co\ndex", "~/yo")
                .with_startup_notice(resumed)
                .startup_notice()
                .unwrap();
            assert_eq!(
                notice.title,
                if resumed {
                    "Session resumed"
                } else {
                    "New session"
                }
            );
            assert_eq!(notice.message, "Backend: co\\ndex\nWorkspace: ~/yo");
            assert_eq!(notice.level, NoticeLevel::Info);
        }
        for length in [4096, 4097] {
            let notice = TuiSessionInfo::new("가".repeat(length), "~/yo")
                .with_startup_notice(false)
                .startup_notice()
                .unwrap();
            assert_eq!(notice.message.matches('가').count(), 4096);
            assert_eq!(notice.message.contains('…'), length == 4097);
            assert!(notice.to_snapshot().is_some());
        }
    }

    // TuiSession facade는 provider가 발급받은 token을 state slot에 그대로 전달하고,
    // close 뒤 같은 token의 refresh를 stale로 거절한다.
    #[test]
    fn session_facade_preserves_overlay_token_scope() {
        let mut session = TuiSession::new(ColorCapability::Unknown, MotionPreference::Standard);
        let token = session.open_prompt_overlay(panel("First")).unwrap();

        session
            .refresh_prompt_overlay(token, panel("Updated"))
            .unwrap();
        session.close_prompt_overlay(token).unwrap();
        assert_eq!(session.take_prompt_overlay_acceptance(), None);

        assert_eq!(
            session.refresh_prompt_overlay(token, panel("Late")),
            Err(SlotError::StaleToken)
        );
    }

    // 시작 시 선택한 frame 제한은 terminal ownership generation이 parts를 다시 빌려도 유지됩니다.
    #[test]
    fn frame_rate_limit_is_retained_across_generation_borrows() {
        let mut session = TuiSession::new(ColorCapability::Unknown, MotionPreference::Standard)
            .with_frame_rate_limit(FrameRateLimit::Fps60);

        assert_eq!(session.parts_mut().frame_rate_limit, FrameRateLimit::Fps60);
        assert_eq!(session.parts_mut().frame_rate_limit, FrameRateLimit::Fps60);
    }
    // 상태 snapshot의 키·개수·escaped text 한도를 검증하고 실패한 입력은 display 값을 만들지
    // 않는다.
    #[test]
    fn host_status_snapshot_is_bounded_ordered_and_control_safe() {
        use super::{TuiStatusError, TuiStatusLine};
        let status = TuiStatusLine::new([("z", "last"), ("a", "first\n\x1b[31m")]).unwrap();
        assert_eq!(status.as_str(), "first\\n\\u{1b}[31m · last");
        for count in [16, 17] {
            let result = TuiStatusLine::new((0..count).map(|index| (index.to_string(), "x")));
            assert_eq!(result.is_ok(), count == 16);
        }
        for len in [64, 65] {
            assert_eq!(
                TuiStatusLine::new([("k".repeat(len), "value")]).is_ok(),
                len == 64
            );
        }
        for len in [1024, 1025] {
            assert_eq!(
                TuiStatusLine::new([("key", "x".repeat(len))]).is_ok(),
                len == 1024
            );
        }
        assert!(TuiStatusLine::new([("key", "\n".repeat(512))]).is_ok());
        assert_eq!(
            TuiStatusLine::new([("key", "\n".repeat(513))]),
            Err(TuiStatusError::TextTooLong)
        );
        assert_eq!(
            TuiStatusLine::new([("same", "a"), ("same", "b")]),
            Err(TuiStatusError::DuplicateKey)
        );
        for key in ["", "bad\nkey"] {
            assert_eq!(
                TuiStatusLine::new([(key, "value")]),
                Err(TuiStatusError::InvalidKey)
            );
        }
        assert_eq!(
            TuiStatusLine::new([("empty", "")]).unwrap(),
            TuiStatusLine::default()
        );
    }

    // 실제 durable fork에서 부모 파일이 없더라도 placeholder는 선택에서 건너뛰고 child는
    // 좁은 20/40열 panel에서도 기존 resume route로 선택합니다.
    #[test]
    fn session_tree_keeps_missing_parent_disabled_and_child_selectable_at_narrow_widths() {
        use std::{
            fs,
            num::NonZeroU64,
            thread,
            time::{Duration, Instant},
        };

        use yo_core::{
            AgentCommand, AgentEvent, AgentIntent, AgentSession, BackendBindingEvidence,
            BackendCommandEvidence, BackendEvent, BackendIdentity, BackendOutcomeEvidence,
            BackendRequestEvidence, BackendScriptStep, CommandAdmission, ContextPolicyChanged,
            ContextStrategy, ContinuationStrategy, HostWorkspacePath, InputSubmission,
            ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole,
            ReplayExecutor, ReplayProfile, ScriptedBackend, SessionDescriptor, SessionId,
            SubmissionId, TranscriptRecord, TurnId, TurnOutcome, TurnRef, WorkspaceHostId,
            session_repository::{
                LocalSessionReader, LocalSessionRepository, SessionTreeLimits, StoredSessionReader,
                read_stored_session_continuation,
            },
        };

        use crate::{
            input::event::{InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState},
            runner::state::StateEffect,
            surface::{CellContent, Point, Size},
        };
        let root = std::env::temp_dir().join(format!("yo-tui-tree-{}", SessionId::new().unwrap()));
        fs::create_dir(&root).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        let host: WorkspaceHostId = "10000000-0000-4000-8000-000000000001".parse().unwrap();
        let workspace = HostWorkspacePath::normalize_local(&root).unwrap();
        let parent_id = SessionId::new().unwrap();
        let descriptor = SessionDescriptor::for_session(parent_id, host, workspace.clone());
        let binding = |id: SessionId| {
            BackendBindingEvidence::new(
                "managed",
                "1",
                BackendIdentity::new("binding/v1", "account"),
                BackendIdentity::new("model/v1", "model"),
                BackendIdentity::new("locator/v1", id.to_string()),
                ContinuationStrategy::ExactReplay {
                    executor: ReplayExecutor::LocalClient,
                    replay_profile: ReplayProfile::SemanticOnly,
                },
            )
        };
        let turn = TurnRef::new(parent_id, TurnId::new(NonZeroU64::new(1).unwrap()));
        let backend = ScriptedBackend::new([
            BackendScriptStep::AcceptCommandWithEvidence {
                command: AgentCommand::CreateSession {
                    session_id: parent_id,
                },
                evidence: BackendCommandEvidence::BindingOpened(binding(parent_id)),
            },
            BackendScriptStep::Emit(BackendEvent::ContextPolicyChanged {
                policy: ContextPolicyChanged::try_new(
                    1,
                    true,
                    ContextStrategy::PortableSummaryV1Alpha1,
                    85,
                    90,
                    Some(10),
                    Some(65536),
                )
                .unwrap(),
            }),
            BackendScriptStep::AcceptCommandWithEvidence {
                command: AgentCommand::StartTurn {
                    turn,
                    input: "question".into(),
                },
                evidence: BackendCommandEvidence::RequestAccepted(BackendRequestEvidence::new(
                    "request/v1",
                    BackendIdentity::new("exchange/v1", "1"),
                    BackendIdentity::new("accepted/v1", "1"),
                )),
            },
            BackendScriptStep::Emit(BackendEvent::ResumableTurnFinished {
                turn,
                evidence: BackendOutcomeEvidence::with_identity(BackendIdentity::new(
                    "outcome/v1",
                    "1",
                ))
                .with_replay(ModelReplayDelta::new(
                    Some(ModelReplayContract::new("system", vec![])),
                    vec![
                        ModelReplayItem::Message {
                            role: ModelReplayRole::User,
                            content: "question".into(),
                            refusal: None,
                        },
                        ModelReplayItem::Message {
                            role: ModelReplayRole::Assistant,
                            content: "answer".into(),
                            refusal: None,
                        },
                    ],
                )),
            }),
            BackendScriptStep::Shutdown(Ok(())),
        ]);
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut agent = AgentSession::start_cancellable_with_repository(
            backend,
            descriptor,
            LocalSessionRepository::open(&root, 1024 * 1024).unwrap(),
            || Instant::now() >= deadline,
        )
        .unwrap()
        .unwrap();
        let mut admission = agent
            .dispatch(AgentIntent::Submit(InputSubmission::new(
                SubmissionId::new().unwrap(),
                "question".into(),
            )))
            .unwrap();
        while let CommandAdmission::Backpressured(pending) = admission {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
            admission = agent.retry(pending).unwrap();
        }
        while !agent
            .transcript_reader()
            .read_after(None)
            .entries()
            .iter()
            .any(|entry| {
                matches!(
                    entry.record(),
                    TranscriptRecord::EventCommitted(AgentEvent::TurnFinished {
                        outcome: TurnOutcome::Completed,
                        ..
                    })
                )
            })
        {
            assert!(Instant::now() < deadline);
            agent.poll().unwrap();
            thread::sleep(Duration::from_millis(1));
        }
        agent.shutdown().unwrap();
        drop(agent);
        let reader = LocalSessionReader::open(&root).unwrap();
        let parent = read_stored_session_continuation(&reader, parent_id).unwrap();
        let child_id = SessionId::new().unwrap();
        let child = parent
            .prepare_exact_fork(
                SessionDescriptor::for_session(child_id, host, workspace.clone()),
                binding(child_id),
            )
            .unwrap();
        let backend = ScriptedBackend::new([
            BackendScriptStep::Resume {
                target: Box::new(child.target().clone()),
                evidence: binding(child_id),
            },
            BackendScriptStep::Shutdown(Ok(())),
        ]);
        let mut agent = AgentSession::start_cancellable_with_continuation(
            backend,
            child,
            LocalSessionRepository::open(&root, 1024 * 1024).unwrap(),
            || Instant::now() >= deadline,
        )
        .unwrap()
        .unwrap();
        agent.shutdown().unwrap();
        drop(agent);
        fs::remove_file(root.join(format!("{parent_id}.jsonl"))).unwrap();
        let tree = reader
            .read_tree(host, &workspace, SessionTreeLimits::default())
            .unwrap();
        assert_eq!(tree.nodes().len(), 2);
        for width in [20, 40] {
            let mut tui = TuiSession::new(ColorCapability::Unknown, MotionPreference::Standard);
            tui.state
                .observe_durability(yo_core::JournalDurability::Durable {
                    journal_sequence: None,
                    repository_sequence: yo_core::session_repository::RepositorySequence::new(1),
                })
                .unwrap();
            tui.show_session_tree(&tree, parent_id).unwrap();
            let frame = tui
                .state
                .prepare_frame(Size::new(width, 24), &tui.appearance.pin())
                .unwrap();
            assert!(frame.overlay_presented);
            let visible = (0..24)
                .flat_map(|y| (0..width).map(move |x| Point::new(x, y)))
                .filter_map(|point| match frame.surface.cell(point).unwrap().content() {
                    CellContent::Grapheme { text, .. } => Some(text.to_string()),
                    _ => None,
                })
                .collect::<String>();
            let compact = visible
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect::<String>()
                .to_lowercase();
            assert!(
                compact.contains(&child_id.to_string()),
                "full child UUID missing at width {width}"
            );
            assert!(
                compact.contains(&parent_id.to_string()),
                "full parent UUID missing at width {width}"
            );
            assert!(
                compact.contains("ancestormissing"),
                "missing ancestor status clipped at width {width}"
            );
            tui.state.commit_frame(&frame);
            let result = tui
                .state
                .handle(
                    InputEvent::Key(KeyEvent {
                        code: KeyCode::Enter,
                        modifiers: KeyModifiers::NONE,
                        action: KeyAction::Press,
                        state: KeyState::NONE,
                    }),
                    Duration::ZERO,
                )
                .unwrap();
            assert_eq!(result, StateEffect::Exit);
            assert_eq!(tui.take_resume_session_request(), Some(Some(child_id)));
            let mut readonly =
                TuiSession::new(ColorCapability::Unknown, MotionPreference::Standard);
            readonly.show_session_tree(&tree, child_id).unwrap();
            let mut viewed = String::new();
            for _ in 0..2 {
                let frame = readonly
                    .state
                    .prepare_frame(Size::new(width, 24), &readonly.appearance.pin())
                    .unwrap();
                assert!(frame.overlay_presented);
                for y in 0..24 {
                    for x in 0..width {
                        if let CellContent::Grapheme { text, .. } =
                            frame.surface.cell(Point::new(x, y)).unwrap().content()
                        {
                            viewed.push_str(text);
                        }
                    }
                }
                readonly.state.commit_frame(&frame);
                readonly
                    .state
                    .handle(
                        InputEvent::Key(KeyEvent {
                            code: KeyCode::Down,
                            modifiers: KeyModifiers::NONE,
                            action: KeyAction::Press,
                            state: KeyState::NONE,
                        }),
                        Duration::ZERO,
                    )
                    .unwrap();
            }
            let compact = viewed
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect::<String>()
                .to_lowercase();
            assert!(compact.contains(&parent_id.to_string()));
            assert!(compact.contains(&child_id.to_string()));
            assert!(compact.contains("currentsession"));
            assert!(compact.contains("ancestorunavailable"));
            readonly
                .state
                .handle(
                    InputEvent::Key(KeyEvent {
                        code: KeyCode::Enter,
                        modifiers: KeyModifiers::NONE,
                        action: KeyAction::Press,
                        state: KeyState::NONE,
                    }),
                    Duration::ZERO,
                )
                .unwrap();
            assert_eq!(readonly.take_resume_session_request(), None);
        }
    }
}
