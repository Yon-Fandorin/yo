use yo_core::{
    ActivityDocument, ActivityKind, ActivityNotice, ActivityPlan, ActivityReasoning, NoticeLevel,
    ToolOutput,
};

use super::{
    DocumentSource, GlyphRole, NonZeroU16, PositionedTranscriptGrapheme, PreparedBody,
    TranscriptLayoutConfig, TranscriptMessage, append_plain, compact, markdown, plan_heading,
    plan_marker,
};
use crate::{
    text::flow::{TextFlowError, TextPages},
    transcript::DocumentRenderInput,
};

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
