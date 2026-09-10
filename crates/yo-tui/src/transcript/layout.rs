//! Width-dependent transcript projection into a bounded Surface view.

mod activity;
mod config;
mod markdown;
use markdown::Decoration;
pub(crate) use markdown::MarkdownStyles;

use crate::surface::RasterImage;
mod output;
use std::num::NonZeroU16;

pub(crate) use config::{TranscriptLayoutConfig, TranscriptLayoutConfigError};
pub(super) use output::plain_output;
use yo_core::{ActivityKind, MessageContent, NoticeLevel, PlanStepStatus};

use super::{
    AssistantRenderInput, MessageRole, TranscriptActivityOutcome, TranscriptBody, TranscriptItemId,
    TranscriptMessage, TranscriptPhase, TranscriptSlice, TranscriptState,
    viewport::{TranscriptScrollCommand, TranscriptViewState, VisibleRows},
};
use crate::{
    surface::{
        Attributes, Color, Grapheme, Hyperlink, Point, Rect, Size, Style, SurfaceView, WriteOutcome,
    },
    text::flow::{TextFlowError, flow_literal_prose, flow_text},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TranscriptStyles {
    pub(crate) background: Style,
    pub(crate) user_marker: Style,
    pub(crate) user_body: Style,
    pub(crate) assistant_marker: Style,
    pub(crate) assistant_body: Style,
    pub(crate) markdown: MarkdownStyles,
    pub(crate) activity: TranscriptActivityStyles,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TranscriptActivityStyles {
    pub(crate) heading: Style,
    pub(crate) body: Style,
    pub(crate) reasoning: Style,
    pub(crate) success: Style,
    pub(crate) warning: Style,
    pub(crate) error: Style,
}

impl TranscriptActivityStyles {
    #[cfg(test)]
    pub(crate) const fn plain(style: Style) -> Self {
        Self {
            heading: style,
            body: style,
            reasoning: style,
            success: style,
            warning: style,
            error: style,
        }
    }

    pub(crate) const fn status(self, outcome: Option<TranscriptActivityOutcome>) -> Style {
        match outcome {
            None => self.heading,
            Some(TranscriptActivityOutcome::Completed) => self.success,
            Some(TranscriptActivityOutcome::Interrupted) => self.warning,
            Some(TranscriptActivityOutcome::Failed) => self.error,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TranscriptRenderFrame {
    pub(crate) content_height: usize,
    pub(crate) first_visible_row: usize,
    pub(crate) context_item: Option<TranscriptItemId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TranscriptMeasure {
    pub(crate) content_height: usize,
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
    decoration: Decoration,
    hyperlink: Option<Hyperlink>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GlyphRole {
    UserMarker,
    UserBody,
    AssistantMarker,
    AssistantBody,
    ActivityHeading(Option<TranscriptActivityOutcome>),
    NoticeHeading(NoticeLevel),
    PlanStep(PlanStepStatus),
    ActivityBody,
    ReasoningBody,
    ActivityOutcome(Option<TranscriptActivityOutcome>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TranscriptLayout {
    // Document rows remain separate from each bounded item's cell geometry.
    rasters: Vec<(usize, RasterImage)>,
    row_bands: Vec<(usize, u16, u16, Decoration)>,
    glyphs: Vec<(usize, PositionedTranscriptGrapheme)>,
    items: Vec<PositionedTranscriptItem>,
    height: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PositionedTranscriptItem {
    id: TranscriptItemId,
    role: MessageRole,
    first_row: usize,
    end_row: usize,
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

pub(crate) fn measure_slice(
    transcript: TranscriptSlice<'_>,
    width: u16,
    config: &TranscriptLayoutConfig,
) -> Result<TranscriptMeasure, TranscriptMeasureError> {
    let prepared = prepare_slice(transcript, width, config)?;
    Ok(TranscriptMeasure {
        content_height: prepared.content_height(),
    })
}

pub(crate) fn prepare(
    transcript: &TranscriptState,
    width: u16,
    config: &TranscriptLayoutConfig,
) -> Result<PreparedTranscript, TranscriptMeasureError> {
    prepare_slice(transcript.all(), width, config)
}

pub(crate) fn prepare_slice(
    transcript: TranscriptSlice<'_>,
    width: u16,
    config: &TranscriptLayoutConfig,
) -> Result<PreparedTranscript, TranscriptMeasureError> {
    prepare_with_format(transcript, width, config, true)
}

fn prepare_with_format(
    transcript: TranscriptSlice<'_>,
    width: u16,
    config: &TranscriptLayoutConfig,
    format_markdown: bool,
) -> Result<PreparedTranscript, TranscriptMeasureError> {
    let width = NonZeroU16::new(width).ok_or(TranscriptMeasureError::ZeroWidth)?;
    config
        .validate_for_width(width.get())
        .map_err(TranscriptMeasureError::InvalidConfig)?;
    let layout = layout(transcript, width, config, format_markdown).map_err(measure_error)?;
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
    render_slice(transcript.all(), view, config, styles, state, command)
}

pub(crate) fn render_slice(
    transcript: TranscriptSlice<'_>,
    view: &mut SurfaceView<'_>,
    config: &TranscriptLayoutConfig,
    styles: TranscriptStyles,
    state: &mut TranscriptViewState,
    command: Option<TranscriptScrollCommand>,
) -> Result<TranscriptRenderFrame, TranscriptRenderError> {
    render_slice_commands(transcript, view, config, styles, state, command.as_slice())
}

pub(crate) fn render_commands(
    transcript: &TranscriptState,
    view: &mut SurfaceView<'_>,
    config: &TranscriptLayoutConfig,
    styles: TranscriptStyles,
    state: &mut TranscriptViewState,
    commands: &[TranscriptScrollCommand],
) -> Result<TranscriptRenderFrame, TranscriptRenderError> {
    render_slice_commands(transcript.all(), view, config, styles, state, commands)
}

fn render_slice_commands(
    transcript: TranscriptSlice<'_>,
    view: &mut SurfaceView<'_>,
    config: &TranscriptLayoutConfig,
    styles: TranscriptStyles,
    state: &mut TranscriptViewState,
    commands: &[TranscriptScrollCommand],
) -> Result<TranscriptRenderFrame, TranscriptRenderError> {
    let size = view.size();
    let width = NonZeroU16::new(size.width).ok_or(TranscriptRenderError::ZeroWidth)?;
    NonZeroU16::new(size.height).ok_or(TranscriptRenderError::ZeroHeight)?;
    let prepared = prepare_slice(transcript, width.get(), config).map_err(render_error)?;

    if view.clear(styles.background) == WriteOutcome::Clipped {
        return Err(TranscriptRenderError::SurfaceConflict);
    }

    paint_prepared_commands(prepared, view, styles, state, commands).map_err(|error| match error {
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
    paint_prepared_commands(prepared, view, styles, state, command.as_slice())
}

pub(crate) fn paint_prepared_commands(
    prepared: PreparedTranscript,
    view: &mut SurfaceView<'_>,
    styles: TranscriptStyles,
    state: &mut TranscriptViewState,
    commands: &[TranscriptScrollCommand],
) -> Result<TranscriptRenderFrame, TranscriptPaintError> {
    if view.size().width != prepared.width.get() {
        return Err(TranscriptPaintError::WidthMismatch {
            prepared: prepared.width.get(),
            actual: view.size().width,
        });
    }
    let height = NonZeroU16::new(view.size().height).ok_or(TranscriptPaintError::ZeroHeight)?;
    let item_starts = if commands.iter().any(|command| {
        matches!(
            command,
            TranscriptScrollCommand::PreviousItem | TranscriptScrollCommand::NextItem
        )
    }) {
        prepared
            .layout
            .items
            .iter()
            .map(|item| item.first_row)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let visible = VisibleRows::resolve_commands(
        prepared.layout.height,
        height,
        *state,
        commands,
        &item_starts,
    );
    let context_item = prepared.layout.context_item(visible);

    if styles.user_body.background != Color::Default {
        for item in &prepared.layout.items {
            if item.role != MessageRole::User {
                continue;
            }
            for row in item.first_row.max(visible.first())..item.end_row.min(visible.end()) {
                let y = visible.translate(0, row).y;
                let width = view.size().width;
                let mut band = view
                    .subview(Rect::new(Point::new(0, y), Size::new(width, 1)))
                    .expect("only visible transcript rows receive a request band");
                let _ = band.clear(styles.user_body);
            }
        }
    }

    let mut row_backgrounds = vec![None; usize::from(view.size().height)];
    for &(row, indent, width, decoration) in &prepared.layout.row_bands {
        if visible.contains(row) {
            let y = visible.translate(0, row).y;
            let style = styles.markdown.resolve(decoration, styles.assistant_body);
            let mut band = view
                .subview(Rect::new(Point::new(indent, y), Size::new(width, 1)))
                .expect("prepared semantic code row fits the transcript");
            let _ = band.clear(style);
            row_backgrounds[usize::from(y)] = Some((indent, indent + width, style.background));
        }
    }

    let rasters = prepared.layout.rasters;
    for (row, positioned) in prepared
        .layout
        .glyphs
        .into_iter()
        .filter(|(row, _)| visible.contains(*row))
    {
        let point = visible.translate(positioned.point.x, row);
        let mut style = styles
            .markdown
            .resolve(positioned.decoration, styles.for_role(positioned.role));
        if let Some((start, end, background)) = row_backgrounds[usize::from(point.y)]
            && (start..end).contains(&point.x)
        {
            style.background = background;
        }
        let grapheme = styles
            .markdown
            .display_glyph(positioned.decoration, positioned.grapheme);
        if view.write_linked(point, grapheme, style, positioned.hyperlink) == WriteOutcome::Clipped
        {
            unreachable!("validated transcript layout must fit its cleared view");
        }
    }

    *state = visible.next_state();
    if styles.markdown.rich_media && styles.markdown.pixel_color_capability != Color::Default {
        for (row, mut raster) in rasters {
            let end = row.checked_add(usize::from(raster.area.size.height));
            if visible.contains(row) && end.is_some_and(|end| end <= visible.end()) {
                raster.area.origin = visible.translate(raster.area.origin.x, row);
                view.place_raster(raster);
            }
        }
    }

    Ok(TranscriptRenderFrame {
        content_height: prepared.layout.height,
        first_visible_row: visible.first(),
        context_item,
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
    pub(crate) const fn content_height(&self) -> usize {
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
            GlyphRole::ActivityHeading(outcome) => {
                let mut style = self.activity.status(outcome);
                style.attributes = style.attributes.union(Attributes::BOLD);
                style
            },
            GlyphRole::NoticeHeading(level) => {
                let mut style = match level {
                    NoticeLevel::Info => self.activity.heading,
                    NoticeLevel::Warning => self.activity.warning,
                };
                style.attributes = style.attributes.union(Attributes::BOLD);
                style
            },
            GlyphRole::PlanStep(status) => match status {
                PlanStepStatus::Completed => self.activity.success,
                PlanStepStatus::Pending => self.activity.body,
                PlanStepStatus::InProgress => {
                    let mut style = self.activity.heading;
                    style.attributes = style.attributes.union(Attributes::BOLD);
                    style
                },
            },
            GlyphRole::ActivityBody => self.activity.body,
            GlyphRole::ReasoningBody => self.activity.reasoning,
            GlyphRole::ActivityOutcome(outcome) => self.activity.status(outcome),
        }
    }
}

struct PreparedBody {
    skip_activity_folding: bool,
    rasters: Vec<RasterImage>,
    glyphs: Vec<PositionedTranscriptGrapheme>,
    row_styles: Vec<(u16, Decoration)>,
    height: u16,
}

fn prepare_body(
    config: &TranscriptLayoutConfig,
    message: &TranscriptMessage,
    width: NonZeroU16,
    format_markdown: bool,
    finalized: bool,
) -> Result<PreparedBody, TranscriptRenderError> {
    if format_markdown && message.activity.is_some_and(|activity| activity.diff) {
        return activity::prepare_diff(message, width, config.code_padding);
    }
    if let Some(prepared) = activity::prepare_plan(message, width) {
        return Ok(prepared);
    }
    if format_markdown
        && config.compact_activities()
        && message.activity.is_some_and(|activity| activity.usage)
    {
        let summary = format!(
            "Usage · {}",
            message.text().lines().nth(1).unwrap_or("No values")
        );
        let flow = flow_text(&summary, width).map_err(TranscriptRenderError::Text)?;
        return Ok(PreparedBody {
            skip_activity_folding: true,
            rasters: Vec::new(),
            row_styles: Vec::new(),
            height: flow.height,
            glyphs: flow
                .glyphs
                .into_iter()
                .map(|glyph| PositionedTranscriptGrapheme {
                    point: glyph.point,
                    grapheme: glyph.grapheme,
                    role: GlyphRole::ActivityBody,
                    decoration: Decoration::default(),
                    hyperlink: None,
                })
                .collect(),
        });
    }
    if let Some(prepared) = activity::prepare_document(config, message, width, format_markdown) {
        return Ok(prepared);
    }
    if let Some(prepared) = activity::prepare_interaction(message, width) {
        return Ok(prepared);
    }
    if let Some(prepared) = activity::prepare_notice(message, width) {
        return Ok(prepared);
    }
    if let Some(prepared) = activity::prepare_tool(config, message, width, format_markdown) {
        return Ok(prepared);
    }
    if message.is_markdown() && format_markdown {
        let end = message
            .assistant_footer_start
            .unwrap_or(message.text().len());
        let source = &message.text()[..end];
        let replacement = config.assistant_renderer.as_ref().and_then(|renderer| {
            renderer.render(AssistantRenderInput {
                source,
                columns: width,
                finalized,
            })
        });
        let prepare = |text: &str| -> Result<PreparedBody, TranscriptRenderError> {
            let prepared = markdown::prepare_with_links(
                text,
                width,
                config.show_images,
                config.image_max_width,
                config.show_diagrams,
                config.active_link_resolver(),
                config.code_padding,
            )
            .map_err(TranscriptRenderError::Text)?;
            let mut body = PreparedBody {
                skip_activity_folding: false,
                rasters: prepared.rasters,
                glyphs: prepared
                    .glyphs
                    .into_iter()
                    .map(|glyph| PositionedTranscriptGrapheme {
                        point: glyph.point,
                        grapheme: glyph.grapheme,
                        role: GlyphRole::AssistantBody,
                        decoration: glyph.decoration,
                        hyperlink: glyph.hyperlink,
                    })
                    .collect(),
                row_styles: prepared.row_styles,
                height: prepared.height,
            };
            if let Some(start) = message.assistant_footer_start {
                let footer =
                    flow_literal_prose(message.text()[start..].trim_start_matches('\n'), width)
                        .map_err(TranscriptRenderError::Text)?;
                let offset = body
                    .height
                    .checked_add(u16::from(body.height > 0))
                    .ok_or(TranscriptRenderError::HeightOverflow)?;
                body.height = offset
                    .checked_add(footer.height)
                    .ok_or(TranscriptRenderError::HeightOverflow)?;
                body.glyphs.extend(footer.glyphs.into_iter().map(|glyph| {
                    PositionedTranscriptGrapheme {
                        point: Point::new(glyph.point.x, offset + glyph.point.y),
                        grapheme: glyph.grapheme,
                        role: GlyphRole::AssistantBody,
                        decoration: Decoration::default(),
                        hyperlink: None,
                    }
                }));
            }
            Ok(body)
        };
        let content = MessageContent::from_snapshot(source)
            .map(|content| activity::content_block_markdown(&content.block, true));
        let rendered_source = content.as_deref().unwrap_or(source);
        return match replacement {
            Some(replacement) => prepare(&replacement).or_else(|_| prepare(rendered_source)),
            None => prepare(rendered_source),
        };
    }

    let flow = flow_text(message.text(), width).map_err(TranscriptRenderError::Text)?;
    let header_end = message.text().find('\n').unwrap_or(message.text().len());
    let glyphs = flow
        .glyphs
        .into_iter()
        .map(|glyph| {
            let role = match message
                .activity
                .filter(|activity| activity.kind != Some(ActivityKind::ModelWork))
            {
                Some(activity) if glyph.byte_index < header_end => {
                    GlyphRole::ActivityHeading(activity.outcome)
                },
                Some(activity)
                    if activity
                        .footer_start
                        .is_some_and(|start| glyph.byte_index >= start) =>
                {
                    GlyphRole::ActivityOutcome(activity.outcome)
                },
                Some(_) => GlyphRole::ActivityBody,
                None => body_role(message.role()),
            };
            PositionedTranscriptGrapheme {
                point: glyph.point,
                grapheme: glyph.grapheme,
                role,
                decoration: Decoration::default(),
                hyperlink: None,
            }
        })
        .collect();
    Ok(PreparedBody {
        skip_activity_folding: false,
        rasters: Vec::new(),
        glyphs,
        row_styles: Vec::new(),
        height: flow.height,
    })
}

fn layout(
    transcript: TranscriptSlice<'_>,
    view_width: NonZeroU16,
    config: &TranscriptLayoutConfig,
    format_markdown: bool,
) -> Result<TranscriptLayout, TranscriptRenderError> {
    let available_body_width = view_width
        .get()
        .checked_sub(config.body_indent())
        .and_then(NonZeroU16::new);
    let mut glyphs = Vec::new();
    let mut row_bands = Vec::new();
    let mut rasters = Vec::new();
    let mut items = Vec::new();
    let mut height = 0_usize;
    let mut has_visible_item = transcript.has_visible_predecessor();

    for item in transcript.items() {
        let item_config = config.for_item(item.id());
        let config = item_config.as_ref().unwrap_or(config);
        let TranscriptBody::Message(message) = item.body();
        if message.text().is_empty() && item.phase() == TranscriptPhase::Streaming {
            continue;
        }
        let flow = if message.text().is_empty() {
            None
        } else {
            let body_width = configured_body_width(available_body_width, config.max_body_width())?;
            let body = prepare_body(
                config,
                message,
                body_width,
                format_markdown,
                item.phase() == TranscriptPhase::Final,
            )?;
            Some(
                if format_markdown
                    && config.compact_activities()
                    && !body.skip_activity_folding
                    && message
                        .activity
                        .is_some_and(|activity| activity.kind != Some(ActivityKind::ModelWork))
                {
                    activity::compact(
                        body,
                        body_width,
                        config.activity_head_rows(
                            message.activity.is_some_and(|activity| activity.diff),
                        ),
                    )?
                } else {
                    body
                },
            )
        };
        let separator = if has_visible_item {
            separator_height(message.role())
        } else {
            0
        };
        let item_y = height
            .checked_add(usize::from(separator))
            .ok_or(TranscriptRenderError::HeightOverflow)?;
        let role = message.role();

        let mut markers = marker_glyphs(config.marker(role), 0, role)?;
        if let Some(activity) = message
            .activity
            .filter(|activity| activity.kind != Some(ActivityKind::ModelWork))
        {
            for marker in &mut markers {
                marker.role = GlyphRole::ActivityOutcome(activity.outcome);
            }
        }
        glyphs.extend(markers.into_iter().map(|marker| (item_y, marker)));
        if let Some(flow) = flow {
            for mut raster in flow.rasters {
                raster.area.origin.x += config.body_indent();
                let row = item_y
                    .checked_add(usize::from(raster.area.origin.y))
                    .ok_or(TranscriptRenderError::HeightOverflow)?;
                rasters.push((row, raster));
            }
            let flow_height = flow.height.max(1);
            let body_width = configured_body_width(available_body_width, config.max_body_width())?;
            for (row, decoration) in flow.row_styles {
                row_bands.push((
                    item_y
                        .checked_add(usize::from(row))
                        .ok_or(TranscriptRenderError::HeightOverflow)?,
                    config.body_indent(),
                    body_width.get(),
                    decoration,
                ));
            }
            for positioned in flow.glyphs {
                let x = config
                    .body_indent()
                    .checked_add(positioned.point.x)
                    .ok_or(TranscriptRenderError::HeightOverflow)?;
                let y = item_y
                    .checked_add(usize::from(positioned.point.y))
                    .ok_or(TranscriptRenderError::HeightOverflow)?;
                glyphs.push((
                    y,
                    PositionedTranscriptGrapheme {
                        point: Point::new(x, positioned.point.y),
                        grapheme: positioned.grapheme,
                        role: positioned.role,
                        decoration: positioned.decoration,
                        hyperlink: positioned.hyperlink.filter(|_| config.hyperlinks),
                    },
                ));
            }
            height = item_y
                .checked_add(usize::from(flow_height))
                .ok_or(TranscriptRenderError::HeightOverflow)?;
        } else {
            height = item_y
                .checked_add(1)
                .ok_or(TranscriptRenderError::HeightOverflow)?;
        }
        items.push(PositionedTranscriptItem {
            id: item.id(),
            role,
            first_row: item_y,
            end_row: height,
        });
        has_visible_item = true;
    }

    Ok(TranscriptLayout {
        row_bands,
        rasters,
        glyphs,
        items,
        height,
    })
}

impl TranscriptLayout {
    fn context_item(&self, visible: VisibleRows) -> Option<TranscriptItemId> {
        let mut visible_items = self
            .items
            .iter()
            .filter(|item| item.first_row < visible.end() && item.end_row > visible.first());
        if visible.follows_tail() {
            visible_items.next_back().map(|item| item.id)
        } else {
            visible_items.map(|item| item.id).next()
        }
    }
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
            decoration: Decoration::default(),
            hyperlink: None,
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
