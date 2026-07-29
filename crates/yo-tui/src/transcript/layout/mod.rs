//! Width-dependent transcript projection into a bounded Surface view.

mod config;
mod output;

use std::num::NonZeroU16;

pub(crate) use config::{TranscriptLayoutConfig, TranscriptLayoutConfigError};

use super::{
    MessageRole, TranscriptBody, TranscriptPhase, TranscriptState,
    viewport::{TranscriptScrollCommand, TranscriptViewState, VisibleRows},
};
use crate::{
    surface::{Grapheme, Point, Style, SurfaceView, WriteOutcome},
    text::flow::{TextFlowError, flow_text},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TranscriptStyles {
    pub(crate) background: Style,
    pub(crate) user_marker: Style,
    pub(crate) user_body: Style,
    pub(crate) assistant_marker: Style,
    pub(crate) assistant_body: Style,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TranscriptRenderFrame {
    pub(crate) content_height: u16,
    pub(crate) first_visible_row: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TranscriptMeasure {
    pub(crate) content_height: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreparedTranscript {
    layout: TranscriptLayout,
    width: NonZeroU16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TranscriptMeasureError {
    ZeroWidth,
    InvalidConfig(TranscriptLayoutConfigError),
    BodyWidthUnavailable,
    Text(TextFlowError),
    HeightOverflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TranscriptRenderError {
    ZeroWidth,
    ZeroHeight,
    InvalidConfig(TranscriptLayoutConfigError),
    BodyWidthUnavailable,
    Text(TextFlowError),
    HeightOverflow,
    SurfaceConflict,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TranscriptPaintError {
    WidthMismatch { prepared: u16, actual: u16 },
    ZeroHeight,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PositionedTranscriptGrapheme {
    point: Point,
    grapheme: Grapheme,
    role: GlyphRole,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GlyphRole {
    UserMarker,
    UserBody,
    AssistantMarker,
    AssistantBody,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TranscriptLayout {
    glyphs: Vec<PositionedTranscriptGrapheme>,
    height: u16,
}

pub(crate) fn measure(
    transcript: &TranscriptState,
    width: u16,
    config: &TranscriptLayoutConfig,
) -> Result<TranscriptMeasure, TranscriptMeasureError> {
    let prepared = prepare(transcript, width, config)?;
    Ok(TranscriptMeasure {
        content_height: prepared.content_height(),
    })
}

pub(crate) fn prepare(
    transcript: &TranscriptState,
    width: u16,
    config: &TranscriptLayoutConfig,
) -> Result<PreparedTranscript, TranscriptMeasureError> {
    let width = NonZeroU16::new(width).ok_or(TranscriptMeasureError::ZeroWidth)?;
    config
        .validate_for_width(width.get())
        .map_err(TranscriptMeasureError::InvalidConfig)?;
    let layout = layout(transcript, width, config).map_err(measure_error)?;
    Ok(PreparedTranscript { layout, width })
}

pub(crate) fn render(
    transcript: &TranscriptState,
    view: &mut SurfaceView<'_>,
    config: &TranscriptLayoutConfig,
    styles: TranscriptStyles,
    state: &mut TranscriptViewState,
    command: Option<TranscriptScrollCommand>,
) -> Result<TranscriptRenderFrame, TranscriptRenderError> {
    let size = view.size();
    let width = NonZeroU16::new(size.width).ok_or(TranscriptRenderError::ZeroWidth)?;
    NonZeroU16::new(size.height).ok_or(TranscriptRenderError::ZeroHeight)?;
    let prepared = prepare(transcript, width.get(), config).map_err(render_error)?;

    if view.clear(styles.background) == WriteOutcome::Clipped {
        return Err(TranscriptRenderError::SurfaceConflict);
    }

    paint_prepared(prepared, view, styles, state, command).map_err(|error| match error {
        TranscriptPaintError::WidthMismatch { .. } => {
            unreachable!("transcript render prepares against the target view width")
        },
        TranscriptPaintError::ZeroHeight => {
            unreachable!("the transcript view height was checked before painting")
        },
    })
}

pub(crate) fn paint_prepared(
    prepared: PreparedTranscript,
    view: &mut SurfaceView<'_>,
    styles: TranscriptStyles,
    state: &mut TranscriptViewState,
    command: Option<TranscriptScrollCommand>,
) -> Result<TranscriptRenderFrame, TranscriptPaintError> {
    if view.size().width != prepared.width.get() {
        return Err(TranscriptPaintError::WidthMismatch {
            prepared: prepared.width.get(),
            actual: view.size().width,
        });
    }
    let height = NonZeroU16::new(view.size().height).ok_or(TranscriptPaintError::ZeroHeight)?;
    let visible = VisibleRows::resolve(prepared.layout.height, height, *state, command);

    for positioned in prepared
        .layout
        .glyphs
        .into_iter()
        .filter(|positioned| visible.contains(positioned.point.y))
    {
        let point = visible.translate(positioned.point);
        let style = styles.for_role(positioned.role);
        if view.write(point, positioned.grapheme, style) == WriteOutcome::Clipped {
            unreachable!("validated transcript layout must fit its cleared view");
        }
    }

    *state = visible.next_state();
    Ok(TranscriptRenderFrame {
        content_height: prepared.layout.height,
        first_visible_row: visible.first(),
    })
}

fn measure_error(error: TranscriptRenderError) -> TranscriptMeasureError {
    match error {
        TranscriptRenderError::InvalidConfig(error) => TranscriptMeasureError::InvalidConfig(error),
        TranscriptRenderError::BodyWidthUnavailable => TranscriptMeasureError::BodyWidthUnavailable,
        TranscriptRenderError::Text(error) => TranscriptMeasureError::Text(error),
        TranscriptRenderError::HeightOverflow => TranscriptMeasureError::HeightOverflow,
        TranscriptRenderError::ZeroWidth
        | TranscriptRenderError::ZeroHeight
        | TranscriptRenderError::SurfaceConflict => {
            unreachable!("pure transcript layout does not return view or Surface failures")
        },
    }
}

fn render_error(error: TranscriptMeasureError) -> TranscriptRenderError {
    match error {
        TranscriptMeasureError::ZeroWidth => TranscriptRenderError::ZeroWidth,
        TranscriptMeasureError::InvalidConfig(error) => TranscriptRenderError::InvalidConfig(error),
        TranscriptMeasureError::BodyWidthUnavailable => TranscriptRenderError::BodyWidthUnavailable,
        TranscriptMeasureError::Text(error) => TranscriptRenderError::Text(error),
        TranscriptMeasureError::HeightOverflow => TranscriptRenderError::HeightOverflow,
    }
}

impl PreparedTranscript {
    pub(crate) const fn content_height(&self) -> u16 {
        self.layout.height
    }
}

impl TranscriptStyles {
    const fn for_role(self, role: GlyphRole) -> Style {
        match role {
            GlyphRole::UserMarker => self.user_marker,
            GlyphRole::UserBody => self.user_body,
            GlyphRole::AssistantMarker => self.assistant_marker,
            GlyphRole::AssistantBody => self.assistant_body,
        }
    }
}

fn layout(
    transcript: &TranscriptState,
    view_width: NonZeroU16,
    config: &TranscriptLayoutConfig,
) -> Result<TranscriptLayout, TranscriptRenderError> {
    let available_body_width = view_width
        .get()
        .checked_sub(config.body_indent())
        .and_then(NonZeroU16::new);
    let mut glyphs = Vec::new();
    let mut height = 0_u16;
    let mut has_visible_item = false;

    for item in transcript.items() {
        let TranscriptBody::Message(message) = item.body();
        if message.text().is_empty() && item.phase() == TranscriptPhase::Streaming {
            continue;
        }
        let flow = if message.text().is_empty() {
            None
        } else {
            let body_width = configured_body_width(available_body_width, config.max_body_width())?;
            Some(flow_text(message.text(), body_width).map_err(TranscriptRenderError::Text)?)
        };
        let separator = if has_visible_item {
            separator_height(message.role())
        } else {
            0
        };
        let item_y = height
            .checked_add(separator)
            .ok_or(TranscriptRenderError::HeightOverflow)?;
        let role = message.role();

        glyphs.extend(marker_glyphs(config.marker(role), item_y, role)?);
        if let Some(flow) = flow {
            let flow_height = flow.height;
            for positioned in flow.glyphs {
                let x = config
                    .body_indent()
                    .checked_add(positioned.point.x)
                    .ok_or(TranscriptRenderError::HeightOverflow)?;
                let y = item_y
                    .checked_add(positioned.point.y)
                    .ok_or(TranscriptRenderError::HeightOverflow)?;
                glyphs.push(PositionedTranscriptGrapheme {
                    point: Point::new(x, y),
                    grapheme: positioned.grapheme,
                    role: body_role(role),
                });
            }
            height = item_y
                .checked_add(flow_height)
                .ok_or(TranscriptRenderError::HeightOverflow)?;
        } else {
            height = item_y
                .checked_add(1)
                .ok_or(TranscriptRenderError::HeightOverflow)?;
        }
        has_visible_item = true;
    }

    Ok(TranscriptLayout { glyphs, height })
}

fn configured_body_width(
    available: Option<NonZeroU16>,
    maximum: Option<NonZeroU16>,
) -> Result<NonZeroU16, TranscriptRenderError> {
    let available = available.ok_or(TranscriptRenderError::BodyWidthUnavailable)?;
    Ok(maximum.map_or(available, |maximum| available.min(maximum)))
}

fn marker_glyphs(
    marker: &str,
    y: u16,
    role: MessageRole,
) -> Result<Vec<PositionedTranscriptGrapheme>, TranscriptRenderError> {
    let mut glyphs = Vec::new();
    let mut x = 0_u16;
    for text in unicode_segmentation::UnicodeSegmentation::graphemes(marker, true) {
        let grapheme = Grapheme::try_from(text).map_err(|cause| {
            TranscriptRenderError::InvalidConfig(TranscriptLayoutConfigError::UnrenderableMarker {
                role,
                cause,
            })
        })?;
        let width = grapheme.width().get();
        glyphs.push(PositionedTranscriptGrapheme {
            point: Point::new(x, y),
            role: marker_role(role),
            grapheme,
        });
        x = x
            .checked_add(width)
            .ok_or(TranscriptRenderError::InvalidConfig(
                TranscriptLayoutConfigError::MarkerWidthOverflow { role },
            ))?;
    }
    Ok(glyphs)
}

const fn separator_height(role: MessageRole) -> u16 {
    match role {
        MessageRole::User => 2,
        MessageRole::Assistant => 1,
    }
}

const fn marker_role(role: MessageRole) -> GlyphRole {
    match role {
        MessageRole::User => GlyphRole::UserMarker,
        MessageRole::Assistant => GlyphRole::AssistantMarker,
    }
}

const fn body_role(role: MessageRole) -> GlyphRole {
    match role {
        MessageRole::User => GlyphRole::UserBody,
        MessageRole::Assistant => GlyphRole::AssistantBody,
    }
}

#[cfg(test)]
mod tests;
