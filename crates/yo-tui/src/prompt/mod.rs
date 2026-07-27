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
pub(crate) struct PromptMeasure {
    pub(crate) desired_height: NonZeroU16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PromptMeasureError {
    ZeroWidth,
    Layout(LayoutError),
}

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

pub(crate) fn measure(
    editor: &PromptEditor,
    width: u16,
) -> Result<PromptMeasure, PromptMeasureError> {
    let layout = prompt_layout(editor, width)?;
    Ok(PromptMeasure {
        desired_height: layout.height,
    })
}

pub(crate) fn render(
    editor: &PromptEditor,
    view: &mut SurfaceView<'_>,
    style: Style,
    state: &mut PromptViewState,
) -> Result<PromptFrame, PromptRenderError> {
    if view.size().width == 0 {
        return Err(PromptRenderError::ZeroWidth);
    }
    let height = NonZeroU16::new(view.size().height).ok_or(PromptRenderError::ZeroHeight)?;
    let layout = prompt_layout(editor, view.size().width).map_err(|error| match error {
        PromptMeasureError::ZeroWidth => {
            unreachable!("the prompt view width was checked before layout")
        },
        PromptMeasureError::Layout(error) => PromptRenderError::Layout(error),
    })?;
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

fn prompt_layout(
    editor: &PromptEditor,
    width: u16,
) -> Result<crate::input::editor::layout::TextLayout, PromptMeasureError> {
    let width = NonZeroU16::new(width).ok_or(PromptMeasureError::ZeroWidth)?;
    editor.layout(width).map_err(PromptMeasureError::Layout)
}

#[cfg(test)]
mod tests;
