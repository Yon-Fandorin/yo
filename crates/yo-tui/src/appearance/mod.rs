//! Session-owned, resolved appearance values used before Surface writes.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the first Slice keeps replacement and ASCII selection crate-private"
    )
)]

use std::sync::Arc;

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    shell::AgentShellStyles,
    surface::{Attributes, Color, Grapheme, GraphemeError, Style},
    transcript::{TranscriptLayoutConfig, TranscriptStyles},
};

const BODY_INDENT: u16 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GlyphProfile {
    Rich,
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AppearanceSnapshot {
    transcript_config: TranscriptLayoutConfig,
    styles: AgentShellStyles,
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
        let (user_marker, assistant_marker) = match profile {
            GlyphProfile::Rich => ("❯", "⏺"),
            GlyphProfile::Ascii => (">", "*"),
        };
        Self {
            user_marker: user_marker.to_owned(),
            assistant_marker: assistant_marker.to_owned(),
            styles: default_styles(),
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
}

impl AppearanceSnapshot {
    pub(crate) const fn transcript_config(&self) -> &TranscriptLayoutConfig {
        &self.transcript_config
    }

    pub(crate) const fn styles(&self) -> AgentShellStyles {
        self.styles
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

const fn default_styles() -> AgentShellStyles {
    let style = Style::new(Color::Default, Color::Default, Attributes::empty());
    AgentShellStyles {
        transcript: TranscriptStyles {
            background: style,
            user_marker: style,
            user_body: style,
            assistant_marker: style,
            assistant_body: style,
        },
        prompt: style,
    }
}

#[cfg(test)]
mod tests;
