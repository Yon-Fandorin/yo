//! Prompt cursor adapter over shared terminal-independent text flow.

use std::num::NonZeroU16;

pub(crate) use crate::text::flow::{CursorTextFlow as TextLayout, TextFlowError as LayoutError};
use crate::{
    surface::Point,
    text::flow::{flow_cursor_stops, flow_text_with_cursor},
};

pub(crate) fn layout_text(
    text: &str,
    cursor: usize,
    width: NonZeroU16,
) -> Result<TextLayout, LayoutError> {
    flow_text_with_cursor(text, cursor, width)
}

pub(crate) fn cursor_stops(
    text: &str,
    width: NonZeroU16,
) -> Result<Vec<(usize, Point)>, LayoutError> {
    flow_cursor_stops(text, width)
}

#[cfg(test)]
mod tests;
