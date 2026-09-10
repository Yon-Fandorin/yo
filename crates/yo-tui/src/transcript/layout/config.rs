use std::{collections::HashMap, num::NonZeroU16, sync::Arc};

use unicode_segmentation::UnicodeSegmentation;

use super::MessageRole;
use crate::{
    surface::{Grapheme, GraphemeError},
    transcript::{
        AssistantRenderer, DocumentRenderer, LinkResolver, ToolRenderer, TranscriptItemId,
    },
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TranscriptLayoutConfig {
    max_body_width: Option<NonZeroU16>,
    body_indent: u16,
    user_marker: String,
    assistant_marker: String,
    compact_activities: bool,
    item_expansion: Arc<HashMap<TranscriptItemId, bool>>,
    tool_head_rows: u16,
    pub(super) shell_tail_rows: u16,
    diff_head_rows: u16,
    pub(super) tool_renderer: Option<ToolRenderer>,
    pub(super) assistant_renderer: Option<AssistantRenderer>,
    pub(super) document_renderer: Option<DocumentRenderer>,
    link_resolver: Option<LinkResolver>,
    pub(super) show_images: bool,
    pub(super) hyperlinks: bool,
    pub(super) show_reasoning: bool,
    pub(super) show_diagrams: bool,
    pub(super) image_max_width: NonZeroU16,
    pub(super) code_padding: u16,
}

impl Default for TranscriptLayoutConfig {
    fn default() -> Self {
        Self {
            max_body_width: None,
            body_indent: 2,
            user_marker: "❯".to_owned(),
            assistant_marker: "•".to_owned(),
            compact_activities: false,
            item_expansion: Arc::default(),
            tool_head_rows: 2,
            shell_tail_rows: 5,
            diff_head_rows: 6,
            tool_renderer: None,
            assistant_renderer: None,
            document_renderer: None,
            link_resolver: None,
            show_images: true,
            hyperlinks: true,
            show_reasoning: true,
            show_diagrams: true,
            image_max_width: NonZeroU16::new(64).unwrap(),
            code_padding: 1,
        }
    }
}

impl TranscriptLayoutConfig {
    pub(crate) fn with_assistant_renderer(mut self, renderer: Option<AssistantRenderer>) -> Self {
        self.assistant_renderer = renderer;
        self
    }

    pub(crate) const fn with_code_padding(mut self, columns: u16) -> Self {
        self.code_padding = columns;
        self
    }

    pub(crate) fn with_item_expansion(
        mut self,
        expansion: Arc<HashMap<TranscriptItemId, bool>>,
    ) -> Self {
        self.item_expansion = expansion;
        self
    }

    pub(super) fn for_item(&self, id: TranscriptItemId) -> Option<Self> {
        self.item_expansion
            .get(&id)
            .map(|expanded| self.clone().with_compact_activities(!expanded))
    }

    pub(crate) fn with_link_resolver(mut self, resolver: Option<LinkResolver>) -> Self {
        self.link_resolver = resolver;
        self
    }

    pub(super) fn active_link_resolver(&self) -> Option<&LinkResolver> {
        self.hyperlinks
            .then_some(self.link_resolver.as_ref())
            .flatten()
    }

    pub(crate) const fn with_hyperlinks(mut self, enabled: bool) -> Self {
        self.hyperlinks = enabled;
        self
    }

    pub(crate) const fn with_diagrams(mut self, visible: bool) -> Self {
        self.show_diagrams = visible;
        self
    }

    pub(crate) const fn with_reasoning(mut self, visible: bool) -> Self {
        self.show_reasoning = visible;
        self
    }

    pub(crate) fn with_document_renderer(mut self, renderer: Option<DocumentRenderer>) -> Self {
        self.document_renderer = renderer;
        self
    }

    pub(crate) fn with_tool_renderer(mut self, renderer: Option<ToolRenderer>) -> Self {
        self.tool_renderer = renderer;
        self
    }

    pub(crate) const fn with_images(mut self, show: bool, width: NonZeroU16) -> Self {
        self.show_images = show;
        self.image_max_width = width;
        self
    }

    pub(crate) const fn with_shell_tail_rows(mut self, rows: u16) -> Self {
        self.shell_tail_rows = rows;
        self
    }

    pub(crate) const fn with_activity_head_rows(mut self, tool: u16, diff: u16) -> Self {
        self.tool_head_rows = tool;
        self.diff_head_rows = diff;
        self
    }

    pub(super) const fn activity_head_rows(&self, file_change: bool) -> u16 {
        if file_change {
            self.diff_head_rows
        } else {
            self.tool_head_rows
        }
    }
    pub(crate) const fn with_compact_activities(mut self, compact: bool) -> Self {
        self.compact_activities = compact;
        self
    }

    pub(super) const fn compact_activities(&self) -> bool {
        self.compact_activities
    }
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

    pub(crate) const fn max_body_width(&self) -> Option<NonZeroU16> {
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
