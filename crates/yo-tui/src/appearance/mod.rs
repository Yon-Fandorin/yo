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
const ACTIVITY_FRAME_PERIOD: Duration = Duration::from_millis(120);

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
    EmptyActivityFrames,
    ZeroActivityFramePeriod,
    InvalidActivityFrame {
        index: usize,
        cause: GraphemeError,
    },
    ActivityFrameContainsControl {
        index: usize,
    },
    ActivityFrameMustBeOneGrapheme {
        index: usize,
    },
    UnequalActivityFrameWidth {
        index: usize,
        expected: u16,
        actual: u16,
    },
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
struct ActivityMotionProfile {
    period: Duration,
    frames: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ActivityMotionFrame<'frame> {
    marker: &'frame str,
    period: Option<Duration>,
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
        let (user_marker, assistant_marker, activity_frames) = match profile {
            GlyphProfile::Rich => (
                "❯",
                "⏺",
                vec!["·", "✢", "✳", "✶", "✻", "✽", "✽", "✻", "✶", "✳", "✢", "·"],
            ),
            GlyphProfile::Ascii => (">", "*", vec![".", "*"]),
        };
        let activity_motion = validate_activity_motion(
            ACTIVITY_FRAME_PERIOD,
            activity_frames.into_iter().map(str::to_owned).collect(),
        )
        .expect("the built-in activity motion profile must always be valid");
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
        period: Duration,
        frames: &[&str],
    ) -> Result<Self, AppearanceCandidateError> {
        self.activity_motion = validate_activity_motion(
            period,
            frames.iter().map(|frame| (*frame).to_owned()).collect(),
        )?;
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

impl ActivityMotionProfile {
    fn frame_at(&self, elapsed: Duration) -> ActivityMotionFrame<'_> {
        let tick = elapsed.as_nanos() / self.period.as_nanos();
        let index = usize::try_from(tick % self.frames.len() as u128)
            .expect("a frame index is always representable as usize");
        ActivityMotionFrame {
            marker: &self.frames[index],
            period: (self.frames.len() > 1).then_some(self.period),
        }
    }
}

impl<'frame> ActivityMotionFrame<'frame> {
    pub(crate) const fn still(marker: &'frame str) -> Self {
        Self {
            marker,
            period: None,
        }
    }

    pub(crate) const fn marker(self) -> &'frame str {
        self.marker
    }

    pub(crate) const fn period(self) -> Option<Duration> {
        self.period
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

fn validate_activity_motion(
    period: Duration,
    frames: Vec<String>,
) -> Result<ActivityMotionProfile, AppearanceCandidateError> {
    if period.is_zero() {
        return Err(AppearanceCandidateError::ZeroActivityFramePeriod);
    }
    if frames.is_empty() {
        return Err(AppearanceCandidateError::EmptyActivityFrames);
    }
    let mut expected_width = None;
    for (index, frame) in frames.iter().enumerate() {
        if frame.chars().any(char::is_control) {
            return Err(AppearanceCandidateError::ActivityFrameContainsControl { index });
        }
        let mut clusters = frame.graphemes(true);
        let text = clusters
            .next()
            .ok_or(AppearanceCandidateError::InvalidActivityFrame {
                index,
                cause: GraphemeError::Empty,
            })?;
        if clusters.next().is_some() {
            return Err(AppearanceCandidateError::ActivityFrameMustBeOneGrapheme { index });
        }
        let grapheme = Grapheme::try_from(text)
            .map_err(|cause| AppearanceCandidateError::InvalidActivityFrame { index, cause })?;
        let actual = grapheme.width().get();
        if let Some(expected) = expected_width
            && actual != expected
        {
            return Err(AppearanceCandidateError::UnequalActivityFrameWidth {
                index,
                expected,
                actual,
            });
        }
        expected_width = Some(actual);
    }
    Ok(ActivityMotionProfile { period, frames })
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
            activity: Style::new(Color::Default, Color::Default, Attributes::BOLD),
            metrics: Style::new(Color::Default, Color::Default, Attributes::DIM),
            mode: Style::new(Color::Default, Color::Default, Attributes::DIM),
        },
        overlay: SelectionPanelAppearance {
            styles: SelectionPanelStyles {
                background: style,
                frame: Style::new(Color::Default, Color::Default, Attributes::DIM),
                title: Style::new(Color::Default, Color::Default, Attributes::BOLD),
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

#[cfg(test)]
mod tests;
