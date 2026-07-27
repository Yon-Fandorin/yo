//! Deterministic prompt text placement before surface styling and clipping.

use std::num::NonZeroU16;

use unicode_segmentation::UnicodeSegmentation;

use crate::surface::{Grapheme, GraphemeError, Point};

mod display;

use display::{control_notation, tab_spaces};

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

        if text == "\t" {
            if x == width {
                x = 0;
                y = y.checked_add(1).ok_or(LayoutError::HeightOverflow)?;
            }
            let spaces = tab_spaces(x);
            for offset in 0..spaces {
                place_grapheme(
                    Grapheme::try_from(" ").expect("ASCII space is renderable"),
                    byte_index,
                    cursor,
                    offset == 0,
                    width,
                    &mut x,
                    &mut y,
                    &mut cursor_point,
                    &mut glyphs,
                )?;
            }
            continue;
        }

        if let Some(notation) = control_notation(text) {
            for (offset, character) in notation.chars().enumerate() {
                let mut encoded = [0; 4];
                let character = character.encode_utf8(&mut encoded);
                place_grapheme(
                    Grapheme::try_from(&*character).expect("ASCII control notation is renderable"),
                    byte_index,
                    cursor,
                    offset == 0,
                    width,
                    &mut x,
                    &mut y,
                    &mut cursor_point,
                    &mut glyphs,
                )?;
            }
            continue;
        }

        let grapheme = Grapheme::try_from(text)
            .map_err(|cause| LayoutError::UnrenderableGrapheme { byte_index, cause })?;
        place_grapheme(
            grapheme,
            byte_index,
            cursor,
            true,
            width,
            &mut x,
            &mut y,
            &mut cursor_point,
            &mut glyphs,
        )?;
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

#[expect(
    clippy::too_many_arguments,
    reason = "the helper advances one explicit layout cursor and output"
)]
fn place_grapheme(
    grapheme: Grapheme,
    byte_index: usize,
    source_cursor: usize,
    marks_source_start: bool,
    width: u16,
    x: &mut u16,
    y: &mut u16,
    cursor: &mut Option<Point>,
    glyphs: &mut Vec<PositionedGrapheme>,
) -> Result<(), LayoutError> {
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
        *x = 0;
        *y = y.checked_add(1).ok_or(LayoutError::HeightOverflow)?;
    }
    if marks_source_start && byte_index == source_cursor {
        *cursor = Some(Point::new(*x, *y));
    }

    glyphs.push(PositionedGrapheme {
        point: Point::new(*x, *y),
        grapheme,
    });
    *x += grapheme_width.get();
    Ok(())
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
