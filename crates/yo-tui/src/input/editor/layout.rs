//! Prompt cursor adapter over shared terminal-independent text flow.

use std::num::NonZeroU16;

use crate::text::flow::flow_text_with_cursor;
pub(crate) use crate::text::flow::{CursorTextFlow as TextLayout, TextFlowError as LayoutError};

pub(crate) fn layout_text(
    text: &str,
    cursor: usize,
    width: NonZeroU16,
) -> Result<TextLayout, LayoutError> {
    flow_text_with_cursor(text, cursor, width)
}

#[cfg(test)]
mod tests;
