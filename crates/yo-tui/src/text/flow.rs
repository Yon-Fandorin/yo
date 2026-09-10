//! Deterministic terminal-cell placement with optional source cursor mapping.

use std::num::NonZeroU16;

use crate::surface::{Grapheme, GraphemeError, Point};

mod display;
mod engine;

pub(crate) use engine::TextPages;
use engine::{flow, flow_tail, validate_cursor};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PositionedGrapheme {
    /// Original input offset, shared by all cells of an expanded tab/control notation.
    pub(crate) byte_index: usize,
    pub(crate) point: Point,
    pub(crate) grapheme: Grapheme,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TextFlow {
    pub(crate) glyphs: Vec<PositionedGrapheme>,
    pub(crate) height: u16,
}

/// Bounded trailing rows, with source offsets retained and omitted display rows counted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TailTextFlow {
    pub(crate) flow: TextFlow,
    pub(crate) skipped_rows: usize,
}

pub(crate) fn flow_tail_text(
    text: &str,
    width: NonZeroU16,
    rows: NonZeroU16,
) -> Result<TailTextFlow, TextFlowError> {
    let layout = flow_tail(text, width, rows)?;
    Ok(TailTextFlow {
        flow: TextFlow {
            glyphs: layout.glyphs,
            height: layout.content_height,
        },
        skipped_rows: layout.skipped_rows,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CursorTextFlow {
    pub(crate) glyphs: Vec<PositionedGrapheme>,
    pub(crate) cursor: Point,
    pub(crate) height: NonZeroU16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TextFlowError {
    CursorOutOfBounds,
    CursorNotOnGraphemeBoundary,
    GraphemeTooWide {
        byte_index: usize,
        width: NonZeroU16,
    },
    UnrenderableGrapheme {
        byte_index: usize,
        cause: GraphemeError,
    },
    HeightOverflow,
}

pub(crate) fn flow_text(text: &str, width: NonZeroU16) -> Result<TextFlow, TextFlowError> {
    let layout = flow(text, None, width)?;
    Ok(TextFlow {
        glyphs: layout.glyphs,
        height: layout.content_height,
    })
}

/// Cursor-free prose wrapping used by parsed Markdown; leading whitespace is omitted.
pub(crate) fn flow_prose(text: &str, width: NonZeroU16) -> Result<TextFlow, TextFlowError> {
    flow_prose_with_indent(text, width, false, false)
}

/// Literal reading text wraps whole words while retaining explicit line indentation.
/// Source offsets remain original; this function never parses markup or maps an editor cursor.
pub(crate) fn flow_literal_prose(text: &str, width: NonZeroU16) -> Result<TextFlow, TextFlowError> {
    flow_prose_with_indent(text, width, true, false)
}

/// Word-aware code display that retains every whitespace glyph and source offset.
/// Long tokens still split by whole graphemes; editor cursor mapping is unchanged.
pub(crate) fn flow_code(text: &str, width: NonZeroU16) -> Result<TextFlow, TextFlowError> {
    flow_prose_with_indent(text, width, true, true)
}

// Source offsets survive wrapping so syntax and control-notation expansions retain styles.
fn flow_prose_with_indent(
    text: &str,
    width: NonZeroU16,
    preserve_indent: bool,
    preserve_spaces: bool,
) -> Result<TextFlow, TextFlowError> {
    let mut result = TextFlow {
        glyphs: Vec::new(),
        height: 0,
    };
    let mut y = 0_u16;
    let mut offset = 0;
    for (line_index, line) in text.split('\n').enumerate() {
        if line_index > 0 {
            y = y.checked_add(1).ok_or(TextFlowError::HeightOverflow)?;
        }
        let mut x = 0_u16;
        let mut leading = true;
        let flow = flow_text(line, width).map_err(|error| match error {
            TextFlowError::GraphemeTooWide { byte_index, width } => {
                TextFlowError::GraphemeTooWide {
                    byte_index: byte_index + offset,
                    width,
                }
            },
            TextFlowError::UnrenderableGrapheme { byte_index, cause } => {
                TextFlowError::UnrenderableGrapheme {
                    byte_index: byte_index + offset,
                    cause,
                }
            },
            error => error,
        })?;
        // Visual continuation space does not add glyphs or alter source offsets.
        // Bound it to a quarter of the row so narrow code keeps useful body space.
        let continuation_indent = if preserve_spaces {
            flow.glyphs
                .iter()
                .take_while(|glyph| glyph.grapheme.as_str().chars().all(char::is_whitespace))
                .map(|glyph| usize::from(glyph.grapheme.width().get()))
                .sum::<usize>()
                .min(usize::from(width.get() / 4)) as u16
        } else {
            0
        };
        for (index, source) in flow.glyphs.iter().enumerate() {
            let is_space = source.grapheme.as_str().chars().all(char::is_whitespace);
            let starts_word = !is_space
                && (index == 0
                    || flow.glyphs[index - 1]
                        .grapheme
                        .as_str()
                        .chars()
                        .all(char::is_whitespace));
            if starts_word && x > 0 && !(preserve_indent && leading) {
                let word_width: usize = flow.glyphs[index..]
                    .iter()
                    .take_while(|glyph| !glyph.grapheme.as_str().chars().all(char::is_whitespace))
                    .map(|glyph| usize::from(glyph.grapheme.width().get()))
                    .sum();
                if word_width <= usize::from(width.get() - continuation_indent)
                    && word_width > usize::from(width.get() - x)
                {
                    while !preserve_spaces
                        && result.glyphs.last().is_some_and(|glyph| {
                            glyph.point.y == y
                                && glyph.grapheme.as_str().chars().all(char::is_whitespace)
                        })
                    {
                        result.glyphs.pop();
                    }
                    x = continuation_indent;
                    y = y.checked_add(1).ok_or(TextFlowError::HeightOverflow)?;
                }
            }
            if !preserve_spaces
                && is_space
                && (x == 0 || x == width.get())
                && !(preserve_indent && leading)
            {
                continue;
            }
            leading &= is_space;
            let glyph_width = source.grapheme.width().get();
            if x.checked_add(glyph_width)
                .is_none_or(|end| end > width.get())
            {
                x = continuation_indent;
                y = y.checked_add(1).ok_or(TextFlowError::HeightOverflow)?;
            }
            let mut glyph = source.clone();
            glyph.point = Point::new(x, y);
            glyph.byte_index += offset;
            result.glyphs.push(glyph);
            x += glyph_width;
        }
        offset += line.len() + 1;
    }
    result.height = if text.is_empty() {
        0
    } else {
        y.checked_add(1).ok_or(TextFlowError::HeightOverflow)?
    };
    Ok(result)
}

pub(crate) fn flow_text_with_cursor(
    text: &str,
    cursor: usize,
    width: NonZeroU16,
) -> Result<CursorTextFlow, TextFlowError> {
    validate_cursor(text, cursor)?;
    let layout = flow(text, Some(cursor), width)?;
    let cursor = layout
        .cursor
        .expect("a requested and validated cursor is always positioned");
    let cursor_height = cursor
        .y
        .checked_add(1)
        .and_then(NonZeroU16::new)
        .ok_or(TextFlowError::HeightOverflow)?;
    let height = NonZeroU16::new(layout.content_height)
        .map_or(cursor_height, |content| content.max(cursor_height));

    Ok(CursorTextFlow {
        glyphs: layout.glyphs,
        cursor,
        height,
    })
}

#[cfg(test)]
mod tests;
