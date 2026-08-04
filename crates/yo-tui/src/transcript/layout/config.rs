use std::num::NonZeroU16;

use unicode_segmentation::UnicodeSegmentation;

use super::MessageRole;
use crate::surface::{Grapheme, GraphemeError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TranscriptLayoutConfig {
    max_body_width: Option<NonZeroU16>,
    body_indent: u16,
    user_marker: String,
    assistant_marker: String,
}

impl Default for TranscriptLayoutConfig {
    fn default() -> Self {
        Self {
            max_body_width: None,
            body_indent: 2,
            user_marker: "❯".to_owned(),
            assistant_marker: "•".to_owned(),
        }
    }
}

impl TranscriptLayoutConfig {
    pub(crate) fn with_max_body_width(mut self, width: Option<NonZeroU16>) -> Self {
        self.max_body_width = width;
        self
    }

    pub(crate) const fn with_body_indent(mut self, columns: u16) -> Self {
        self.body_indent = columns;
        self
    }

    pub(crate) fn with_user_marker(mut self, marker: impl Into<String>) -> Self {
        self.user_marker = marker.into();
        self
    }

    pub(crate) fn with_assistant_marker(mut self, marker: impl Into<String>) -> Self {
        self.assistant_marker = marker.into();
        self
    }

    #[cfg(test)]
    pub(crate) fn user_marker(&self) -> &str {
        &self.user_marker
    }

    #[cfg(test)]
    pub(crate) fn assistant_marker(&self) -> &str {
        &self.assistant_marker
    }

    pub(super) const fn max_body_width(&self) -> Option<NonZeroU16> {
        self.max_body_width
    }

    pub(super) const fn body_indent(&self) -> u16 {
        self.body_indent
    }

    pub(super) fn marker(&self, role: MessageRole) -> &str {
        match role {
            MessageRole::User => &self.user_marker,
            MessageRole::Assistant => &self.assistant_marker,
        }
    }

    pub(super) fn validate_for_width(
        &self,
        view_width: u16,
    ) -> Result<(), TranscriptLayoutConfigError> {
        for role in [MessageRole::User, MessageRole::Assistant] {
            let marker_width = validate_marker(self.marker(role), role, self.body_indent)?;
            if marker_width > view_width {
                return Err(TranscriptLayoutConfigError::MarkerWiderThanView {
                    role,
                    marker_width,
                    view_width,
                });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TranscriptLayoutConfigError {
    MarkerContainsControl {
        role: MessageRole,
    },
    UnrenderableMarker {
        role: MessageRole,
        cause: GraphemeError,
    },
    MarkerWidthOverflow {
        role: MessageRole,
    },
    MarkerWiderThanIndent {
        role: MessageRole,
        marker_width: u16,
        body_indent: u16,
    },
    MarkerWiderThanView {
        role: MessageRole,
        marker_width: u16,
        view_width: u16,
    },
}

fn validate_marker(
    marker: &str,
    role: MessageRole,
    body_indent: u16,
) -> Result<u16, TranscriptLayoutConfigError> {
    if marker.chars().any(char::is_control) {
        return Err(TranscriptLayoutConfigError::MarkerContainsControl { role });
    }

    let marker_width = marker.graphemes(true).try_fold(0_u16, |width, text| {
        let grapheme = Grapheme::try_from(text)
            .map_err(|cause| TranscriptLayoutConfigError::UnrenderableMarker { role, cause })?;
        width
            .checked_add(grapheme.width().get())
            .ok_or(TranscriptLayoutConfigError::MarkerWidthOverflow { role })
    })?;
    if marker_width > body_indent {
        return Err(TranscriptLayoutConfigError::MarkerWiderThanIndent {
            role,
            marker_width,
            body_indent,
        });
    }
    Ok(marker_width)
}
