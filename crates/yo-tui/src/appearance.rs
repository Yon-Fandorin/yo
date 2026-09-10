//! Session-owned, resolved appearance values used before Surface writes.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "live appearance replacement remains crate-private until it has a host consumer"
    )
)]

use std::{num::NonZeroU16, sync::Arc, time::Duration};

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    shell::AgentShellStyles,
    surface::{Grapheme, GraphemeError},
    transcript::{
        AssistantRenderer, DocumentRenderer, LinkResolver, ToolRenderer, TranscriptLayoutConfig,
    },
};

const BODY_INDENT: u16 = 2;
mod activity;
mod palette;

use activity::ActivityMotionProfile;
pub(crate) use activity::{ActivityMotionFrame, ActivityStyles};
pub use activity::{ColorCapability, MotionPreference};
pub use palette::{Theme, ThemeColor, ThemeOverrides, ThemeRole};

/// Session-owned output layout preferences, independent of palette and terminal mode.
/// These affect presentation only; retained records and plain output stay complete.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutputPreferences {
    max_body_width: Option<NonZeroU16>,
    tool_head_rows: u16,
    shell_tail_rows: u16,
    diff_head_rows: u16,
    show_images: bool,
    hyperlinks: bool,
    show_reasoning: bool,
    show_diagrams: bool,
    image_max_width: NonZeroU16,
    code_padding: u16,
}

impl Default for OutputPreferences {
    fn default() -> Self {
        Self {
            max_body_width: None,
            tool_head_rows: 2,
            shell_tail_rows: 5,
            diff_head_rows: 6,
            show_images: true,
            hyperlinks: true,
            show_reasoning: true,
            show_diagrams: true,
            image_max_width: NonZeroU16::new(64).unwrap(),
            code_padding: 1,
        }
    }
}

impl OutputPreferences {
    /// Sets horizontal code-panel padding (default 1), capped at 8 cells per side.
    /// Narrow panels reduce padding to preserve readable body columns.
    #[must_use]
    pub const fn with_code_padding(mut self, columns: u16) -> Self {
        self.code_padding = if columns > 8 { 8 } else { columns };
        self
    }

    /// Enables OSC 8 HTTP(S) links on terminals that support them (default true).
    /// Visible destinations and original text remain when disabled.
    pub const fn with_hyperlinks(mut self, enabled: bool) -> Self {
        self.hyperlinks = enabled;
        self
    }

    /// Renders Mermaid fences as terminal diagrams; false retains source code blocks.
    #[must_use]
    pub const fn with_diagrams(mut self, visible: bool) -> Self {
        self.show_diagrams = visible;
        self
    }

    /// Shows supplied reasoning and public reasoning summary bodies. Hidden bodies remain in
    /// records and plain export.
    #[must_use]
    pub const fn with_reasoning(mut self, visible: bool) -> Self {
        self.show_reasoning = visible;
        self
    }

    /// Enables image decoding and display. Disabled images retain an alt-text placeholder.
    #[must_use]
    pub const fn with_images(mut self, visible: bool) -> Self {
        self.show_images = visible;
        self
    }

    /// Caps image columns, bounded by the renderer's 64-column thumbnail limit.
    #[must_use]
    pub const fn with_image_max_width(mut self, columns: NonZeroU16) -> Self {
        self.image_max_width = if columns.get() > 64 {
            NonZeroU16::new(64).unwrap()
        } else {
            columns
        };
        self
    }

    /// Caps message body columns; `None` uses the available terminal width.
    /// A one-column request uses two columns so wide Unicode remains renderable.
    #[must_use]
    pub const fn with_max_body_width(mut self, width: Option<NonZeroU16>) -> Self {
        self.max_body_width = match width {
            Some(width) if width.get() == 1 => NonZeroU16::new(2),
            other => other,
        };
        self
    }

    /// Keeps this many opening tool-log or summary rows when folded, plus the final three rows.
    /// Short logs and outcome details remain visible. Ctrl+O expands the complete log.
    #[must_use]
    pub const fn with_tool_head_rows(mut self, rows: u16) -> Self {
        self.tool_head_rows = rows;
        self
    }

    /// Keeps the latest shell output rows while retaining command and result metadata.
    /// Zero selects ordinary tool folding; Ctrl+O always restores the complete output.
    #[must_use]
    pub const fn with_shell_tail_rows(mut self, rows: u16) -> Self {
        self.shell_tail_rows = rows;
        self
    }

    /// Keeps this many opening diff rows when folded, plus the final three rows.
    /// `/changes` always displays the complete diff regardless of this setting.
    #[must_use]
    pub const fn with_diff_head_rows(mut self, rows: u16) -> Self {
        self.diff_head_rows = rows;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum GlyphProfile {
    /// Rich Unicode transcript and prompt markers.
    Rich,
    /// ASCII-only transcript and prompt markers.
    Ascii,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AppearanceGlyphRole {
    UserMarker,
    AssistantMarker,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AppearanceCandidateError {
    EmptyMarker {
        role: AppearanceGlyphRole,
    },
    MarkerContainsControl {
        role: AppearanceGlyphRole,
    },
    MarkerMustBeOneGrapheme {
        role: AppearanceGlyphRole,
    },
    UnrenderableMarker {
        role: AppearanceGlyphRole,
        cause: GraphemeError,
    },
    MarkerWiderThanIndent {
        role: AppearanceGlyphRole,
        marker_width: u16,
        body_indent: u16,
    },
    EmptyActivityMarkerFrames,
    EmptyActivityMarkerFrame {
        frame_index: usize,
    },
    ActivityMarkerFrameContainsControl {
        frame_index: usize,
    },
    InvalidActivityMarkerGrapheme {
        frame_index: usize,
        grapheme_index: usize,
        cause: GraphemeError,
    },
    ActivityMarkerGraphemeTooWide {
        frame_index: usize,
        grapheme_index: usize,
        actual: u16,
    },
    ActivityMarkerWidthOverflow {
        frame_index: usize,
    },
    ZeroActivityMarkerInterval,
    ActivityMarkerIntervalTooFast {
        minimum: Duration,
        actual: Duration,
    },
    ActivityRepaintIntervalTooFast {
        minimum: Duration,
        actual: Duration,
    },
    ZeroActivitySweepPeriod,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AppearanceCommitError {
    InvalidCandidate(AppearanceCandidateError),
    RevisionOverflow,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct AppearanceRevision(u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AppearanceCandidate {
    user_marker: String,
    assistant_marker: String,
    styles: AgentShellStyles,
    color_capability: ColorCapability,
    activity_motion: ActivityMotionProfile,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AppearanceSnapshot {
    theme: Theme,
    overrides: ThemeOverrides,
    transcript_config: TranscriptLayoutConfig,
    styles: AgentShellStyles,
    color_capability: ColorCapability,
    activity_motion: ActivityMotionProfile,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AppearancePin {
    revision: AppearanceRevision,
    snapshot: Arc<AppearanceSnapshot>,
}

#[derive(Debug)]
pub(crate) struct AppearanceState {
    revision: AppearanceRevision,
    committed: Arc<AppearanceSnapshot>,
}

impl AppearanceCandidate {
    pub(crate) fn for_profile(profile: GlyphProfile) -> Self {
        Self::for_profile_with_host_preferences(
            profile,
            ColorCapability::Unknown,
            MotionPreference::Standard,
        )
    }

    pub(crate) fn for_profile_with_host_preferences(
        profile: GlyphProfile,
        color_capability: ColorCapability,
        motion_preference: MotionPreference,
    ) -> Self {
        let (user_marker, assistant_marker, activity_frames): (&str, &str, &[&str]) = match profile
        {
            GlyphProfile::Rich => ("❯", "•", &["⠋", "⠙", "⠸", "⠴", "⠦", "⠇"]),
            GlyphProfile::Ascii => (">", "*", &["|", "/", "-", "\\"]),
        };
        let mut activity_motion =
            ActivityMotionProfile::built_in(activity_frames, color_capability, motion_preference);
        if profile == GlyphProfile::Rich {
            // Six equal steps, preserving the former ~800ms revolution.
            activity_motion =
                activity_motion.with_marker_interval(Duration::from_nanos(800_000_000 / 6));
        }
        Self {
            user_marker: user_marker.to_owned(),
            assistant_marker: assistant_marker.to_owned(),
            styles: palette::styles(profile, color_capability, Theme::Default),
            color_capability,
            activity_motion,
        }
    }

    fn resolve(self) -> Result<AppearanceSnapshot, AppearanceCandidateError> {
        validate_marker(
            &self.user_marker,
            AppearanceGlyphRole::UserMarker,
            BODY_INDENT,
        )?;
        self.activity_motion.validate()?;
        validate_marker(
            &self.assistant_marker,
            AppearanceGlyphRole::AssistantMarker,
            BODY_INDENT,
        )?;
        Ok(AppearanceSnapshot {
            theme: Theme::Default,
            overrides: ThemeOverrides::default(),
            transcript_config: TranscriptLayoutConfig::default()
                .with_body_indent(BODY_INDENT)
                .with_user_marker(self.user_marker)
                .with_assistant_marker(self.assistant_marker),
            styles: self.styles,
            color_capability: self.color_capability,
            activity_motion: self.activity_motion,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_markers_for_test(
        mut self,
        user_marker: impl Into<String>,
        assistant_marker: impl Into<String>,
    ) -> Self {
        self.user_marker = user_marker.into();
        self.assistant_marker = assistant_marker.into();
        self
    }

    #[cfg(test)]
    pub(crate) const fn with_styles_for_test(mut self, styles: AgentShellStyles) -> Self {
        self.styles = styles;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_activity_motion_for_test(
        mut self,
        repaint_interval: Duration,
        marker_interval: Duration,
        marker_frames: &[&str],
    ) -> Result<Self, AppearanceCandidateError> {
        self.activity_motion = self.activity_motion.with_test_motion(
            repaint_interval,
            marker_interval,
            marker_frames,
        )?;
        Ok(self)
    }

    #[cfg(test)]
    pub(crate) fn with_activity_sweep_period_for_test(
        mut self,
        period: Duration,
    ) -> Result<Self, AppearanceCandidateError> {
        self.activity_motion = self.activity_motion.with_test_sweep_period(period);
        self.activity_motion.validate()?;
        Ok(self)
    }
}

impl AppearanceSnapshot {
    pub(crate) const fn transcript_config(&self) -> &TranscriptLayoutConfig {
        &self.transcript_config
    }

    pub(crate) const fn styles(&self) -> AgentShellStyles {
        self.styles
    }

    pub(crate) fn activity_motion_frame(&self, elapsed: Duration) -> ActivityMotionFrame<'_> {
        self.activity_motion.frame_at(elapsed)
    }
}

impl AppearancePin {
    pub(crate) const fn revision(&self) -> AppearanceRevision {
        self.revision
    }

    pub(crate) fn snapshot(&self) -> &AppearanceSnapshot {
        &self.snapshot
    }
}

impl AppearanceState {
    pub(crate) fn select_theme_overrides(
        &mut self,
        overrides: ThemeOverrides,
    ) -> Result<AppearanceRevision, AppearanceCommitError> {
        let mut snapshot = (*self.committed).clone();
        palette::apply(
            &mut snapshot.styles,
            snapshot.color_capability,
            snapshot.theme,
        );
        overrides.apply(
            &mut snapshot.styles,
            snapshot.color_capability,
            snapshot.theme,
        );
        snapshot.overrides = overrides;
        self.publish(snapshot)
    }

    pub(crate) fn select_link_resolver(
        &mut self,
        resolver: Option<LinkResolver>,
    ) -> Result<AppearanceRevision, AppearanceCommitError> {
        let mut snapshot = (*self.committed).clone();
        snapshot.transcript_config = snapshot.transcript_config.with_link_resolver(resolver);
        self.publish(snapshot)
    }

    pub(crate) fn select_assistant_renderer(
        &mut self,
        renderer: Option<AssistantRenderer>,
    ) -> Result<AppearanceRevision, AppearanceCommitError> {
        let mut snapshot = (*self.committed).clone();
        snapshot.transcript_config = snapshot.transcript_config.with_assistant_renderer(renderer);
        self.publish(snapshot)
    }

    pub(crate) fn select_document_renderer(
        &mut self,
        renderer: Option<DocumentRenderer>,
    ) -> Result<AppearanceRevision, AppearanceCommitError> {
        let mut snapshot = (*self.committed).clone();
        snapshot.transcript_config = snapshot.transcript_config.with_document_renderer(renderer);
        self.publish(snapshot)
    }

    pub(crate) fn select_tool_renderer(
        &mut self,
        renderer: Option<ToolRenderer>,
    ) -> Result<AppearanceRevision, AppearanceCommitError> {
        let mut snapshot = (*self.committed).clone();
        snapshot.transcript_config = snapshot.transcript_config.with_tool_renderer(renderer);
        self.publish(snapshot)
    }

    pub(crate) fn select_output_preferences(
        &mut self,
        preferences: OutputPreferences,
    ) -> Result<AppearanceRevision, AppearanceCommitError> {
        let mut snapshot = (*self.committed).clone();
        snapshot.transcript_config = snapshot
            .transcript_config
            .with_max_body_width(preferences.max_body_width)
            .with_code_padding(preferences.code_padding)
            .with_hyperlinks(preferences.hyperlinks)
            .with_activity_head_rows(preferences.tool_head_rows, preferences.diff_head_rows)
            .with_shell_tail_rows(preferences.shell_tail_rows)
            .with_images(preferences.show_images, preferences.image_max_width)
            .with_reasoning(preferences.show_reasoning)
            .with_diagrams(preferences.show_diagrams);
        self.publish(snapshot)
    }
    pub(crate) fn new(candidate: AppearanceCandidate) -> Result<Self, AppearanceCandidateError> {
        Ok(Self {
            revision: AppearanceRevision(1),
            committed: Arc::new(candidate.resolve()?),
        })
    }

    pub(crate) fn select_theme(
        &mut self,
        theme: Theme,
    ) -> Result<AppearanceRevision, AppearanceCommitError> {
        let mut snapshot = (*self.committed).clone();
        palette::apply(&mut snapshot.styles, snapshot.color_capability, theme);
        snapshot.theme = theme;
        snapshot
            .overrides
            .apply(&mut snapshot.styles, snapshot.color_capability, theme);
        snapshot.activity_motion = snapshot.activity_motion.with_theme(theme);
        self.publish(snapshot)
    }

    pub(crate) fn pin(&self) -> AppearancePin {
        AppearancePin {
            revision: self.revision,
            snapshot: Arc::clone(&self.committed),
        }
    }

    pub(crate) fn commit(
        &mut self,
        candidate: AppearanceCandidate,
    ) -> Result<AppearanceRevision, AppearanceCommitError> {
        let snapshot = candidate
            .resolve()
            .map_err(AppearanceCommitError::InvalidCandidate)?;
        self.publish(snapshot)
    }

    fn publish(
        &mut self,
        snapshot: AppearanceSnapshot,
    ) -> Result<AppearanceRevision, AppearanceCommitError> {
        let revision = self
            .revision
            .0
            .checked_add(1)
            .map(AppearanceRevision)
            .ok_or(AppearanceCommitError::RevisionOverflow)?;
        self.committed = Arc::new(snapshot);
        self.revision = revision;
        Ok(revision)
    }
}

impl Default for AppearanceState {
    fn default() -> Self {
        Self::new(AppearanceCandidate::for_profile(GlyphProfile::Rich))
            .expect("the built-in Rich appearance must always be valid")
    }
}

impl AppearanceRevision {
    #[cfg(test)]
    pub(crate) const fn get(self) -> u64 {
        self.0
    }
}

fn validate_marker(
    marker: &str,
    role: AppearanceGlyphRole,
    body_indent: u16,
) -> Result<(), AppearanceCandidateError> {
    if marker.is_empty() {
        return Err(AppearanceCandidateError::EmptyMarker { role });
    }
    if marker.chars().any(char::is_control) {
        return Err(AppearanceCandidateError::MarkerContainsControl { role });
    }
    let mut graphemes = marker.graphemes(true);
    let text = graphemes
        .next()
        .ok_or(AppearanceCandidateError::EmptyMarker { role })?;
    if graphemes.next().is_some() {
        return Err(AppearanceCandidateError::MarkerMustBeOneGrapheme { role });
    }
    let grapheme = Grapheme::try_from(text)
        .map_err(|cause| AppearanceCandidateError::UnrenderableMarker { role, cause })?;
    let marker_width = grapheme.width().get();
    if marker_width > body_indent {
        return Err(AppearanceCandidateError::MarkerWiderThanIndent {
            role,
            marker_width,
            body_indent,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
