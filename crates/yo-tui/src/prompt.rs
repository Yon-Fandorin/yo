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

pub(crate) mod assist;
mod chrome;
pub(crate) mod image;
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
    empty: bool,
    placeholder: Option<&'static str>,
    image_thumbnail: Option<image::ImageThumbnail>,
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
    Ok(PreparedPrompt {
        layout,
        chrome,
        empty: editor.text().is_empty(),
        placeholder: None,
        image_thumbnail: None,
    })
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
    let mut viewport = prepared.chrome.viewport(height);
    if let Some(thumbnail) = prepared.image_thumbnail.as_ref() {
        let (width, height) = thumbnail_cells(thumbnail, viewport.content_size.width);
        if viewport.content_size.height > height + 1 && width > 0 {
            let area = Rect::new(
                viewport.content_origin,
                crate::surface::Size::new(width, height),
            );
            view.place_raster(crate::surface::RasterImage {
                area,
                png: thumbnail.png.clone(),
            });
            for (column, character) in "Image preview"
                .chars()
                .take(usize::from(viewport.content_size.width))
                .enumerate()
            {
                let _ = view.write(
                    Point::new(
                        viewport.content_origin.x + column as u16,
                        viewport.content_origin.y + height,
                    ),
                    crate::surface::Grapheme::try_from(character.to_string().as_str())
                        .expect("preview label is ASCII"),
                    styles.rule,
                );
            }
            viewport.content_origin.y += height + 1;
            viewport.content_size.height -= height + 1;
        }
    }
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
    prepared.chrome.paint(
        view,
        viewport,
        styles.glyphs,
        styles,
        visible.first(),
        prepared.layout.height.get(),
    );

    // The hint is painted after measurement: it never becomes input, changes
    // cursor mapping, or creates a wrapped row in a narrow terminal.
    if prepared.empty && viewport.content_size.width >= 30 {
        let hint = if let Some(placeholder) = prepared.placeholder {
            placeholder
        } else if viewport.content_size.width >= 33 {
            "Ask anything, or describe a change"
        } else {
            "Describe a change..."
        };
        for (column, text) in hint.split("").filter(|text| !text.is_empty()).enumerate() {
            if column >= usize::from(viewport.content_size.width) {
                break;
            }
            let _ = view.write(
                Point::new(
                    viewport.content_origin.x + column as u16,
                    viewport.content_origin.y,
                ),
                crate::surface::Grapheme::try_from(text).expect("the hint is printable ASCII"),
                styles.rule,
            );
        }
    }

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
    pub(crate) fn with_image_thumbnail(
        mut self,
        thumbnail: Option<&image::ImageThumbnail>,
    ) -> Self {
        self.image_thumbnail = thumbnail.cloned();
        self
    }
    pub(crate) fn with_placeholder(mut self, placeholder: &'static str) -> Self {
        self.placeholder = Some(placeholder);
        self
    }

    pub(crate) fn desired_height(&self) -> NonZeroU16 {
        let preview_rows = self.image_thumbnail.as_ref().map_or(0, |thumbnail| {
            thumbnail_cells(thumbnail, self.chrome.content_width().get()).1 + 1
        });
        self.chrome.desired_height(
            NonZeroU16::new(self.layout.height.get().saturating_add(preview_rows)).unwrap(),
        )
    }

    pub(crate) const fn with_frame(mut self, enabled: bool) -> Self {
        self.chrome = self.chrome.with_frame(enabled);
        self
    }
}

fn thumbnail_cells(thumbnail: &image::ImageThumbnail, available: u16) -> (u16, u16) {
    let width = available.min(16).min(thumbnail.width as u16).max(1);
    let height =
        ((u64::from(thumbnail.height) * u64::from(width)) / u64::from(thumbnail.width).max(1) / 2)
            .clamp(1, 6) as u16;
    (width, height)
}

pub(crate) mod skill_reference;
#[cfg(test)]
mod tests;
pub(crate) mod workspace_reference;
