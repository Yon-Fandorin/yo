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
    surface::{Point, Rect, SurfaceView, WriteOutcome},
};

mod chrome;
mod viewport;

use chrome::PromptChrome;
pub(crate) use chrome::{PromptGlyphs, PromptStyles};
pub(crate) use viewport::PromptViewState;
use viewport::VisibleRows;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PromptMeasure {
    pub(crate) desired_height: NonZeroU16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreparedPrompt {
    layout: crate::input::editor::layout::TextLayout,
    chrome: PromptChrome,
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
    let chrome = PromptChrome::new(width);
    let layout = editor
        .layout(chrome.content_width())
        .map_err(PromptMeasureError::Layout)?;
    Ok(PreparedPrompt { layout, chrome })
}

pub(crate) fn render(
    editor: &PromptEditor,
    view: &mut SurfaceView<'_>,
    styles: PromptStyles,
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

    if view.clear(styles.body) == WriteOutcome::Clipped {
        return Err(PromptRenderError::SurfaceConflict);
    }

    paint_prepared(prepared, view, styles, state).map_err(|error| match error {
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
    styles: PromptStyles,
    state: &mut PromptViewState,
) -> Result<PromptFrame, PromptPaintError> {
    if view.size().width != prepared.chrome.outer_width().get() {
        return Err(PromptPaintError::WidthMismatch {
            prepared: prepared.chrome.outer_width().get(),
            actual: view.size().width,
        });
    }
    let height = NonZeroU16::new(view.size().height).ok_or(PromptPaintError::ZeroHeight)?;
    let viewport = prepared.chrome.viewport(height);
    let visible = VisibleRows::for_cursor(
        prepared.layout.height,
        prepared.layout.cursor.y,
        NonZeroU16::new(viewport.content_size.height)
            .expect("prompt chrome always preserves a content row"),
        *state,
    );

    {
        let mut content_view = view
            .subview(Rect::new(viewport.content_origin, viewport.content_size))
            .expect("prompt chrome reserves content inside the prompt view");
        for positioned in prepared
            .layout
            .glyphs
            .into_iter()
            .filter(|positioned| visible.contains(positioned.point.y))
        {
            let point = visible.translate(positioned.point);
            if content_view.write(point, positioned.grapheme, styles.body) == WriteOutcome::Clipped
            {
                unreachable!("validated layout must fit the prompt content view");
            }
        }
    }
    prepared
        .chrome
        .paint(view, viewport, styles.glyphs, styles, visible.first());

    state.set_first_visible_row(visible.first());
    let content_cursor = visible.translate(prepared.layout.cursor);

    Ok(PromptFrame {
        cursor: Point::new(
            viewport.content_origin.x + content_cursor.x,
            viewport.content_origin.y + content_cursor.y,
        ),
        content_height: prepared.layout.height,
        first_visible_row: visible.first(),
    })
}

impl PreparedPrompt {
    pub(crate) fn desired_height(&self) -> NonZeroU16 {
        self.chrome.desired_height(self.layout.height)
    }

    pub(crate) const fn with_frame(mut self, enabled: bool) -> Self {
        self.chrome = self.chrome.with_frame(enabled);
        self
    }
}

#[cfg(test)]
mod tests;
pub(crate) mod workspace_reference;
