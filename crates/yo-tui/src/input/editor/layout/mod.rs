//! Deterministic prompt text placement before surface styling and clipping.

use std::num::NonZeroU16;

use unicode_segmentation::UnicodeSegmentation;

use crate::surface::{Grapheme, GraphemeError, Point};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PositionedGrapheme {
    pub(crate) point: Point,
    pub(crate) grapheme: Grapheme,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TextLayout {
    pub(crate) glyphs: Vec<PositionedGrapheme>,
    pub(crate) cursor: Point,
    pub(crate) height: NonZeroU16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LayoutError {
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

pub(crate) fn layout_text(
    text: &str,
    cursor: usize,
    width: NonZeroU16,
) -> Result<TextLayout, LayoutError> {
    validate_cursor(text, cursor)?;

    let width = width.get();
    let mut glyphs = Vec::new();
    let mut x = 0_u16;
    let mut y = 0_u16;
    let mut cursor_point = None;

    for (byte_index, text) in text.grapheme_indices(true) {
        if is_hard_break(text) {
            if byte_index == cursor {
                cursor_point = Some(normalized_point(x, y, width)?);
            }
            x = 0;
            y = y.checked_add(1).ok_or(LayoutError::HeightOverflow)?;
            continue;
        }

        let grapheme = Grapheme::try_from(text)
            .map_err(|cause| LayoutError::UnrenderableGrapheme { byte_index, cause })?;
        let grapheme_width = grapheme.width();
        if grapheme_width.get() > width {
            return Err(LayoutError::GraphemeTooWide {
                byte_index,
                width: grapheme_width,
            });
        }

        if x.checked_add(grapheme_width.get())
            .is_none_or(|end| end > width)
        {
            x = 0;
            y = y.checked_add(1).ok_or(LayoutError::HeightOverflow)?;
        }
        if byte_index == cursor {
            cursor_point = Some(Point::new(x, y));
        }

        glyphs.push(PositionedGrapheme {
            point: Point::new(x, y),
            grapheme,
        });
        x += grapheme_width.get();
    }

    let cursor = match cursor_point {
        Some(point) => point,
        None => normalized_point(x, y, width)?,
    };
    let height = y
        .max(cursor.y)
        .checked_add(1)
        .and_then(NonZeroU16::new)
        .ok_or(LayoutError::HeightOverflow)?;

    Ok(TextLayout {
        glyphs,
        cursor,
        height,
    })
}

fn validate_cursor(text: &str, cursor: usize) -> Result<(), LayoutError> {
    if cursor > text.len() {
        return Err(LayoutError::CursorOutOfBounds);
    }
    if cursor == text.len()
        || text
            .grapheme_indices(true)
            .any(|(byte_index, _)| byte_index == cursor)
    {
        Ok(())
    } else {
        Err(LayoutError::CursorNotOnGraphemeBoundary)
    }
}

fn normalized_point(x: u16, y: u16, width: u16) -> Result<Point, LayoutError> {
    if x < width {
        return Ok(Point::new(x, y));
    }

    Ok(Point::new(
        0,
        y.checked_add(1).ok_or(LayoutError::HeightOverflow)?,
    ))
}

fn is_hard_break(text: &str) -> bool {
    matches!(text, "\n" | "\r" | "\r\n")
}

#[cfg(test)]
mod tests;
