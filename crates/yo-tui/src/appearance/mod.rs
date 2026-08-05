//! Session-owned, resolved appearance values used before Surface writes.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "live appearance replacement remains crate-private until it has a host consumer"
    )
)]

use std::{sync::Arc, time::Duration};

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    overlay::{SelectionPanelAppearance, SelectionPanelGlyphs, SelectionPanelStyles},
    prompt::{PromptGlyphs, PromptStyles},
    shell::{AgentShellStyles, ShellChromeStyles},
    surface::{Attributes, Color, Grapheme, GraphemeError, Style},
    transcript::{TranscriptLayoutConfig, TranscriptStyles},
};

const BODY_INDENT: u16 = 2;

mod activity;

use activity::ActivityMotionProfile;
pub(crate) use activity::{ActivityMotionFrame, ActivityStyles};
pub use activity::{ColorCapability, MotionPreference};

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
    activity_motion: ActivityMotionProfile,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AppearanceSnapshot {
    transcript_config: TranscriptLayoutConfig,
    styles: AgentShellStyles,
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
            GlyphProfile::Rich => (
                "❯",
                "•",
                &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
            ),
            GlyphProfile::Ascii => (">", "*", &["|", "/", "-", "\\"]),
        };
        let activity_motion =
            ActivityMotionProfile::built_in(activity_frames, color_capability, motion_preference);
        Self {
            user_marker: user_marker.to_owned(),
            assistant_marker: assistant_marker.to_owned(),
            styles: default_styles(profile),
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
            transcript_config: TranscriptLayoutConfig::default()
                .with_body_indent(BODY_INDENT)
                .with_user_marker(self.user_marker)
                .with_assistant_marker(self.assistant_marker),
            styles: self.styles,
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
    pub(crate) fn new(candidate: AppearanceCandidate) -> Result<Self, AppearanceCandidateError> {
        Ok(Self {
            revision: AppearanceRevision(1),
            committed: Arc::new(candidate.resolve()?),
        })
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

const fn default_styles(profile: GlyphProfile) -> AgentShellStyles {
    let style = Style::new(Color::Default, Color::Default, Attributes::empty());
    AgentShellStyles {
        transcript: TranscriptStyles {
            background: style,
            user_marker: style,
            user_body: style,
            assistant_marker: style,
            assistant_body: style,
        },
        prompt: PromptStyles {
            body: style,
            marker: Style::new(Color::Default, Color::Default, Attributes::BOLD),
            rule: Style::new(Color::Default, Color::Default, Attributes::DIM),
            glyphs: match profile {
                GlyphProfile::Rich => PromptGlyphs::rich(),
                GlyphProfile::Ascii => PromptGlyphs::ascii(),
            },
        },
        chrome: ShellChromeStyles {
            activity: default_activity_styles(),
            metrics: Style::new(Color::Default, Color::Default, Attributes::DIM),
            mode: Style::new(Color::Default, Color::Default, Attributes::DIM),
            key_hint: Style::new(Color::Default, Color::Default, Attributes::BOLD),
        },
        overlay: SelectionPanelAppearance {
            styles: SelectionPanelStyles {
                activity: default_activity_styles(),
                background: style,
                frame: Style::new(Color::Default, Color::Default, Attributes::DIM),
                title: Style::new(Color::Default, Color::Default, Attributes::BOLD),
                key_hint: Style::new(Color::Default, Color::Default, Attributes::BOLD),
                hint: Style::new(Color::Default, Color::Default, Attributes::DIM),
                label: style,
                detail: Style::new(Color::Default, Color::Default, Attributes::DIM),
                selected: Style::new(Color::Default, Color::Default, Attributes::BOLD),
                disabled: Style::new(Color::Default, Color::Default, Attributes::DIM),
            },
            glyphs: match profile {
                GlyphProfile::Rich => SelectionPanelGlyphs::rich(),
                GlyphProfile::Ascii => SelectionPanelGlyphs::ascii(),
            },
        },
    }
}

const fn default_activity_styles() -> ActivityStyles {
    ActivityStyles::built_in()
}

#[cfg(test)]
mod tests;
