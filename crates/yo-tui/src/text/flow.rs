//! Deterministic terminal-cell placement with optional source cursor mapping.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "cursor-free flow lands immediately before its transcript consumer"
    )
)]

use std::num::NonZeroU16;

use crate::surface::{Grapheme, GraphemeError, Point};

mod display;
mod engine;

use engine::{flow, validate_cursor};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PositionedGrapheme {
    pub(crate) point: Point,
    pub(crate) grapheme: Grapheme,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TextFlow {
    pub(crate) glyphs: Vec<PositionedGrapheme>,
    pub(crate) height: u16,
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
