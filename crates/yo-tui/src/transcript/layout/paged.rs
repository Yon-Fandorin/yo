//! A reading body's document rows are indexed once; visible pages acquire cells.

use yo_core::{ActivityDocument, ActivityNotice, ActivityPlan, ActivityReasoning, ToolOutput};

use super::{
    ActivityKind, AssistantRenderInput, Decoration, GlyphRole, MessageContent, MessageRole,
    NonZeroU16, NoticeLevel, Point, PositionedTranscriptGrapheme, PreparedBody, RasterImage,
    TranscriptLayoutConfig, TranscriptMessage, TranscriptRenderError, activity, body_role,
    flow_text, markdown,
};
use crate::{text::flow::TextPages, transcript::DocumentRenderInput};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PagedBody {
    sections: Vec<(usize, Section, GlyphRole)>,
    pub(super) height: usize,
    pub(super) width: NonZeroU16,
    pub(super) hyperlinks: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Section {
    Literal(TextPages, Vec<(usize, GlyphRole)>),
    Markdown(markdown::PagedMarkdown),
    Rows(std::sync::Arc<PagedBody>, usize, usize),
    PlanStep(TextPages, &'static str, NonZeroU16),
}

impl PagedBody {
    pub(super) fn raster_rows(&self) -> Vec<(usize, RasterImage)> {
        self.sections
            .iter()
            .flat_map(|(start, section, _)| match section {
                Section::Literal(_, _) => Vec::new(),
                Section::PlanStep(_, _, _) => Vec::new(),
                Section::Markdown(pages) => pages
                    .raster_rows()
                    .into_iter()
                    .map(|(row, raster)| (start + row, raster))
                    .collect(),
                Section::Rows(body, first, height) => body
                    .raster_rows()
                    .into_iter()
                    .filter(|(row, raster)| {
                        *row >= *first
                            && row + usize::from(raster.area.size.height) <= first + height
                    })
                    .map(|(row, raster)| (start + row - first, raster))
                    .collect(),
            })
            .collect()
    }
    fn new(width: NonZeroU16, hyperlinks: bool) -> Self {
        Self {
            sections: Vec::new(),
            height: 0,
            width,
            hyperlinks,
        }
    }

    fn literal(
        &mut self,
        source: &str,
        role: GlyphRole,
        prose: bool,
    ) -> Result<(), TranscriptRenderError> {
        let pages = if prose {
            TextPages::prose(source, self.width, true, false)
        } else {
            TextPages::new(source, self.width)
        }
        .map_err(TranscriptRenderError::Text)?;
        self.append(Section::Literal(pages, Vec::new()), role)
    }

    fn append(&mut self, section: Section, role: GlyphRole) -> Result<(), TranscriptRenderError> {
        let height = match &section {
            Section::Literal(p, _) => p.row_count(),
            Section::Markdown(p) => p.height,
            Section::Rows(_, _, height) => *height,
            Section::PlanStep(pages, _, _) => pages.row_count().max(1),
        };
        if height > 0 {
            self.sections.push((self.height, section, role));
            self.height = self
                .height
                .checked_add(height)
                .ok_or(TranscriptRenderError::HeightOverflow)?;
        }
        Ok(())
    }

    fn markdown(
        &mut self,
        source: &str,
        role: GlyphRole,
        config: &TranscriptLayoutConfig,
    ) -> Result<(), TranscriptRenderError> {
        self.append(
            Section::Markdown(
                markdown::PagedMarkdown::new(
                    source,
                    self.width,
                    config.show_images,
                    config.image_max_width,
                    config.show_diagrams,
                    config.active_link_resolver(),
                    config.code_padding,
                )
                .map_err(TranscriptRenderError::Text)?,
            ),
            role,
        )
    }

    pub(super) fn window(&self, first: usize, height: NonZeroU16) -> PreparedBody {
        let end = first.saturating_add(usize::from(height.get()));
        let mut body = PreparedBody {
            skip_activity_folding: true,
            rasters: Vec::new(),
            glyphs: Vec::new(),
            row_styles: Vec::new(),
            height: height.get(),
        };
        for (start, section, role) in &self.sections {
            let section_height = match section {
                Section::Literal(p, _) => p.row_count(),
                Section::Markdown(p) => p.height,
                Section::Rows(_, _, height) => *height,
                Section::PlanStep(pages, _, _) => pages.row_count().max(1),
            };
            if *start >= end || start + section_height <= first {
                continue;
            }
            let from = first.saturating_sub(*start);
            let offset =
                u16::try_from(start.saturating_sub(first)).expect("visible section offset");
            let remaining = NonZeroU16::new(
                (height.get() - offset).min(
                    u16::try_from(
                        section_height
                            .saturating_sub(from)
                            .min(usize::from(u16::MAX)),
                    )
                    .expect("bounded section window"),
                ),
            )
            .expect("visible section height");
            match section {
                Section::Literal(pages, roles) => {
                    let flow = flow_text(pages.window(from, remaining), self.width)
                        .expect("validated bounded display page");
                    body.glyphs.extend(flow.glyphs.into_iter().map(|g| {
                        let original =
                            pages.source_byte_at(pages.window_offset(from) + g.byte_index);
                        let next = roles.partition_point(|(start, _)| *start <= original);
                        let role = next
                            .checked_sub(1)
                            .and_then(|index| roles.get(index))
                            .map_or(*role, |(_, role)| *role);
                        PositionedTranscriptGrapheme {
                            point: Point::new(g.point.x, offset + g.point.y),
                            grapheme: g.grapheme,
                            role,
                            decoration: Decoration::default(),
                            hyperlink: None,
                        }
                    }));
                },
                Section::Markdown(pages) => {
                    let page = pages.window(from, remaining);
                    body.glyphs.extend(page.glyphs.into_iter().map(|g| {
                        PositionedTranscriptGrapheme {
                            point: Point::new(g.point.x, offset + g.point.y),
                            grapheme: g.grapheme,
                            role: *role,
                            decoration: g.decoration,
                            hyperlink: g.hyperlink,
                        }
                    }));
                    body.row_styles.extend(
                        page.row_styles
                            .into_iter()
                            .map(|(row, style)| (offset + row, style)),
                    );
                    body.rasters.extend(page.rasters.into_iter().map(|mut r| {
                        r.area.origin.y += offset;
                        r
                    }));
                },
                Section::Rows(source, source_first, _) => {
                    let page = source.window(source_first + from, remaining);
                    body.glyphs.extend(page.glyphs.into_iter().map(|mut g| {
                        g.point.y += offset;
                        g
                    }));
                    body.row_styles.extend(
                        page.row_styles
                            .into_iter()
                            .map(|(row, style)| (offset + row, style)),
                    );
                },
                Section::PlanStep(pages, marker, inner) => {
                    if from == 0 {
                        body.glyphs.extend(
                            flow_text(marker.trim_end(), self.width)
                                .expect("validated plan marker")
                                .glyphs
                                .into_iter()
                                .map(|g| PositionedTranscriptGrapheme {
                                    point: Point::new(g.point.x, offset + g.point.y),
                                    grapheme: g.grapheme,
                                    role: *role,
                                    decoration: Decoration::default(),
                                    hyperlink: None,
                                }),
                        );
                    }
                    body.glyphs.extend(
                        flow_text(pages.window(from, remaining), *inner)
                            .expect("validated plan page")
                            .glyphs
                            .into_iter()
                            .map(|g| PositionedTranscriptGrapheme {
                                point: Point::new(g.point.x + 4, offset + g.point.y),
                                grapheme: g.grapheme,
                                role: *role,
                                decoration: Decoration::default(),
                                hyperlink: None,
                            }),
                    );
                },
            }
        }
        body
    }

    fn compact(self, opening: u16) -> Result<Self, TranscriptRenderError> {
        let Some(start) = self
            .sections
            .iter()
            .find(|(_, _, role)| matches!(role, GlyphRole::ActivityBody | GlyphRole::ReasoningBody))
            .map(|(start, _, _)| *start)
        else {
            return Ok(self);
        };
        let end = self
            .sections
            .iter()
            .find(|(_, _, role)| matches!(role, GlyphRole::ActivityOutcome(_)))
            .map_or(self.height, |(start, _, _)| *start);
        if end.saturating_sub(start) <= usize::from(opening) + 6 {
            return Ok(self);
        }
        self.compact_range(start, end, opening)
    }

    fn compact_range(
        self,
        start: usize,
        end: usize,
        opening: u16,
    ) -> Result<Self, TranscriptRenderError> {
        if end.saturating_sub(start) <= usize::from(opening) + 6 {
            return Ok(self);
        }
        let hidden_start = start + usize::from(opening);
        let hidden_end = end - 3;
        let mut folded = Self::new(self.width, self.hyperlinks);
        let hint = TextPages::new(
            &format!(
                "... {} rows hidden · Ctrl+O expand",
                hidden_end - hidden_start
            ),
            self.width,
        )
        .map_err(TranscriptRenderError::Text)?;
        if hint.row_count() >= hidden_end - hidden_start {
            return Ok(self);
        }
        let original_height = self.height;
        let hint_role = self
            .sections
            .iter()
            .find(|(_, _, role)| matches!(role, GlyphRole::ActivityBody | GlyphRole::ReasoningBody))
            .map_or(GlyphRole::ActivityBody, |(_, _, role)| *role);
        let original = std::sync::Arc::new(self);
        folded.append(
            Section::Rows(original.clone(), 0, hidden_start),
            GlyphRole::ActivityBody,
        )?;
        folded.append(Section::Literal(hint, Vec::new()), hint_role)?;
        folded.append(
            Section::Rows(original, hidden_end, original_height - hidden_end),
            GlyphRole::ActivityBody,
        )?;
        Ok(folded)
    }
}

pub(super) fn prepare(
    config: &TranscriptLayoutConfig,
    message: &TranscriptMessage,
    width: NonZeroU16,
    format_markdown: bool,
    finalized: bool,
) -> Result<Option<PagedBody>, TranscriptRenderError> {
    let mut body = PagedBody::new(width, config.hyperlinks);
    if !format_markdown || message.role() == MessageRole::User {
        let header_end = message.text().find('\n').unwrap_or(message.text().len());
        if let Some(activity) = message
            .activity
            .filter(|a| a.kind != Some(ActivityKind::ModelWork))
        {
            let footer_start = activity.footer_start.unwrap_or(message.text().len());
            let pages =
                TextPages::new(message.text(), width).map_err(TranscriptRenderError::Text)?;
            let roles = vec![
                (0, GlyphRole::ActivityHeading(activity.outcome)),
                (header_end, GlyphRole::ActivityBody),
                (
                    footer_start.max(header_end),
                    GlyphRole::ActivityOutcome(activity.outcome),
                ),
            ];
            body.append(Section::Literal(pages, roles), GlyphRole::ActivityBody)?;
        } else {
            body.literal(message.text(), body_role(message.role()), false)?;
        }
        return Ok(Some(body));
    }
    if message.activity.is_none() && message.is_markdown() {
        let end = message
            .assistant_footer_start
            .unwrap_or(message.text().len());
        let source = &message.text()[..end];
        let content = MessageContent::from_snapshot(source)
            .map(|c| activity::content_block_markdown(&c.block, true));
        let source = content.as_deref().unwrap_or(source);
        let replacement = config.assistant_renderer.as_ref().and_then(|renderer| {
            renderer.render(AssistantRenderInput {
                source: &message.text()[..end],
                columns: width,
                finalized,
            })
        });
        if let Some(replacement) = replacement {
            body.markdown(&replacement, GlyphRole::AssistantBody, config)
                .or_else(|_| body.markdown(source, GlyphRole::AssistantBody, config))?;
        } else {
            body.markdown(source, GlyphRole::AssistantBody, config)?;
        }
        if end < message.text().len() {
            body.height = body
                .height
                .checked_add(usize::from(body.height > 0))
                .ok_or(TranscriptRenderError::HeightOverflow)?;
            body.literal(
                message.text()[end..].trim_start_matches('\n'),
                GlyphRole::AssistantBody,
                true,
            )?;
        }
        return Ok(Some(body));
    }
    if let Some(activity) = message
        .activity
        .filter(|activity| activity.kind == Some(ActivityKind::ModelWork))
    {
        let end = activity.footer_start.unwrap_or(message.text().len());
        if let Some((_, source)) = message.text()[..end].split_once('\n') {
            if let Some(plan) = ActivityPlan::from_snapshot(source) {
                body.literal(
                    &activity::plan_heading(&plan),
                    GlyphRole::NoticeHeading(NoticeLevel::Info),
                    false,
                )?;
                if let Some(explanation) = plan.explanation {
                    body.literal(&explanation, GlyphRole::ActivityBody, false)?;
                }
                if plan.steps.is_empty() {
                    body.literal("No steps provided.", GlyphRole::ActivityBody, false)?;
                }
                for step in plan.steps {
                    let marker = activity::plan_marker(step.status);
                    let role = GlyphRole::PlanStep(step.status);
                    if let Some(inner) = width
                        .get()
                        .checked_sub(4)
                        .and_then(NonZeroU16::new)
                        .filter(|n| n.get() >= 2)
                    {
                        body.append(
                            Section::PlanStep(
                                TextPages::new(&step.text, inner)
                                    .map_err(TranscriptRenderError::Text)?,
                                marker,
                                inner,
                            ),
                            role,
                        )?;
                    } else {
                        body.literal(&format!("{marker}{}", step.text), role, false)?;
                    }
                }
                body.literal(
                    message.text()[end..].trim_start_matches('\n'),
                    GlyphRole::ActivityOutcome(activity.outcome),
                    false,
                )?;
                return Ok(Some(body));
            }
            if let Some(notice) = ActivityNotice::from_snapshot(source) {
                body.literal(&notice.title, GlyphRole::NoticeHeading(notice.level), true)?;
                body.literal(&notice.message, GlyphRole::ActivityBody, true)?;
                body.literal(
                    message.text()[end..].trim_start_matches('\n'),
                    GlyphRole::ActivityOutcome(activity.outcome),
                    true,
                )?;
                return Ok(Some(body));
            }
        }
        if let Some((_, source)) = message.text()[..end].split_once('\n')
            && let Some(document) = activity::DocumentSource::parse(source)
        {
            body.literal(
                &document.heading,
                GlyphRole::NoticeHeading(NoticeLevel::Info),
                false,
            )?;
            if let Some(tokens) = document.tokens_before {
                body.literal(
                    &format!("Before compaction: {tokens} tokens"),
                    GlyphRole::NoticeHeading(NoticeLevel::Info),
                    false,
                )?;
            }
            let role = if document.reasoning {
                GlyphRole::ReasoningBody
            } else {
                GlyphRole::ActivityBody
            };
            let replacement = config.document_renderer.as_ref().and_then(|renderer| {
                let input = ActivityDocument::from_snapshot(source).or_else(|| {
                    ActivityReasoning::from_snapshot(source).map(|_| ActivityDocument {
                        title: document.heading.clone(),
                        markdown: document.content.clone(),
                    })
                })?;
                renderer
                    .render(DocumentRenderInput {
                        document: &input,
                        outcome: activity.outcome,
                        expanded: !config.compact_activities(),
                        columns: width,
                    })
                    .filter(|text| text.len() <= ToolOutput::MAX_SNAPSHOT_BYTES)
            });
            if document.reasoning && !config.show_reasoning {
                body.literal(document.hidden_hint, role, false)?;
            } else if document.content.is_empty() && replacement.is_none() {
                body.literal(document.empty_hint, role, false)?;
            } else {
                let mut config = config.clone();
                config.show_images = false;
                config.show_diagrams &= !document.reasoning;
                body.markdown(
                    replacement.as_deref().unwrap_or(&document.content),
                    role,
                    &config,
                )
                .or_else(|_| body.markdown(&document.content, role, &config))
                .or_else(|_| body.literal(&document.content, role, false))?;
            }
            body.literal(
                message.text()[end..].trim_start_matches('\n'),
                GlyphRole::ActivityOutcome(activity.outcome),
                false,
            )?;
            if config.compact_activities() {
                body = body.compact(config.activity_head_rows(false))?;
            }
            return Ok(Some(body));
        }
    }
    if !config.compact_activities() && message.activity.is_some_and(|activity| activity.diff) {
        let activity = message.activity.expect("diff activity");
        let text = message.text();
        let header_end = text.find('\n').unwrap_or(text.len());
        let footer_start = activity.footer_start.unwrap_or(text.len());
        let source = text[header_end.min(footer_start)..footer_start]
            .strip_prefix('\n')
            .unwrap_or("");
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
        body.literal(
            &heading,
            GlyphRole::ActivityHeading(activity.outcome),
            false,
        )?;
        if !source.is_empty() {
            body.append(
                Section::Markdown(
                    markdown::PagedMarkdown::diff(source, width, config.code_padding)
                        .map_err(TranscriptRenderError::Text)?,
                ),
                GlyphRole::ActivityBody,
            )?;
        }
        body.literal(
            text[footer_start..].trim_start_matches('\n'),
            GlyphRole::ActivityOutcome(activity.outcome),
            false,
        )?;
        return Ok(Some(body));
    }
    if let Some((heading, source, footer, rendered, skip_fold)) =
        activity::tool_presentation(config, message, width, true)
    {
        let activity = message.activity.expect("tool presentation has activity");
        body.literal(
            &heading,
            GlyphRole::ActivityHeading(activity.outcome),
            false,
        )?;
        if let Some(rendered) = rendered {
            body.markdown(&rendered, GlyphRole::ActivityBody, config)
                .or_else(|_| body.literal(&source, GlyphRole::ActivityBody, false))?;
        } else {
            body.literal(&source, GlyphRole::ActivityBody, false)?;
        }
        body.literal(&footer, GlyphRole::ActivityOutcome(activity.outcome), false)?;
        if config.compact_activities() && !skip_fold {
            body = body.compact(config.activity_head_rows(false))?;
        }
        return Ok(Some(body));
    }
    if message.activity.is_none() {
        body.literal(message.text(), body_role(message.role()), false)?;
        return Ok(Some(body));
    }
    if let Some(activity) = message.activity.filter(|activity| {
        matches!(
            activity.kind,
            Some(
                ActivityKind::UserInputResponse { .. }
                    | ActivityKind::ApprovalResponse { .. }
                    | ActivityKind::UserInputRequest { .. }
                    | ActivityKind::ApprovalRequest { .. }
            )
        )
    }) {
        let end = activity.footer_start.unwrap_or(message.text().len());
        let (heading, source) = message.text()[..end]
            .split_once('\n')
            .unwrap_or((&message.text()[..end], ""));
        body.literal(heading, GlyphRole::ActivityHeading(activity.outcome), true)?;
        body.literal(source, GlyphRole::ActivityBody, true)?;
        body.literal(
            message.text()[end..].trim_start_matches('\n'),
            GlyphRole::ActivityOutcome(activity.outcome),
            true,
        )?;
        return Ok(Some(body));
    }
    if let Some(activity) = message
        .activity
        .filter(|activity| !activity.diff && !activity.usage && !message.is_markdown())
    {
        let header_end = message.text().find('\n').unwrap_or(message.text().len());
        let footer_start = activity.footer_start.unwrap_or(message.text().len());
        let pages = TextPages::new(message.text(), width).map_err(TranscriptRenderError::Text)?;
        let roles = vec![
            (0, GlyphRole::ActivityHeading(activity.outcome)),
            (header_end, GlyphRole::ActivityBody),
            (
                footer_start.max(header_end),
                GlyphRole::ActivityOutcome(activity.outcome),
            ),
        ];
        body.append(Section::Literal(pages, roles), GlyphRole::ActivityBody)?;
        if config.compact_activities() && activity.kind != Some(ActivityKind::ModelWork) {
            let start = TextPages::new(&message.text()[..header_end], width)
                .map_err(TranscriptRenderError::Text)?
                .row_count();
            let footer = TextPages::new(
                message.text()[footer_start..].trim_start_matches('\n'),
                width,
            )
            .map_err(TranscriptRenderError::Text)?
            .row_count();
            let end = body.height.saturating_sub(footer);
            body = body.compact_range(start, end, config.activity_head_rows(false))?;
        }
        return Ok(Some(body));
    }
    Ok(None)
}
