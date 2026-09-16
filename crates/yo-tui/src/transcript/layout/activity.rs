//! Typed activity bodies and folding; retained content and plain source stay complete.

mod document;
mod media;
mod tool;
mod tool_files;
mod tool_resources;
mod tool_shell;

use serde_json::Value;
use yo_core::{
    ActivityDocument, ActivityKind, ActivityNotice, ActivityPlan, ActivityReasoning,
    ActivitySummary, NoticeLevel, PlanStepStatus, SummaryKind,
};

use super::{
    GlyphRole, NonZeroU16, PositionedTranscriptGrapheme, PreparedBody, TranscriptLayoutConfig,
    TranscriptMessage, TranscriptRenderError, flow_text, markdown,
};
use crate::text::flow::{TextFlow, TextFlowError, TextPages, flow_literal_prose};

pub(super) struct DocumentSource {
    pub(super) heading: String,
    pub(super) content: String,
    pub(super) tokens_before: Option<u64>,
    pub(super) reasoning: bool,
    pub(super) empty_hint: &'static str,
    pub(super) hidden_hint: &'static str,
}

impl DocumentSource {
    pub(super) fn parse(source: &str) -> Option<Self> {
        if let Some(reasoning) = ActivityReasoning::from_snapshot(source) {
            return Some(Self {
                heading: "Agent reasoning".to_owned(),
                content: match reasoning.content {
                    Value::String(text) => text,
                    content => format!("~~~json\n{content:#}\n~~~"),
                },
                tokens_before: None,
                reasoning: true,
                empty_hint: "No reasoning text was provided.",
                hidden_hint: "Reasoning hidden",
            });
        }
        let (heading, content, tokens_before, reasoning, empty_hint) =
            if let Some(document) = ActivityDocument::from_snapshot(source) {
                (
                    document.title,
                    document.markdown,
                    None,
                    false,
                    "No document text was provided.",
                )
            } else {
                let summary = ActivitySummary::from_snapshot(source)?;
                let heading = match summary.kind {
                    SummaryKind::Compaction => "Compaction summary",
                    SummaryKind::Branch => "Branch summary",
                    SummaryKind::Reasoning => "Public reasoning summary",
                }
                .to_owned();
                (
                    heading,
                    summary.summary,
                    summary.tokens_before,
                    summary.kind == SummaryKind::Reasoning,
                    "No summary text was provided.",
                )
            };
        Some(Self {
            heading,
            content,
            tokens_before,
            reasoning,
            empty_hint,
            hidden_hint: "Summary hidden",
        })
    }
}

pub(super) fn prepare_document(
    config: &TranscriptLayoutConfig,
    message: &TranscriptMessage,
    width: NonZeroU16,
    format_markdown: bool,
) -> Option<PreparedBody> {
    document::prepare_document(config, message, width, format_markdown)
}

pub(super) fn model_source_text(
    message: &TranscriptMessage,
    width: NonZeroU16,
) -> Result<Option<String>, TextFlowError> {
    document::model_source_text(message, width)
}

pub(super) fn prepare_tool(
    config: &TranscriptLayoutConfig,
    message: &TranscriptMessage,
    width: NonZeroU16,
    format_markdown: bool,
) -> Option<PreparedBody> {
    tool::prepare_tool(config, message, width, format_markdown)
}

pub(super) fn tool_presentation(
    config: &TranscriptLayoutConfig,
    message: &TranscriptMessage,
    width: NonZeroU16,
    format_markdown: bool,
) -> Option<(String, String, String, Option<String>, bool)> {
    tool::tool_presentation(config, message, width, format_markdown)
}

pub(super) fn tool_source_text(message: &TranscriptMessage) -> Option<String> {
    tool::tool_source_text(message)
}

pub(super) fn content_block_markdown(block: &Value, successful: bool) -> String {
    tool::content_block_markdown(block, successful)
}

pub(super) fn prepare_diff(
    message: &TranscriptMessage,
    width: NonZeroU16,
    code_padding: u16,
) -> Result<PreparedBody, TranscriptRenderError> {
    let activity = message.activity.expect("the caller selected a file change");
    let text = message.text();
    let header_end = text.find('\n').unwrap_or(text.len());
    let footer_start = activity.footer_start.unwrap_or(text.len());
    let source = &text[header_end.min(footer_start)..footer_start];
    let source = source.strip_prefix('\n').unwrap_or(source);
    let added = source
        .lines()
        .filter(|line| line.starts_with('+') && !line.starts_with("+++ "))
        .count();
    let removed = source
        .lines()
        .filter(|line| line.starts_with('-') && !line.starts_with("--- "))
        .count();
    let heading = if added + removed > 0 {
        format!("{} · +{added} -{removed}", &text[..header_end])
    } else {
        text[..header_end].to_owned()
    };
    let mut body = PreparedBody {
        skip_activity_folding: false,
        rasters: Vec::new(),
        glyphs: Vec::new(),
        row_styles: Vec::new(),
        height: 0,
    };
    append_plain(
        &mut body,
        &heading,
        width,
        GlyphRole::ActivityHeading(activity.outcome),
    )?;
    let conservative_width = NonZeroU16::new(width.get().saturating_sub(3).max(2).min(width.get()))
        .expect("diff width is nonzero");
    // Code layout omits one terminal newline; use the same source for preflight.
    let measured_source = source.strip_suffix('\n').unwrap_or(source);
    let source_rows = measured_source.lines().count();
    let rows = if source_rows > usize::from(u16::MAX) - 256 {
        source_rows
    } else {
        TextPages::new(measured_source, conservative_width)
            .map_err(TranscriptRenderError::Text)?
            .row_count()
    };
    if rows > usize::from(u16::MAX) - 256 {
        append_plain(
            &mut body,
            "Large diff · open /changes to read every retained row.",
            width,
            GlyphRole::ActivityBody,
        )?;
    } else if !source.is_empty() {
        let prepared = markdown::prepare_diff(source, width, code_padding)
            .map_err(TranscriptRenderError::Text)?;
        let offset = body.height;
        body.height = body
            .height
            .checked_add(prepared.height)
            .ok_or(TranscriptRenderError::HeightOverflow)?;
        body.glyphs
            .extend(prepared.glyphs.into_iter().map(|mut glyph| {
                glyph.point.y += offset;
                PositionedTranscriptGrapheme {
                    point: glyph.point,
                    grapheme: glyph.grapheme,
                    role: GlyphRole::ActivityBody,
                    decoration: glyph.decoration,
                    hyperlink: glyph.hyperlink,
                }
            }));
        body.row_styles.extend(
            prepared
                .row_styles
                .into_iter()
                .map(|(row, style)| (row + offset, style)),
        );
    }
    if footer_start < text.len() {
        append_plain(
            &mut body,
            text[footer_start..].trim_start_matches('\n'),
            width,
            GlyphRole::ActivityOutcome(activity.outcome),
        )?;
    }
    Ok(body)
}

pub(super) fn prepare_interaction(
    message: &TranscriptMessage,
    width: NonZeroU16,
) -> Option<PreparedBody> {
    let activity = message.activity?;
    if !matches!(
        activity.kind,
        Some(
            ActivityKind::UserInputResponse { .. }
                | ActivityKind::ApprovalResponse { .. }
                | ActivityKind::UserInputRequest { .. }
                | ActivityKind::ApprovalRequest { .. }
        )
    ) {
        return None;
    }
    let text = message.text();
    let end = activity.footer_start.unwrap_or(text.len());
    let (heading, source) = text[..end].split_once('\n').unwrap_or((&text[..end], ""));
    let mut body = PreparedBody {
        skip_activity_folding: true,
        rasters: Vec::new(),
        glyphs: Vec::new(),
        row_styles: Vec::new(),
        height: 0,
    };
    append_prose(
        &mut body,
        heading,
        width,
        GlyphRole::ActivityHeading(activity.outcome),
    )
    .ok()?;
    if !source.is_empty() {
        append_prose(&mut body, source, width, GlyphRole::ActivityBody).ok()?;
    }
    if end < text.len() {
        append_prose(
            &mut body,
            text[end..].trim_start_matches('\n'),
            width,
            GlyphRole::ActivityOutcome(activity.outcome),
        )
        .ok()?;
    }
    Some(body)
}

pub(super) fn prepare_notice(
    message: &TranscriptMessage,
    width: NonZeroU16,
) -> Option<PreparedBody> {
    let activity = message.activity?;
    if activity.kind != Some(ActivityKind::ModelWork) {
        return None;
    }
    let end = activity.footer_start.unwrap_or(message.text().len());
    let (_, source) = message.text()[..end].split_once('\n')?;
    let notice = ActivityNotice::from_snapshot(source)?;
    let mut body = PreparedBody {
        skip_activity_folding: false,
        rasters: Vec::new(),
        glyphs: Vec::new(),
        row_styles: Vec::new(),
        height: 0,
    };
    append_prose(
        &mut body,
        &notice.title,
        width,
        GlyphRole::NoticeHeading(notice.level),
    )
    .ok()?;
    append_prose(&mut body, &notice.message, width, GlyphRole::ActivityBody).ok()?;
    if end < message.text().len() {
        append_prose(
            &mut body,
            message.text()[end..].trim_start_matches('\n'),
            width,
            GlyphRole::ActivityOutcome(activity.outcome),
        )
        .ok()?;
    }
    Some(body)
}

pub(super) fn plan_heading(plan: &ActivityPlan) -> String {
    let completed = plan
        .steps
        .iter()
        .filter(|step| step.status == PlanStepStatus::Completed)
        .count();
    if plan.steps.is_empty() {
        "Plan".to_owned()
    } else {
        format!("Plan · {completed}/{} completed", plan.steps.len())
    }
}

pub(super) fn plan_marker(status: PlanStepStatus) -> &'static str {
    match status {
        PlanStepStatus::Completed => "[x] ",
        PlanStepStatus::InProgress => "[>] ",
        PlanStepStatus::Pending => "[ ] ",
    }
}

pub(super) fn prepare_plan(message: &TranscriptMessage, width: NonZeroU16) -> Option<PreparedBody> {
    let activity = message.activity?;
    if activity.kind != Some(ActivityKind::ModelWork) {
        return None;
    }
    let end = activity.footer_start.unwrap_or(message.text().len());
    let (_, source) = message.text()[..end].split_once('\n')?;
    let plan = ActivityPlan::from_snapshot(source)?;
    let mut body = PreparedBody {
        skip_activity_folding: false,
        rasters: Vec::new(),
        glyphs: Vec::new(),
        row_styles: Vec::new(),
        height: 0,
    };
    append_plain(
        &mut body,
        &plan_heading(&plan),
        width,
        GlyphRole::NoticeHeading(NoticeLevel::Info),
    )
    .ok()?;
    if let Some(explanation) = plan.explanation.filter(|text| !text.is_empty()) {
        append_plain(&mut body, &explanation, width, GlyphRole::ActivityBody).ok()?;
    }
    if plan.steps.is_empty() {
        append_plain(
            &mut body,
            "No steps provided.",
            width,
            GlyphRole::ActivityBody,
        )
        .ok()?;
    }
    for step in plan.steps {
        let marker = plan_marker(step.status);
        let role = GlyphRole::PlanStep(step.status);
        if let Some(inner) = width
            .get()
            .checked_sub(4)
            .and_then(NonZeroU16::new)
            .filter(|n| n.get() >= 2)
        {
            let flow = flow_text(&step.text, inner).ok()?;
            let offset = body.height;
            append_plain(&mut body, marker.trim_end(), width, role).ok()?;
            body.height = offset.checked_add(flow.height.max(1))?;
            body.glyphs.extend(flow.glyphs.into_iter().map(|mut glyph| {
                glyph.point.x += 4;
                glyph.point.y += offset;
                PositionedTranscriptGrapheme {
                    point: glyph.point,
                    grapheme: glyph.grapheme,
                    role,
                    decoration: markdown::Decoration::default(),
                    hyperlink: None,
                }
            }));
        } else {
            append_plain(&mut body, &format!("{marker}{}", step.text), width, role).ok()?;
        }
    }
    if end < message.text().len() {
        append_plain(
            &mut body,
            message.text()[end..].trim_start_matches('\n'),
            width,
            GlyphRole::ActivityOutcome(activity.outcome),
        )
        .ok()?;
    }
    Some(body)
}

fn append_plain(
    body: &mut PreparedBody,
    text: &str,
    width: NonZeroU16,
    role: GlyphRole,
) -> Result<(), TranscriptRenderError> {
    let flow = flow_text(text, width).map_err(TranscriptRenderError::Text)?;
    append_flow(body, flow, role)
}

fn append_prose(
    body: &mut PreparedBody,
    text: &str,
    width: NonZeroU16,
    role: GlyphRole,
) -> Result<(), TranscriptRenderError> {
    let flow = flow_literal_prose(text, width).map_err(TranscriptRenderError::Text)?;
    append_flow(body, flow, role)
}

fn append_flow(
    body: &mut PreparedBody,
    flow: TextFlow,
    role: GlyphRole,
) -> Result<(), TranscriptRenderError> {
    let offset = body.height;
    body.height = body
        .height
        .checked_add(flow.height)
        .ok_or(TranscriptRenderError::HeightOverflow)?;
    body.glyphs.extend(flow.glyphs.into_iter().map(|mut glyph| {
        glyph.point.y += offset;
        PositionedTranscriptGrapheme {
            point: glyph.point,
            grapheme: glyph.grapheme,
            role,
            decoration: markdown::Decoration::default(),
            hyperlink: None,
        }
    }));
    Ok(())
}

pub(super) fn compact(
    mut body: PreparedBody,
    width: NonZeroU16,
    opening_rows: u16,
) -> Result<PreparedBody, TranscriptRenderError> {
    let Some((start, body_role)) = body
        .glyphs
        .iter()
        .filter(|glyph| {
            matches!(
                glyph.role,
                GlyphRole::ActivityBody | GlyphRole::ReasoningBody
            )
        })
        .map(|glyph| (glyph.point.y, glyph.role))
        .min_by_key(|(row, _)| *row)
    else {
        return Ok(body);
    };
    let end = body
        .glyphs
        .iter()
        .filter(|glyph| matches!(glyph.role, GlyphRole::ActivityOutcome(_)))
        .map(|glyph| glyph.point.y)
        .min()
        .unwrap_or(body.height);
    if end.saturating_sub(start) <= opening_rows.saturating_add(6) {
        return Ok(body);
    }
    let hidden_start = start + opening_rows;
    let hidden_end = end - 3;
    let hidden = hidden_end - hidden_start;
    let hint = flow_text(&format!("... {hidden} rows hidden · Ctrl+O expand"), width)
        .map_err(TranscriptRenderError::Text)?;
    if hint.height >= hidden {
        return Ok(body);
    }
    let removed = hidden - hint.height;
    body.glyphs
        .retain(|glyph| glyph.point.y < hidden_start || glyph.point.y >= hidden_end);
    for glyph in &mut body.glyphs {
        if glyph.point.y >= hidden_end {
            glyph.point.y -= removed;
        }
    }
    body.row_styles
        .retain(|(row, _)| *row < hidden_start || *row >= hidden_end);
    for (row, _) in &mut body.row_styles {
        if *row >= hidden_end {
            *row -= removed;
        }
    }
    body.glyphs.extend(hint.glyphs.into_iter().map(|mut glyph| {
        glyph.point.y += hidden_start;
        PositionedTranscriptGrapheme {
            point: glyph.point,
            grapheme: glyph.grapheme,
            role: body_role,
            decoration: markdown::Decoration::default(),
            hyperlink: None,
        }
    }));
    // A native image cannot span a removed region. Retain its cell fallback,
    // and shift only complete images that remain below the folded rows.
    body.rasters.retain_mut(|raster| {
        let top = raster.area.origin.y;
        let bottom = top + raster.area.size.height;
        if top < hidden_end && bottom > hidden_start {
            return false;
        }
        if top >= hidden_end {
            raster.area.origin.y -= removed;
        }
        true
    });
    body.height -= removed;
    Ok(body)
}
