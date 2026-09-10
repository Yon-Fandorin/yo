//! Typed activity bodies and folding; retained content and plain source stay complete.

use serde_json::{Map, Value, from_str};
use yo_core::{
    ActivityDocument, ActivityKind, ActivityNotice, ActivityPlan, ActivityReasoning,
    ActivitySummary, NoticeLevel, PlanStepStatus, SummaryKind, ToolOutput,
};

use super::{
    GlyphRole, NonZeroU16, PositionedTranscriptGrapheme, PreparedBody, TranscriptLayoutConfig,
    TranscriptMessage, TranscriptRenderError, flow_text, markdown,
};
use crate::{
    text::flow::{TextFlow, TextFlowError, TextPages, flow_literal_prose, flow_tail_text},
    transcript::{DocumentRenderInput, ToolRenderInput},
};

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

fn plan_heading(plan: &ActivityPlan) -> String {
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

fn plan_marker(status: PlanStepStatus) -> &'static str {
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

struct DocumentSource {
    heading: String,
    content: String,
    tokens_before: Option<u64>,
    reasoning: bool,
    empty_hint: &'static str,
    hidden_hint: &'static str,
}

impl DocumentSource {
    fn parse(source: &str) -> Option<Self> {
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
    prepare_document_body(config, message, width, format_markdown).or_else(|| {
        if !format_markdown || config.document_renderer.is_none() {
            return None;
        }
        let fallback = config.clone().with_document_renderer(None);
        prepare_document_body(&fallback, message, width, format_markdown)
    })
}

fn prepare_document_body(
    config: &TranscriptLayoutConfig,
    message: &TranscriptMessage,
    width: NonZeroU16,
    format_markdown: bool,
) -> Option<PreparedBody> {
    let activity = message.activity?;
    if activity.kind != Some(ActivityKind::ModelWork) {
        return None;
    }
    let end = activity.footer_start.unwrap_or(message.text().len());
    let (_, source) = message.text()[..end].split_once('\n')?;
    let DocumentSource {
        heading,
        content,
        tokens_before,
        reasoning,
        empty_hint,
        hidden_hint,
    } = DocumentSource::parse(source)?;
    let replacement = if format_markdown {
        config.document_renderer.as_ref().and_then(|renderer| {
            let document = ActivityDocument::from_snapshot(source).or_else(|| {
                ActivityReasoning::from_snapshot(source).map(|_| ActivityDocument {
                    title: heading.clone(),
                    markdown: content.clone(),
                })
            })?;
            renderer
                .render(DocumentRenderInput {
                    document: &document,
                    outcome: activity.outcome,
                    expanded: !config.compact_activities(),
                    columns: width,
                })
                .filter(|text| text.len() <= ToolOutput::MAX_SNAPSHOT_BYTES)
        })
    } else {
        None
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
        GlyphRole::NoticeHeading(NoticeLevel::Info),
    )
    .ok()?;
    if let Some(tokens) = tokens_before {
        append_plain(
            &mut body,
            &format!("Before compaction: {tokens} tokens"),
            width,
            GlyphRole::NoticeHeading(NoticeLevel::Info),
        )
        .ok()?;
    }
    let body_role = if reasoning {
        GlyphRole::ReasoningBody
    } else {
        GlyphRole::ActivityBody
    };
    if reasoning && format_markdown && !config.show_reasoning {
        append_plain(&mut body, hidden_hint, width, body_role).ok()?;
    } else if content.is_empty() && replacement.is_none() {
        append_plain(&mut body, empty_hint, width, body_role).ok()?;
    } else if format_markdown
        && let Ok(prepared) = markdown::prepare_with_links(
            replacement.as_deref().unwrap_or(&content),
            width,
            false,
            config.image_max_width,
            config.show_diagrams && !reasoning,
            config.active_link_resolver(),
            config.code_padding,
        )
        .or_else(|_| {
            markdown::prepare_with_links(
                &content,
                width,
                false,
                config.image_max_width,
                config.show_diagrams && !reasoning,
                config.active_link_resolver(),
                config.code_padding,
            )
        })
    {
        // Document bodies do not grant attachment authority. Images stay placeholders.
        let offset = body.height;
        body.height = body.height.checked_add(prepared.height)?;
        body.glyphs
            .extend(prepared.glyphs.into_iter().map(|mut glyph| {
                glyph.point.y += offset;
                PositionedTranscriptGrapheme {
                    point: glyph.point,
                    grapheme: glyph.grapheme,
                    role: body_role,
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
    } else {
        append_plain(&mut body, &content, width, body_role).ok()?;
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
    if format_markdown && config.compact_activities() {
        body = compact(body, width, config.activity_head_rows(false)).ok()?;
    }
    Some(body)
}

pub(super) fn prepare_tool(
    config: &TranscriptLayoutConfig,
    message: &TranscriptMessage,
    width: NonZeroU16,
    format_markdown: bool,
) -> Option<PreparedBody> {
    let activity = message.activity?;
    let kind = activity
        .kind
        .filter(|kind| matches!(kind, ActivityKind::ToolCall | ActivityKind::ToolResult))?;
    let text = message.text();
    let header_end = text.find('\n').unwrap_or(text.len());
    let footer_start = activity.footer_start.unwrap_or(text.len());
    let source = text[header_end.min(footer_start)..footer_start].strip_prefix('\n')?;
    if source.is_empty() {
        return None;
    }
    let output = ToolOutput::from_snapshot(source);
    let source = output
        .as_ref()
        .map_or(source, |output| output.plain_text.as_str());
    let mut skip_activity_folding = false;
    let rendered = if format_markdown {
        config
            .tool_renderer
            .as_ref()
            .and_then(|renderer| {
                renderer.render(ToolRenderInput {
                    kind,
                    outcome: activity.outcome,
                    source,
                    output: output.as_ref(),
                    expanded: !config.compact_activities(),
                    columns: width,
                })
            })
            .or_else(|| {
                output.as_ref().map(|output| {
                    skip_activity_folding = config.shell_tail_rows > 0 && is_shell(&output.tool);
                    let preview = (skip_activity_folding && config.compact_activities())
                        .then_some((width, config.shell_tail_rows));
                    tool_markdown(output, preview)
                })
            })
    } else {
        None
    };
    if rendered.is_none() && output.is_none() {
        return None;
    }
    let prepared = rendered.and_then(|rendered| {
        markdown::prepare_with_links(
            &rendered,
            width,
            config.show_images,
            config.image_max_width,
            config.show_diagrams,
            config.active_link_resolver(),
            config.code_padding,
        )
        .ok()
    });
    let mut body = PreparedBody {
        skip_activity_folding,
        rasters: Vec::new(),
        glyphs: Vec::new(),
        row_styles: Vec::new(),
        height: 0,
    };
    append_plain(
        &mut body,
        &text[..header_end],
        width,
        GlyphRole::ActivityHeading(activity.outcome),
    )
    .ok()?;
    if let Some(prepared) = prepared {
        let offset = body.height;
        body.height = body.height.checked_add(prepared.height)?;
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
        body.rasters
            .extend(prepared.rasters.into_iter().map(|mut raster| {
                raster.area.origin.y += offset;
                raster
            }));
    } else {
        append_plain(&mut body, source, width, GlyphRole::ActivityBody).ok()?;
    }
    if footer_start < text.len() {
        append_plain(
            &mut body,
            text[footer_start..].trim_start_matches('\n'),
            width,
            GlyphRole::ActivityOutcome(activity.outcome),
        )
        .ok()?;
    }
    Some(body)
}

// The source projection shares profile interpretation with the visual renderers.
pub(super) fn model_source_text(
    message: &TranscriptMessage,
    width: NonZeroU16,
) -> Result<Option<String>, TextFlowError> {
    let Some(activity) = message
        .activity
        .filter(|a| a.kind == Some(ActivityKind::ModelWork))
    else {
        return Ok(None);
    };
    let end = activity.footer_start.unwrap_or(message.text().len());
    let Some((_, source)) = message.text()[..end].split_once('\n') else {
        return Ok(None);
    };
    let mut parts = Vec::new();
    if let Some(plan) = ActivityPlan::from_snapshot(source) {
        parts.push(plan_heading(&plan));
        if let Some(explanation) = plan.explanation {
            parts.push(explanation);
        }
        if plan.steps.is_empty() {
            parts.push("No steps provided.".to_owned());
        }
        for step in plan.steps {
            let marker = plan_marker(step.status);
            if let Some(inner) = width
                .get()
                .checked_sub(4)
                .and_then(NonZeroU16::new)
                .filter(|n| n.get() >= 2)
            {
                let pages = TextPages::new(&step.text, inner)?;
                let mut text = marker.trim_end().to_owned();
                for row in 0..pages.row_count() {
                    let line = pages.window(row, NonZeroU16::MIN);
                    if row > 0 {
                        text.push('\n');
                    }
                    if !line.is_empty() {
                        text.push_str(if row == 0 { " " } else { "    " });
                        text.push_str(line);
                    }
                }
                parts.push(text);
            } else {
                parts.push(format!("{marker}{}", step.text));
            }
        }
    } else if let Some(document) = DocumentSource::parse(source) {
        parts.push(document.heading);
        if let Some(tokens) = document.tokens_before {
            parts.push(format!("Before compaction: {tokens} tokens"));
        }
        parts.push(if document.content.is_empty() {
            document.empty_hint.to_owned()
        } else {
            document.content
        });
    } else if let Some(notice) = ActivityNotice::from_snapshot(source) {
        parts.push(notice.title);
        parts.push(notice.message);
    } else {
        return Ok(None);
    }
    parts.push(message.text()[end..].trim_start_matches('\n').to_owned());
    Ok(Some(
        parts
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
    ))
}

// Source export uses the admitted readable body, independently of visual layout limits.
pub(super) fn tool_source_text(message: &TranscriptMessage) -> Option<String> {
    let output = ToolOutput::from_snapshot(message.tool_source()?)?;
    let text = message.text();
    let header_end = text.find('\n').unwrap_or(text.len());
    let footer_start = message.activity?.footer_start.unwrap_or(text.len());
    let mut source = text[..header_end].to_owned();
    if !output.plain_text.is_empty() {
        source.push('\n');
        source.push_str(&output.plain_text);
    }
    let footer = text[footer_start..].trim_start_matches('\n');
    if !footer.is_empty() {
        source.push('\n');
        source.push_str(footer);
    }
    Some(source)
}

fn is_shell(tool: &str) -> bool {
    matches!(
        tool,
        "run_command" | "bash" | "powershell" | "commandExecution"
    )
}

pub(super) fn content_block_markdown(block: &Value, successful: bool) -> String {
    let kind = block.get("type").and_then(Value::as_str).unwrap_or("");
    match (kind, block.get("text").and_then(Value::as_str)) {
        ("image", _) => {
            let mime = block
                .get("mimeType")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let data = block
                .get("data")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            image_markdown(mime, data)
        },
        ("inputImage", _) => {
            let url = block
                .get("imageUrl")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if let Some((mime, data)) = url
                .strip_prefix("data:")
                .and_then(|value| value.split_once(";base64,"))
            {
                image_markdown(mime, data)
            } else {
                literal_block("text", &format!("Image URL (not fetched)\n{url}"))
            }
        },
        ("resource_link", _) => resource_link_markdown(block)
            .unwrap_or_else(|| literal_block("json", &format!("{block:#}"))),
        ("resource", _) => embedded_resource_markdown(block, successful)
            .unwrap_or_else(|| literal_block("json", &format!("{block:#}"))),
        ("audio" | "inputAudio", _) => literal_block("text", "Audio output · playback unavailable"),
        _ => literal_block("json", &format!("{block:#}")),
    }
}

fn tool_markdown(output: &ToolOutput, preview: Option<(NonZeroU16, u16)>) -> String {
    if output.tool == "webSearch"
        && output.result.is_none()
        && output.content_items.is_none()
        && output.error.is_none()
        && let Some((heading, body)) = output.plain_text.split_once('\n')
        && matches!(
            heading,
            "Web search" | "Open web page" | "Find in web page" | "Web search · other action"
        )
    {
        return format!("**{heading}**\n\n{}", literal_block("text", body));
    }
    let identity = output.server.as_ref().map_or_else(
        || output.tool.clone(),
        |server| format!("{server}.{}", output.tool),
    );
    let is_shell = is_shell(&output.tool);
    let is_read = matches!(output.tool.as_str(), "read" | "read_file" | "readFile");
    let is_write = matches!(output.tool.as_str(), "write" | "write_file" | "writeFile");
    let is_edit = matches!(output.tool.as_str(), "edit" | "edit_file" | "editFile");
    let is_find = output.tool == "find";
    let is_grep = output.tool == "grep";
    let is_search = is_find || is_grep;
    let file_path = (is_read || is_write || is_edit || is_search || output.tool == "list_files")
        .then_some(output.arguments.as_ref())
        .flatten()
        .and_then(|args| args.get("file_path").or_else(|| args.get("path")))
        .and_then(|path| path.as_str());
    let successful = output.error.is_none()
        && output
            .result
            .as_ref()
            .and_then(|result| result.get("isError"))
            .and_then(|value| value.as_bool())
            != Some(true);
    let language = if successful && is_read {
        file_path.map_or("text", file_language)
    } else {
        "text"
    };
    let heading = file_path.map_or_else(|| identity.clone(), |path| format!("{identity} · {path}"));
    let mut sections = vec![literal_block("text", &heading)];
    let mut truncation_presented = false;
    let mut progress_presented = false;
    if let Some(arguments) = &output.arguments {
        let pattern = is_search
            .then(|| arguments.get("pattern").and_then(Value::as_str))
            .flatten();
        let command = is_shell
            .then(|| arguments.get("command").and_then(Value::as_str))
            .flatten();
        let content = is_write
            .then(|| arguments.get("content").and_then(|value| value.as_str()))
            .flatten();
        let edits = is_edit.then(|| edit_markdown(arguments)).flatten();
        let mut displayed_arguments = arguments.clone();
        if let Some(pattern) = pattern {
            displayed_arguments
                .as_object_mut()
                .unwrap()
                .remove("pattern");
            let label = if is_grep {
                "Content search pattern"
            } else {
                "File search pattern"
            };
            sections.push(format!("**{label}**\n\n{}", literal_block("text", pattern)));
        }
        if command.is_some()
            && let Some(fields) = displayed_arguments.as_object_mut()
        {
            fields.remove("command");
        }
        if let Some((_, array)) = &edits
            && let Some(fields) = displayed_arguments.as_object_mut()
        {
            if *array {
                fields.remove("edits");
            } else {
                fields.remove("oldText");
                fields.remove("newText");
            }
        }
        if content.is_some()
            && let Some(fields) = displayed_arguments.as_object_mut()
        {
            fields.remove("content");
        }
        if !displayed_arguments
            .as_object()
            .is_some_and(|fields| fields.is_empty())
        {
            sections.push(format!(
                "**Arguments**\n\n{}",
                literal_block("json", &format!("{displayed_arguments:#}"))
            ));
        }
        if let Some(command) = command {
            let language = if output.tool == "powershell" {
                "powershell"
            } else {
                "bash"
            };
            sections.push(format!(
                "**Command**\n\n{}",
                literal_block(
                    language,
                    if command.is_empty() {
                        "(empty command)"
                    } else {
                        command
                    }
                )
            ));
        }
        if let Some((edits, _)) = edits {
            sections.push(format!("**Proposed replacements**\n\n{edits}"));
        }
        if let Some(content) = content {
            sections.push(format!(
                "**Proposed file content**\n\n{}",
                if content.is_empty() {
                    literal_block("text", "(empty file)")
                } else {
                    literal_block(file_path.map_or("text", file_language), content)
                }
            ));
        }
    }
    for block in output.content_blocks() {
        let kind = block
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let text = block.get("text").and_then(|value| value.as_str());
        let body = match (kind, text) {
            ("diff", Some(text)) => format!(
                "{}\n\n{}",
                literal_block(
                    "text",
                    block
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("File change")
                ),
                if text.is_empty() {
                    literal_block("text", "No textual changes")
                } else {
                    literal_block("diff", text)
                },
            ),
            ("text" | "inputText", Some(text)) => (successful && output.tool == "read_files")
                .then(|| batch_read_markdown(text))
                .flatten()
                .or_else(|| {
                    (successful && output.tool == "list_files")
                        .then(|| {
                            let truncated = output.result.as_ref()?.get("truncated")?.as_bool()?;
                            directory_markdown(text, truncated)
                                .inspect(|_| truncation_presented = true)
                        })
                        .flatten()
                })
                .or_else(|| {
                    (output.tool == "run_command"
                        && output
                            .result
                            .as_ref()
                            .and_then(|result| result.get("progress"))
                            .and_then(Value::as_bool)
                            == Some(true))
                    .then(|| {
                        command_progress_markdown(text, preview)
                            .inspect(|_| progress_presented = true)
                    })
                    .flatten()
                })
                .or_else(|| {
                    (output.tool == "run_command")
                        .then(|| command_result_markdown(text, preview))
                        .flatten()
                })
                .or_else(|| {
                    successful
                        .then(|| mutation_result_markdown(&output.tool, text))
                        .flatten()
                })
                .unwrap_or_else(|| {
                    if is_search && successful {
                        let label = if is_grep {
                            "Search results"
                        } else {
                            "Matching paths"
                        };
                        format!("**{label}**\n\n{}", literal_block("text", text))
                    } else if is_shell {
                        shell_output_block(text, preview)
                    } else {
                        literal_block(language, text)
                    }
                }),
            _ => content_block_markdown(block, successful),
        };
        sections.push(body);
    }
    let has_content = output
        .result
        .as_ref()
        .and_then(|result| result.get("content"))
        .or(output.content_items.as_ref());
    if has_content.is_some_and(|content| content.as_array().is_some_and(|blocks| blocks.is_empty()))
    {
        sections.push(literal_block("text", "(empty content)"));
    }
    if let Some(items) = output
        .content_items
        .as_ref()
        .filter(|items| !items.is_array())
    {
        sections.push(literal_block("json", &format!("{items:#}")));
    }
    if let Some(result) = &output.result {
        let mut remaining = result.clone();
        if is_shell {
            let mut details = Vec::new();
            for (key, label, suffix) in [
                ("exitCode", "Exit status", ""),
                ("durationMs", "Duration", " ms"),
                ("status", "Status", ""),
            ] {
                if let Some(value) = remaining.get(key) {
                    let text = match key {
                        "exitCode" => value.as_i64().map(|value| value.to_string()),
                        "durationMs" => value.as_u64().map(|value| value.to_string()),
                        _ => value.as_str().map(str::to_owned),
                    };
                    if let Some(text) = text {
                        details.push(format!("{label}: {text}{suffix}"));
                        remaining.as_object_mut().unwrap().remove(key);
                    }
                }
            }
            if !details.is_empty() {
                sections.push(literal_block("text", &details.join("\n")));
            }
        }
        if is_shell
            && let Some(details) = remaining.get("details")
            && let Some((markdown, truncated)) = shell_details_markdown(details, output)
        {
            sections.push(markdown);
            truncation_presented |= truncated;
            remaining.as_object_mut().unwrap().remove("details");
        }
        if is_search
            && successful
            && let Some(details) = remaining.get("details")
            && let Some((markdown, truncated)) = search_details_markdown(details, output)
        {
            sections.push(markdown);
            truncation_presented |= truncated;
            remaining.as_object_mut().unwrap().remove("details");
        }
        if remaining.get("content").is_some_and(Value::is_array)
            && let Some(details) = execution_details(&remaining)
        {
            sections.push(format!(
                "**Execution details**\n\n{}",
                literal_block("text", &details)
            ));
            let fields = remaining
                .as_object_mut()
                .expect("validated execution metadata");
            for key in ["call_id", "tool_id", "execution_host", "outcome", "isError"] {
                fields.remove(key);
            }
        }
        let result = &remaining;
        if result.as_object().is_some_and(|fields| fields.is_empty()) {
            // Reported command metadata was already displayed above.
        } else if let Some(fields) = result
            .as_object()
            .filter(|fields| fields.get("content").is_some_and(|value| value.is_array()))
        {
            for (key, value) in fields.iter().filter(|(key, _)| *key != "content") {
                if key == "retainedOutput"
                    && value.as_object().is_some_and(|fields| fields.len() == 1)
                    && let Some(truncated) = value.get("truncated").and_then(Value::as_bool)
                {
                    sections.push(if truncated {
                        "**Retained output · /output**\n\nSome captured output was omitted or unavailable.".to_owned()
                    } else {
                        "**Retained output · /output**\n\nCaptured output is available in the output viewer.".to_owned()
                    });
                    continue;
                }
                if key == "progress" && progress_presented {
                    continue;
                }
                if key == "truncated"
                    && let Some(truncated) = value.as_bool()
                {
                    if truncated && !truncation_presented {
                        sections.push("**Output truncated**".to_owned());
                    }
                    continue;
                }
                let label = match key.as_str() {
                    "structuredContent" => "**Structured result**".to_owned(),
                    "_meta" => "**Metadata**".to_owned(),
                    _ => literal_block("text", key),
                };
                sections.push(format!(
                    "{label}\n\n{}",
                    literal_block("json", &format!("{value:#}"))
                ));
            }
        } else {
            sections.push(literal_block("json", &format!("{result:#}")));
        }
    }
    if let Some(error) = &output.error {
        sections.push(format!(
            "**Error details**\n\n{}",
            literal_block("json", &format!("{error:#}"))
        ));
    }
    sections.join("\n\n")
}

// This is a presentation shape, not provider detection or execution authority. Partial or
// unfamiliar metadata keeps the generic renderer so no original field disappears.
fn execution_details(result: &Value) -> Option<String> {
    let text = |key| result.get(key)?.as_str().filter(|value| !value.is_empty());
    let call = text("call_id")?;
    let tool = text("tool_id")?;
    let host = text("execution_host")?;
    let outcome = text("outcome")?;
    let failed = result.get("isError")?.as_bool()?;
    result.get("truncated")?.as_bool()?;
    if !matches!(outcome, "completed" | "failed" | "interrupted")
        || failed != (outcome != "completed")
    {
        return None;
    }
    Some(format!(
        "Status: {outcome}\nTool: {tool}\nCall: {call}\nHost: {host}\nError: {failed}"
    ))
}

fn embedded_resource_markdown(block: &Value, successful: bool) -> Option<String> {
    let fields = block.as_object()?;
    let resource = fields.get("resource")?.as_object()?;
    let uri = resource.get("uri")?.as_str()?;
    if uri.is_empty() {
        return None;
    }
    let mime = match resource.get("mimeType") {
        Some(value) => Some(value.as_str()?),
        None => None,
    };
    let mut heading = format!("Resource · {uri}");
    if let Some(mime) = mime {
        heading.push_str(&format!("\nType: {mime}"));
    }
    let body = match (resource.get("text"), resource.get("blob")) {
        (Some(text), None) => literal_block(
            if successful {
                file_language(uri)
            } else {
                "text"
            },
            text.as_str()?,
        ),
        (None, Some(blob)) => {
            let blob = blob.as_str()?;
            if let Some(mime @ ("image/png" | "image/jpeg")) = mime {
                image_markdown(mime, blob)
            } else {
                literal_block("text", "Binary resource (retained in output record)")
            }
        },
        _ => return None,
    };
    let mut metadata = fields
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "type" | "resource"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Map<_, _>>();
    let resource_metadata = resource
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "uri" | "mimeType" | "text" | "blob"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Map<_, _>>();
    if !resource_metadata.is_empty() {
        metadata.insert("resource".to_owned(), Value::Object(resource_metadata));
    }
    let mut sections = vec![literal_block("text", &heading), body];
    if !metadata.is_empty() {
        sections.push(format!(
            "**Resource metadata**\n\n{}",
            literal_block("json", &format!("{:#}", Value::Object(metadata)))
        ));
    }
    Some(sections.join("\n\n"))
}

fn resource_link_markdown(block: &Value) -> Option<String> {
    if block.to_string().len() > 256 * 1024 {
        return None;
    }
    let fields = block.as_object()?;
    let name = fields.get("name")?.as_str()?;
    let uri = fields.get("uri")?.as_str()?;
    if name.is_empty() || uri.is_empty() {
        return None;
    }
    let mut metadata = fields.clone();
    for key in ["type", "name", "uri"] {
        metadata.remove(key);
    }
    let mut labels = vec![format!("Name: {name}"), format!("URI: {uri}")];
    for (key, label) in [("title", "Title"), ("mimeType", "Type")] {
        if let Some(value) = fields.get(key) {
            labels.push(format!("{label}: {}", value.as_str()?));
            metadata.remove(key);
        }
    }
    if let Some(size) = fields.get("size") {
        labels.push(format!("Size: {} bytes", size.as_u64()?));
        metadata.remove("size");
    }
    let mut sections = vec![format!(
        "**Resource link**\n\n{}",
        literal_block("text", &labels.join("\n"))
    )];
    if let Some(description) = fields.get("description") {
        sections.push(format!(
            "**Description**\n\n{}",
            literal_block("text", description.as_str()?)
        ));
        metadata.remove("description");
    }
    if !metadata.is_empty() {
        sections.push(format!(
            "**Resource metadata**\n\n{}",
            literal_block("json", &format!("{:#}", Value::Object(metadata)))
        ));
    }
    Some(sections.join("\n\n"))
}

fn search_details_markdown(details: &Value, output: &ToolOutput) -> Option<(String, bool)> {
    if details.to_string().len() > 256 * 1024 {
        return None;
    }
    let fields = details.as_object()?;
    let is_grep = output.tool == "grep";
    let limit_key = if is_grep {
        "matchLimitReached"
    } else {
        "resultLimitReached"
    };
    if fields.is_empty()
        || fields.keys().any(|key| {
            key != limit_key && key != "truncation" && !(is_grep && key == "linesTruncated")
        })
    {
        return None;
    }
    let mut sections = Vec::new();
    let mut truncated = false;
    if let Some(limit) = fields.get(limit_key) {
        let limit = limit.as_u64().filter(|limit| *limit > 0)?;
        let (heading, label) = if is_grep {
            ("Partial content search", "Match limit reached")
        } else {
            ("Partial file search", "Result limit reached")
        };
        sections.push(format!("**{heading}**\n\n{label}: {limit}"));
        truncated = true;
    }
    if let Some(lines) = fields.get("linesTruncated")
        && lines.as_bool()?
    {
        sections.push(
            "**Partial search lines**\n\nSome matching or context lines were truncated.".to_owned(),
        );
        truncated = true;
    }
    if let Some(truncation) = fields.get("truncation") {
        let details = Value::Object(
            [("truncation".to_owned(), truncation.clone())]
                .into_iter()
                .collect(),
        );
        let (markdown, partial) = shell_details_markdown(&details, output)?;
        sections.push(markdown);
        truncated |= partial;
    }
    Some((sections.join("\n\n"), truncated))
}

fn directory_markdown(source: &str, truncated: bool) -> Option<String> {
    if source.len() > 256 * 1024 {
        return None;
    }
    let source = if truncated {
        source
            .strip_suffix("\n[yo: tool output truncated]")
            .unwrap_or(source)
    } else {
        source
    };
    if !source.is_empty() && !source.ends_with('\n') {
        return None;
    }
    let entries = source.split_terminator('\n').collect::<Vec<_>>();
    if entries.len() > 1024
        || entries.iter().any(|entry| {
            entry.is_empty() || entry.len() > 1024 || entry.chars().any(char::is_control)
        })
    {
        return None;
    }
    let directories = entries.iter().filter(|entry| entry.ends_with('/')).count();
    let mut body = format!(
        "**{} {} shown · {} {}**",
        entries.len(),
        if entries.len() == 1 {
            "entry"
        } else {
            "entries"
        },
        directories,
        if directories == 1 {
            "directory"
        } else {
            "directories"
        }
    );
    if entries.is_empty() {
        body.push_str("\n\n");
        body.push_str(if truncated {
            "No complete entries were returned."
        } else {
            "No entries were returned."
        });
    } else {
        let rows = entries
            .iter()
            .map(|entry| {
                let kind = if entry.ends_with('/') { "dir " } else { "file" };
                format!("{kind}  {entry}")
            })
            .collect::<Vec<_>>()
            .join("\n");
        body.push_str(&format!("\n\n{}", literal_block("text", &rows)));
    }
    if truncated {
        body.push_str("\n\n**Partial listing · output was truncated**");
    }
    Some(body)
}

fn shell_details_markdown(details: &Value, output: &ToolOutput) -> Option<(String, bool)> {
    if details.to_string().len() > 256 * 1024 {
        return None;
    }
    let fields = details.as_object()?;
    if fields.is_empty()
        || fields
            .keys()
            .any(|key| !matches!(key.as_str(), "fullOutputPath" | "truncation"))
    {
        return None;
    }
    let mut sections = Vec::new();
    if let Some(path) = fields.get("fullOutputPath") {
        let path = path.as_str()?;
        if path.is_empty() || path.len() > 4096 || path.chars().any(char::is_control) {
            return None;
        }
        sections.push(format!(
            "**Full output file**\n\n{}",
            literal_block("text", path)
        ));
    }
    let mut truncated = false;
    if let Some(value) = fields.get("truncation") {
        let truncation = value.as_object()?;
        if truncation.len() != 11
            || truncation.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "content"
                        | "truncated"
                        | "truncatedBy"
                        | "totalLines"
                        | "totalBytes"
                        | "outputLines"
                        | "outputBytes"
                        | "lastLinePartial"
                        | "firstLineExceedsLimit"
                        | "maxLines"
                        | "maxBytes"
                )
            })
        {
            return None;
        }
        let content = value.get("content")?.as_str()?;
        if !content.is_empty()
            && !output.content_blocks().any(|block| {
                block
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| text.starts_with(content))
            })
        {
            return None;
        }

        truncated = value.get("truncated")?.as_bool()?;
        let total_lines = value.get("totalLines")?.as_u64()?;
        let total_bytes = value.get("totalBytes")?.as_u64()?;
        let output_lines = value.get("outputLines")?.as_u64()?;
        let output_bytes = value.get("outputBytes")?.as_u64()?;
        let max_lines = value.get("maxLines")?.as_u64()?;
        let max_bytes = value.get("maxBytes")?.as_u64()?;
        let partial = value.get("lastLinePartial")?.as_bool()?;
        let first_exceeded = value.get("firstLineExceedsLimit")?.as_bool()?;
        if output_lines > total_lines
            || output_lines > max_lines
            || output_bytes > max_bytes
            || output_lines != content.split_terminator('\n').count() as u64
            || output_bytes > total_bytes
            || output_bytes != content.len() as u64
        {
            return None;
        }
        if truncated {
            let reason = value.get("truncatedBy")?.as_str()?;
            let limit = match reason {
                "lines" => format!("Line limit: {max_lines}"),
                "bytes" => format!("Byte limit: {max_bytes} bytes"),
                _ => return None,
            };
            let mut lines = vec![
                format!("Output lines: {output_lines} of {total_lines}"),
                format!("Output bytes: {output_bytes} of {total_bytes}"),
                limit,
            ];
            if partial {
                lines.push("Last line is partial".to_owned());
            }
            if first_exceeded {
                lines.push("First line exceeds the byte limit".to_owned());
            }
            sections.insert(
                0,
                format!(
                    "**Output truncated**\n\n{}",
                    literal_block("text", &lines.join("\n"))
                ),
            );
        } else {
            if !value.get("truncatedBy")?.is_null()
                || partial
                || first_exceeded
                || output_lines != total_lines
                || output_bytes != total_bytes
            {
                return None;
            }
            sections.insert(
                0,
                literal_block(
                    "text",
                    &format!("Complete output: {output_lines} lines · {output_bytes} bytes"),
                ),
            );
        }
    }
    Some((sections.join("\n\n"), truncated))
}

fn shell_output_block(source: &str, preview: Option<(NonZeroU16, u16)>) -> String {
    let Some((width, rows)) = preview else {
        return literal_block("text", source);
    };
    let indent = width.get().saturating_sub(2).min(1);
    let padding = (width.get() - indent).saturating_sub(4).min(1);
    let inner = NonZeroU16::new(width.get() - indent - padding).unwrap();
    let Some(rows) = NonZeroU16::new(rows) else {
        return literal_block("text", source);
    };
    let Ok(tail) = flow_tail_text(source, inner, rows) else {
        return literal_block("text", source);
    };
    if tail.skipped_rows == 0 {
        return literal_block("text", source);
    }
    let first_row = tail.skipped_rows;
    let flow = tail.flow;
    // Rebuild already flowed display cells so tabs and wide characters retain their columns.
    let mut lines = vec![String::new(); usize::from(flow.height)];
    let mut columns = vec![0_u16; usize::from(flow.height)];
    for glyph in flow.glyphs {
        let index = usize::from(glyph.point.y);
        let line = &mut lines[index];
        line.extend(std::iter::repeat_n(
            ' ',
            usize::from(glyph.point.x.saturating_sub(columns[index])),
        ));
        line.push_str(glyph.grapheme.as_str());
        columns[index] = glyph.point.x + glyph.grapheme.width().get();
    }
    format!(
        "{first_row} earlier output rows hidden · Ctrl+O expand\n\n{}",
        literal_block("text", &lines.join("\n"))
    )
}

fn command_progress_markdown(source: &str, preview: Option<(NonZeroU16, u16)>) -> Option<String> {
    let value: Value = from_str(source).ok()?;
    if value.as_object()?.len() != 2 {
        return None;
    }
    let stdout = value.get("stdout")?.as_str()?;
    let stderr = value.get("stderr")?.as_str()?;
    Some(format!(
        "**stdout**\n\n{}\n\n**stderr**\n\n{}",
        shell_output_block(
            if stdout.is_empty() {
                "(no stdout yet)"
            } else {
                stdout
            },
            preview
        ),
        shell_output_block(
            if stderr.is_empty() {
                "(no stderr yet)"
            } else {
                stderr
            },
            preview
        )
    ))
}

fn command_result_markdown(source: &str, preview: Option<(NonZeroU16, u16)>) -> Option<String> {
    // Native combined output has no escaping for its stream delimiter. Ambiguous
    // or truncated receipts must stay literal, without invented stream attribution.
    if source.contains("[yo: tool output truncated]") {
        return None;
    }
    let (status, body) = source.strip_prefix("status: ")?.split_once("\nstdout:\n")?;
    if status != "signal" && status.parse::<i32>().ok()?.to_string() != status {
        return None;
    }
    let separator = "\nstderr:\n";
    if body.match_indices(separator).count() != 1 {
        return None;
    }
    let (stdout, stderr) = body.split_once(separator)?;
    Some(format!(
        "**Exit status: {status}**\n\n**stdout**\n\n{}\n\n**stderr**\n\n{}",
        shell_output_block(
            if stdout.is_empty() {
                "(no stdout)"
            } else {
                stdout
            },
            preview
        ),
        shell_output_block(
            if stderr.is_empty() {
                "(no stderr)"
            } else {
                stderr
            },
            preview
        )
    ))
}

fn mutation_result_markdown(tool: &str, source: &str) -> Option<String> {
    let (field, label) = match tool {
        "edit_file" => ("replacements", "Applied replacements"),
        "write_file" => ("bytes", "Written bytes"),
        _ => return None,
    };
    if source.len() > 16 * 1024 {
        return None;
    }
    let value: Value = from_str(source).ok()?;
    let result = value.as_object()?;
    if result.len() != 3 || result.get("status")?.as_str()? != "ok" {
        return None;
    }
    let count = result.get(field)?.as_u64()?;
    let path = result.get("path")?.as_str()?;
    Some(literal_block("text", &format!("{path}\n{label}: {count}")))
}

fn edit_markdown(arguments: &Value) -> Option<(String, bool)> {
    let array = arguments.get("edits");
    let replacements = if let Some(edits) = array {
        if arguments.get("oldText").is_some() || arguments.get("newText").is_some() {
            return None;
        }
        let edits = edits.as_array()?;
        if edits.is_empty() || edits.len() > 256 {
            return None;
        }
        edits
            .iter()
            .map(|edit| {
                if edit.as_object()?.len() != 2 {
                    return None;
                }
                Some((
                    edit.get("oldText")?.as_str()?,
                    edit.get("newText")?.as_str()?,
                ))
            })
            .collect::<Option<Vec<_>>>()?
    } else {
        vec![(
            arguments.get("oldText")?.as_str()?,
            arguments.get("newText")?.as_str()?,
        )]
    };
    let mut bytes = 0usize;
    for (old, new) in &replacements {
        bytes = bytes.checked_add(old.len())?.checked_add(new.len())?;
        if old.is_empty() || bytes > 256 * 1024 {
            return None;
        }
    }
    let mut sections = Vec::new();
    for (index, (old, new)) in replacements.into_iter().enumerate() {
        let mut diff = String::new();
        for (prefix, source) in [('-', old), ('+', new)] {
            for line in source.split_inclusive('\n') {
                diff.push(prefix);
                diff.push_str(line);
                if !line.ends_with('\n') {
                    diff.push_str(if prefix == '-' {
                        "\n\\ Old text has no trailing newline\n"
                    } else {
                        "\n\\ New text has no trailing newline\n"
                    });
                }
            }
        }
        sections.push(format!(
            "**Replacement {}**\n\n{}",
            index + 1,
            literal_block("diff", &diff)
        ));
    }
    Some((sections.join("\n\n"), array.is_some()))
}

fn batch_read_markdown(source: &str) -> Option<String> {
    // Only this native tool's bounded, complete result shape becomes file panels.
    // Unknown fields or truncated JSON retain the entire literal source.
    if source.len() > 256 * 1024 {
        return None;
    }
    let value: Value = from_str(source).ok()?;
    let root = value.as_object()?;
    if root.len() != 1 {
        return None;
    }
    let results = root.get("results")?.as_array()?;
    if results.is_empty() || results.len() > 8 {
        return None;
    }
    let mut sections = Vec::new();
    for result in results {
        let item = result.as_object()?;
        let path = item.get("path")?.as_str()?;
        let status = item.get("status")?.as_str()?;
        match status {
            "error" => {
                if item.len() != 3 {
                    return None;
                }
                let error = item.get("error")?.as_str()?;
                sections.push(literal_block(
                    "text",
                    &format!("Read failed · {path}\n{error}"),
                ));
            },
            "ok" => {
                if item.keys().any(|key| {
                    !matches!(
                        key.as_str(),
                        "path" | "status" | "start" | "end" | "total" | "content" | "next_offset"
                    )
                }) {
                    return None;
                }
                let start = item.get("start")?.as_u64()?;
                let end = item.get("end")?.as_u64()?;
                let total = item.get("total")?.as_u64()?;
                let content = item.get("content")?.as_str()?;
                let next = match item.get("next_offset") {
                    Some(value) => Some(value.as_u64()?),
                    None => None,
                };
                if total == 0 {
                    if start != 0 || end != 0 || !content.is_empty() || next.is_some() {
                        return None;
                    }
                    sections.push(literal_block("text", &format!("{path}\n(empty file)")));
                } else {
                    if start == 0
                        || end < start
                        || end > total
                        || content.lines().count() as u64 != end - start + 1
                        || next != (end < total).then(|| end + 1)
                    {
                        return None;
                    }
                    sections.push(literal_block(
                        "text",
                        &format!("{path}\nLines {start}–{end} of {total}"),
                    ));
                    sections.push(literal_block(file_language(path), content));
                    if let Some(next) = next {
                        sections.push(literal_block(
                            "text",
                            &format!("Continue reading at line {next}"),
                        ));
                    }
                }
            },
            _ => return None,
        }
    }
    Some(sections.join("\n\n"))
}

fn file_language(path: &str) -> &'static str {
    // Presentation-only suffix inference: never resolve paths or interpret Markdown,
    // diagrams or image syntax contained in file output.
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match file
        .rsplit_once('.')
        .map_or("", |(_, extension)| extension)
        .to_ascii_lowercase()
        .as_str()
    {
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "json" => "json",
        "sh" | "bash" => "bash",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" => "cpp",
        "go" => "go",
        "rb" => "ruby",
        "java" => "java",
        "html" | "htm" => "html",
        "css" => "css",
        "xml" => "xml",
        "yaml" | "yml" => "yaml",
        "sql" => "sql",
        _ => "text",
    }
}

fn literal_block(language: &str, source: &str) -> String {
    // A tool's literal fences must not escape into image/link presentation markup.
    let longest = source
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}{language}\n{source}\n{fence}")
}

fn image_markdown(mime: &str, data: &str) -> String {
    if matches!(mime, "image/png" | "image/jpeg")
        && !data.is_empty()
        && data
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"+/=".contains(&byte))
    {
        format!("![Image output](data:{mime};base64,{data})")
    } else {
        literal_block(
            "text",
            "Image output · unsupported or invalid media (original retained)",
        )
    }
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
