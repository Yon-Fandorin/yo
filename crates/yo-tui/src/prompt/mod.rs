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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreparedPrompt {
    layout: crate::input::editor::layout::TextLayout,
    width: NonZeroU16,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PromptPaintError {
    WidthMismatch { prepared: u16, actual: u16 },
    ZeroHeight,
}

pub(crate) fn measure(
    editor: &PromptEditor,
    width: u16,
) -> Result<PromptMeasure, PromptMeasureError> {
    let prepared = prepare(editor, width)?;
    Ok(PromptMeasure {
        desired_height: prepared.desired_height(),
    })
}

pub(crate) fn prepare(
    editor: &PromptEditor,
    width: u16,
) -> Result<PreparedPrompt, PromptMeasureError> {
    let width = NonZeroU16::new(width).ok_or(PromptMeasureError::ZeroWidth)?;
    let layout = editor.layout(width).map_err(PromptMeasureError::Layout)?;
    Ok(PreparedPrompt { layout, width })
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
    NonZeroU16::new(view.size().height).ok_or(PromptRenderError::ZeroHeight)?;
    let prepared = prepare(editor, view.size().width).map_err(|error| match error {
        PromptMeasureError::ZeroWidth => {
            unreachable!("the prompt view width was checked before layout")
        },
        PromptMeasureError::Layout(error) => PromptRenderError::Layout(error),
    })?;

    if view.clear(style) == WriteOutcome::Clipped {
        return Err(PromptRenderError::SurfaceConflict);
    }

    paint_prepared(prepared, view, style, state).map_err(|error| match error {
        PromptPaintError::WidthMismatch { .. } => {
            unreachable!("prompt render prepares against the target view width")
        },
        PromptPaintError::ZeroHeight => {
            unreachable!("the prompt view height was checked before painting")
        },
    })
}

pub(crate) fn paint_prepared(
    prepared: PreparedPrompt,
    view: &mut SurfaceView<'_>,
    style: Style,
    state: &mut PromptViewState,
) -> Result<PromptFrame, PromptPaintError> {
    if view.size().width != prepared.width.get() {
        return Err(PromptPaintError::WidthMismatch {
            prepared: prepared.width.get(),
            actual: view.size().width,
        });
    }
    let height = NonZeroU16::new(view.size().height).ok_or(PromptPaintError::ZeroHeight)?;
    let visible = VisibleRows::for_cursor(
        prepared.layout.height,
        prepared.layout.cursor.y,
        height,
        *state,
    );

    for positioned in prepared
        .layout
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
        cursor: visible.translate(prepared.layout.cursor),
        content_height: prepared.layout.height,
        first_visible_row: visible.first(),
    })
}

impl PreparedPrompt {
    pub(crate) const fn desired_height(&self) -> NonZeroU16 {
        self.layout.height
    }
}

#[cfg(test)]
mod tests;
