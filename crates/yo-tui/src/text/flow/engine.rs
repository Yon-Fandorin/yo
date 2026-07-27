//! Grapheme scanning and physical-cell placement for text flow.

use std::num::NonZeroU16;

use unicode_segmentation::UnicodeSegmentation;

use super::{
    PositionedGrapheme, TextFlowError,
    display::{control_notation, tab_spaces},
};
use crate::surface::{Grapheme, Point};

pub(super) struct RawFlow {
    pub(super) glyphs: Vec<PositionedGrapheme>,
    pub(super) cursor: Option<Point>,
    pub(super) content_height: u16,
}

pub(super) fn flow(
    text: &str,
    source_cursor: Option<usize>,
    width: NonZeroU16,
) -> Result<RawFlow, TextFlowError> {
    let width = width.get();
    let mut glyphs = Vec::new();
    let mut x = 0_u16;
    let mut y = 0_u16;
    let mut cursor_point = None;

    for (byte_index, text) in text.grapheme_indices(true) {
        if is_hard_break(text) {
            if source_cursor == Some(byte_index) {
                cursor_point = Some(normalized_point(x, y, width)?);
            }
            x = 0;
            y = y.checked_add(1).ok_or(TextFlowError::HeightOverflow)?;
            continue;
        }

        if text == "\t" {
            if x == width {
                x = 0;
                y = y.checked_add(1).ok_or(TextFlowError::HeightOverflow)?;
            }
            let spaces = tab_spaces(x);
            for offset in 0..spaces {
                place_grapheme(
                    Grapheme::try_from(" ").expect("ASCII space is renderable"),
                    byte_index,
                    source_cursor,
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
                    source_cursor,
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
            .map_err(|cause| TextFlowError::UnrenderableGrapheme { byte_index, cause })?;
        place_grapheme(
            grapheme,
            byte_index,
            source_cursor,
            true,
            width,
            &mut x,
            &mut y,
            &mut cursor_point,
            &mut glyphs,
        )?;
    }

    if source_cursor.is_some() && cursor_point.is_none() {
        cursor_point = Some(normalized_point(x, y, width)?);
    }
    let content_height = if text.is_empty() {
        0
    } else {
        y.checked_add(1).ok_or(TextFlowError::HeightOverflow)?
    };

    Ok(RawFlow {
        glyphs,
        cursor: cursor_point,
        content_height,
    })
}

pub(super) fn validate_cursor(text: &str, cursor: usize) -> Result<(), TextFlowError> {
    if cursor > text.len() {
        return Err(TextFlowError::CursorOutOfBounds);
    }
    if cursor == text.len()
        || text
            .grapheme_indices(true)
            .any(|(byte_index, _)| byte_index == cursor)
    {
        Ok(())
    } else {
        Err(TextFlowError::CursorNotOnGraphemeBoundary)
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the helper advances one explicit text-flow cursor and output"
)]
fn place_grapheme(
    grapheme: Grapheme,
    byte_index: usize,
    source_cursor: Option<usize>,
    marks_source_start: bool,
    width: u16,
    x: &mut u16,
    y: &mut u16,
    cursor: &mut Option<Point>,
    glyphs: &mut Vec<PositionedGrapheme>,
) -> Result<(), TextFlowError> {
    let grapheme_width = grapheme.width();
    if grapheme_width.get() > width {
        return Err(TextFlowError::GraphemeTooWide {
            byte_index,
            width: grapheme_width,
        });
    }

    if x.checked_add(grapheme_width.get())
        .is_none_or(|end| end > width)
    {
        *x = 0;
        *y = y.checked_add(1).ok_or(TextFlowError::HeightOverflow)?;
    }
    if marks_source_start && source_cursor == Some(byte_index) {
        *cursor = Some(Point::new(*x, *y));
    }

    glyphs.push(PositionedGrapheme {
        point: Point::new(*x, *y),
        grapheme,
    });
    *x += grapheme_width.get();
    Ok(())
}

fn normalized_point(x: u16, y: u16, width: u16) -> Result<Point, TextFlowError> {
    if x < width {
        return Ok(Point::new(x, y));
    }

    Ok(Point::new(
        0,
        y.checked_add(1).ok_or(TextFlowError::HeightOverflow)?,
    ))
}

fn is_hard_break(text: &str) -> bool {
    matches!(text, "\n" | "\r" | "\r\n")
}
