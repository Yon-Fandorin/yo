use std::{
    num::NonZeroU16,
    path::{Component, Path},
    sync::Arc,
};

use url::Url;

use super::Style;

/// Occupancy of one physical cell.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CellContent {
    Blank,
    Grapheme { text: Box<str>, width: NonZeroU16 },
    Continuation { back: NonZeroU16 },
}

/// One physical cell with fully resolved content and style.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cell {
    content: CellContent,
    style: Style,
    hyperlink: Option<Hyperlink>,
}

impl Cell {
    pub(crate) fn blank(style: Style) -> Self {
        Self {
            content: CellContent::Blank,
            style,
            hyperlink: None,
        }
    }

    pub(crate) fn grapheme(text: Box<str>, width: NonZeroU16, style: Style) -> Self {
        Self {
            content: CellContent::Grapheme { text, width },
            style,
            hyperlink: None,
        }
    }

    pub(crate) fn continuation(back: NonZeroU16, style: Style) -> Self {
        Self {
            content: CellContent::Continuation { back },
            style,
            hyperlink: None,
        }
    }

    #[must_use]
    pub const fn content(&self) -> &CellContent {
        &self.content
    }

    #[must_use]
    pub const fn style(&self) -> Style {
        self.style
    }
}

/// Validated web or explicitly host-authorized file destination, separate from visible text.
/// Construction performs no network or filesystem access.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hyperlink(Arc<str>);

impl Hyperlink {
    /// Accepts an absolute HTTP(S) URL with a host, up to 8 KiB, without controls.
    /// Rejected destinations remain ordinary visible text in Markdown.
    #[must_use]
    pub fn new(destination: &str) -> Option<Self> {
        if destination.len() > 8 * 1024 || destination.chars().any(char::is_control) {
            return None;
        }
        let parsed = Url::parse(destination).ok()?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            return None;
        }
        Some(Self(Arc::from(destination)))
    }

    /// Encodes an explicitly host-authorized absolute UTF-8 file path as a local file URL.
    /// The host must verify ownership, existence and symlink resolution before calling this;
    /// construction checks representation only and performs no filesystem access.
    /// Relative paths, parent components, controls and URLs over 8 KiB are rejected.
    #[must_use]
    pub fn from_file_path(path: &Path) -> Option<Self> {
        let text = path.to_str()?;
        if !path.is_absolute()
            || text.len() > 8 * 1024
            || text.chars().any(char::is_control)
            || path
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            return None;
        }
        let destination = Url::from_file_path(path).ok()?;
        if destination.host_str().is_some() {
            return None;
        }
        let destination = destination.to_string();
        (destination.len() <= 8 * 1024).then(|| Self(Arc::from(destination)))
    }

    /// Validated URL, without ANSI encoding or URL resolution side effects.
    #[must_use]
    pub fn destination(&self) -> &str {
        &self.0
    }
}

impl Cell {
    /// Optional destination shared by every cell in this grapheme's footprint.
    #[must_use]
    pub const fn hyperlink(&self) -> Option<&Hyperlink> {
        self.hyperlink.as_ref()
    }

    pub(crate) fn with_hyperlink(mut self, hyperlink: Option<Hyperlink>) -> Self {
        self.hyperlink = hyperlink;
        self
    }
}
