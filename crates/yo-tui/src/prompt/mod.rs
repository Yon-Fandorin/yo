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
    input::editor::{
        PromptEditor,
        layout::{LayoutError, TextLayout},
    },
    surface::{Point, Style, SurfaceView, WriteOutcome},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PromptFrame {
    pub(crate) cursor: Point,
    pub(crate) content_height: NonZeroU16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PromptRenderError {
    ZeroWidth,
    Layout(LayoutError),
    InsufficientHeight {
        required: NonZeroU16,
        available: u16,
    },
    SurfaceConflict,
}

pub(crate) fn render(
    editor: &PromptEditor,
    view: &mut SurfaceView<'_>,
    style: Style,
) -> Result<PromptFrame, PromptRenderError> {
    let width = NonZeroU16::new(view.size().width).ok_or(PromptRenderError::ZeroWidth)?;
    let layout = editor.layout(width).map_err(PromptRenderError::Layout)?;
    validate_height(&layout, view.size().height)?;

    if view.clear(style) == WriteOutcome::Clipped {
        return Err(PromptRenderError::SurfaceConflict);
    }

    for positioned in layout.glyphs {
        if view.write(positioned.point, positioned.grapheme, style) == WriteOutcome::Clipped {
            unreachable!("validated layout must fit the cleared prompt view");
        }
    }

    Ok(PromptFrame {
        cursor: layout.cursor,
        content_height: layout.height,
    })
}

fn validate_height(layout: &TextLayout, available: u16) -> Result<(), PromptRenderError> {
    if layout.height.get() > available {
        Err(PromptRenderError::InsufficientHeight {
            required: layout.height,
            available,
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
