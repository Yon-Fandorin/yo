use std::num::NonZeroU16;

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
}

impl Cell {
    pub(crate) fn blank(style: Style) -> Self {
        Self {
            content: CellContent::Blank,
            style,
        }
    }

    pub(crate) fn grapheme(text: Box<str>, width: NonZeroU16, style: Style) -> Self {
        Self {
            content: CellContent::Grapheme { text, width },
            style,
        }
    }

    pub(crate) fn continuation(back: NonZeroU16, style: Style) -> Self {
        Self {
            content: CellContent::Continuation { back },
            style,
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
