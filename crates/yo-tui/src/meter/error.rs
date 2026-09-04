//! Typed errors produced while validating and rendering meters.

use std::{error::Error, fmt};

use crate::surface::GraphemeError;

/// The location of a glyph inside a custom MeterGlyphs value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeterGlyphSlot {
    /// The glyph repeated for filled cells in a bar.
    Filled,
    /// The glyph repeated for filled cells in a vertical bar.
    VerticalFilled,
    /// The glyph repeated for unfilled cells in a bar.
    Empty,
    /// One entry in the graduated level set.
    Level,
}

/// Errors raised when a meter definition or value cannot be rendered safely.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MeterError {
    /// No graduated glyph is available for a one-cell level meter.
    EmptyLevels,
    /// A glyph is not a valid terminal grapheme.
    InvalidGlyph {
        /// Glyph role that failed validation.
        slot: MeterGlyphSlot,
        /// Zero-based level index, or `None` for filled and empty glyphs.
        index: Option<usize>,
        /// Underlying surface grapheme validation failure.
        cause: GraphemeError,
    },
    /// A glyph is valid but occupies more or fewer than one terminal cell.
    GlyphMustBeOneCell {
        /// Glyph role that has the wrong width.
        slot: MeterGlyphSlot,
        /// Zero-based level index, or `None` for filled and empty glyphs.
        index: Option<usize>,
        /// Resolved terminal-cell width.
        width: u16,
    },
    /// A one-cell level meter has more graduated glyphs than the bounded palette allows.
    TooManyLevels {
        /// Number of supplied graduated level glyphs.
        count: usize,
    },
    /// The rendered meter exceeds the bounded cell or byte budget.
    RenderTooLarge {
        /// Number of terminal cells requested by the shape.
        cells: usize,
        /// Requested output bytes, saturated at `usize::MAX` on overflow.
        bytes: usize,
    },
    /// The configured template attempted to render an invalid value.
    Template(MeterTemplateError),
    /// A horizontal bar was configured with no cells.
    ZeroWidth,
    /// A multi-line vertical bar was configured with no rows.
    ZeroHeight,
}

impl fmt::Display for MeterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyLevels => formatter.write_str("meter level glyphs cannot be empty"),
            Self::InvalidGlyph { slot, index, cause } => {
                write!(formatter, "invalid {slot:?} glyph")?;
                if let Some(index) = index {
                    write!(formatter, " at index {index}")?;
                }
                write!(formatter, ": {cause}")
            },
            Self::GlyphMustBeOneCell { slot, index, width } => {
                write!(formatter, "{slot:?} glyph")?;
                if let Some(index) = index {
                    write!(formatter, " at index {index}")?;
                }
                write!(formatter, " occupies {width} cells; expected one")
            },
            Self::TooManyLevels { count } => {
                write!(
                    formatter,
                    "meter level glyph count {count} exceeds its bounded palette"
                )
            },
            Self::RenderTooLarge { cells, bytes } => write!(
                formatter,
                "meter output exceeds its bounded size ({cells} cells, {bytes} bytes)"
            ),
            Self::Template(error) => error.fmt(formatter),
            Self::ZeroWidth => formatter.write_str("horizontal meter width must be positive"),
            Self::ZeroHeight => formatter.write_str("vertical meter height must be positive"),
        }
    }
}

impl Error for MeterError {}

/// Errors raised by MeterTemplate expansion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MeterTemplateError {
    /// The label contains a terminal control character.
    LabelContainsControl,
    /// The template or supplied meter contains a disallowed control character.
    ControlCharacter(char),
    /// The template contains an empty pair of braces.
    EmptyPlaceholder,
    /// The template contains a placeholder other than the documented names.
    UnknownPlaceholder(String),
    /// A placeholder contains another opening brace.
    NestedPlaceholder,
    /// A placeholder has no closing brace.
    UnterminatedPlaceholder,
    /// A closing brace is not escaped or paired with an opening brace.
    UnmatchedClosingBrace,
    /// A value cannot be interpreted with the surface grapheme rules.
    InvalidGrapheme(GraphemeError),
    /// Expanded template output exceeds the meter cell or byte budget.
    OutputTooLarge {
        /// Requested output cells, saturated at `usize::MAX` when unavailable or overflowing.
        cells: usize,
        /// Requested output bytes, saturated at `usize::MAX` on overflow.
        bytes: usize,
    },
}

impl fmt::Display for MeterTemplateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LabelContainsControl => {
                formatter.write_str("meter label contains a control character")
            },
            Self::ControlCharacter(character) => write!(
                formatter,
                "meter template contains control character {}",
                character.escape_default()
            ),
            Self::EmptyPlaceholder => {
                formatter.write_str("meter template contains an empty placeholder")
            },
            Self::UnknownPlaceholder(name) => {
                write!(formatter, "unknown meter template placeholder {{{name}}}")
            },
            Self::NestedPlaceholder => {
                formatter.write_str("meter template contains a nested placeholder")
            },
            Self::UnterminatedPlaceholder => {
                formatter.write_str("meter template contains an unterminated placeholder")
            },
            Self::UnmatchedClosingBrace => {
                formatter.write_str("meter template contains an unmatched closing brace")
            },
            Self::InvalidGrapheme(error) => {
                write!(formatter, "invalid meter template text: {error}")
            },
            Self::OutputTooLarge { cells, bytes } => write!(
                formatter,
                "meter template output exceeds its bounded size ({cells} cells, {bytes} bytes)"
            ),
        }
    }
}

impl Error for MeterTemplateError {}
