//! Prompt component projection between editing state and a bounded Surface view.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the prompt component lands before its application loop consumer"
    )
)]

use std::num::NonZeroU16;

use crate::{
    input::editor::{PromptEditor, layout::LayoutError},
    surface::{Point, Style, SurfaceView, WriteOutcome},
};

mod viewport;

pub(crate) use viewport::PromptViewState;
use viewport::VisibleRows;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PromptFrame {
    pub(crate) cursor: Point,
    pub(crate) content_height: NonZeroU16,
    pub(crate) first_visible_row: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PromptRenderError {
    ZeroWidth,
    ZeroHeight,
    Layout(LayoutError),
    SurfaceConflict,
}

pub(crate) fn render(
    editor: &PromptEditor,
    view: &mut SurfaceView<'_>,
    style: Style,
    state: &mut PromptViewState,
) -> Result<PromptFrame, PromptRenderError> {
    let width = NonZeroU16::new(view.size().width).ok_or(PromptRenderError::ZeroWidth)?;
    let height = NonZeroU16::new(view.size().height).ok_or(PromptRenderError::ZeroHeight)?;
    let layout = editor.layout(width).map_err(PromptRenderError::Layout)?;
    let visible = VisibleRows::for_cursor(layout.height, layout.cursor.y, height, *state);

    if view.clear(style) == WriteOutcome::Clipped {
        return Err(PromptRenderError::SurfaceConflict);
    }

    for positioned in layout
        .glyphs
        .into_iter()
        .filter(|positioned| visible.contains(positioned.point.y))
    {
        let point = visible.translate(positioned.point);
        if view.write(point, positioned.grapheme, style) == WriteOutcome::Clipped {
            unreachable!("validated layout must fit the cleared prompt view");
        }
    }

    state.set_first_visible_row(visible.first());

    Ok(PromptFrame {
        cursor: visible.translate(layout.cursor),
        content_height: layout.height,
        first_visible_row: visible.first(),
    })
}

#[cfg(test)]
mod tests;
