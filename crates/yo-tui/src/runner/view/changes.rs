//! Read-only file sections from typed, retained Chat change activities.

use std::{
    cell::RefCell, collections::HashMap, iter::once, num::NonZeroU16, ops::Range, sync::Arc,
};

use unicode_segmentation::UnicodeSegmentation;
use yo_core::{ActivityKind, ToolOutput};

use crate::{
    surface::{Point, Rect, Size, SurfaceView, WriteOutcome, cell_width},
    text::flow::{TextFlowError, TextPages, flow_text},
    transcript::{
        FileChangeView, FilePublicationPresentation, LocalFileProposal, LocalFileProposalBodyStyle,
        TranscriptActivityOutcome, TranscriptBody, TranscriptItemId, TranscriptMessage,
        TranscriptScrollCommand, TranscriptSlice, TranscriptStyles, completed_file_publication,
        file_publication_presentation, local_file_proposal, local_file_proposal_path,
    },
};

pub(super) fn header(
    selected: usize,
    count: usize,
    width: u16,
    section: Option<&Section<'_>>,
) -> String {
    let index = if count == 0 { 0 } else { selected + 1 };
    let label = section.map_or("Changes", Section::header_label);
    let path = section
        .and_then(Section::path)
        .filter(|path| !path.is_empty() && !path.chars().any(char::is_control));
    if let Some((path, path_width)) =
        path.and_then(|path| cell_width(path).ok().map(|width| (path, width)))
    {
        let columns = usize::from(width);
        for (prefix, suffix) in [
            (
                format!("{label} {index}/{count} | "),
                " | Left/Right files | F1 Chat",
            ),
            (format!("{index}/{count} "), " F1"),
            (format!("{index}/{count} "), ""),
        ] {
            if prefix.len() + path_width + suffix.len() <= columns {
                return format!("{prefix}{path}{suffix}");
            }
        }
        let prefix = format!("{index}/{count} …");
        let prefix_width =
            format!("{index}/{count} ").len() + cell_width("…").expect("ellipsis is renderable");
        if prefix_width < columns {
            let mut remaining = columns - prefix_width;
            let mut tail = Vec::new();
            for grapheme in path.graphemes(true).rev() {
                let cells = cell_width(grapheme).expect("path width was validated above");
                if cells > remaining {
                    break;
                }
                tail.push(grapheme);
                remaining -= cells;
            }
            return format!("{prefix}{}", tail.into_iter().rev().collect::<String>());
        }
    }
    [
        format!("{label} {index}/{count} | Left/Right files | F1 Chat"),
        format!("{label} {index}/{count} | <> | F1 Chat"),
        format!("{label} {index}/{count} F1 Chat"),
        format!("{label} {index}/{count}"),
        label.to_owned(),
        "D".to_owned(),
    ]
    .into_iter()
    .find(|text| text.len() <= usize::from(width))
    .unwrap_or_default()
}

#[derive(Clone)]
pub(super) struct Section<'a> {
    pub(super) key: (TranscriptItemId, usize),
    pub(super) revision: u64,
    source: SectionSource<'a>,
}

#[derive(Clone)]
enum SectionSource<'a> {
    Reported(FileChangeView<'a>),
    Proposal {
        message: &'a TranscriptMessage,
        path: String,
    },
    Publication {
        message: &'a TranscriptMessage,
        presentation: Arc<FilePublicationPresentation>,
    },
}

impl Section<'_> {
    fn header_label(&self) -> &'static str {
        match &self.source {
            SectionSource::Reported(_) => "Changes",
            SectionSource::Proposal { .. } => "Proposed",
            SectionSource::Publication { .. } => "Recorded",
        }
    }

    fn path(&self) -> Option<&str> {
        match &self.source {
            SectionSource::Reported(change) => change.body.lines().next().and_then(|line| {
                ["add: ", "update: ", "delete: "]
                    .into_iter()
                    .find_map(|prefix| line.strip_prefix(prefix))
            }),
            SectionSource::Proposal { path, .. } => Some(path),
            SectionSource::Publication { presentation, .. } => Some(&presentation.path),
        }
    }
}

pub(super) fn can_open(message: &TranscriptMessage) -> bool {
    message
        .file_change()
        .is_some_and(|change| !change.body.is_empty())
        || has_publication(message)
        || proposal_path(message).is_some()
}

fn proposal_path(message: &TranscriptMessage) -> Option<String> {
    if !matches!(
        message.tool_kind(),
        Some(ActivityKind::ToolCall | ActivityKind::ToolResult)
    ) {
        return None;
    }
    let output = ToolOutput::from_snapshot(message.tool_source()?)?;
    local_file_proposal_path(&output).map(str::to_owned)
}

fn has_publication(message: &TranscriptMessage) -> bool {
    message
        .tool_source()
        .and_then(ToolOutput::from_snapshot)
        .is_some_and(|output| {
            completed_file_publication(&output, message.tool_kind(), message.tool_outcome())
                .is_some()
        })
}

#[derive(Clone, Debug, Default)]
struct ObservationIndex {
    signature: Vec<(TranscriptItemId, u64)>,
    observations: Arc<HashMap<TranscriptItemId, IndexedObservation>>,
}

#[derive(Clone, Debug)]
struct IndexedObservation {
    revision: u64,
    proposal_path: Option<String>,
    publication: Option<Arc<FilePublicationPresentation>>,
}

struct SourceParts {
    source: String,
    bodies: Vec<(Range<usize>, LocalFileProposalBodyStyle)>,
    labels: Vec<Range<usize>>,
    outcome: Option<TranscriptActivityOutcome>,
}

fn is_file_header(line: &str) -> bool {
    ["add: ", "update: ", "delete: "]
        .iter()
        .any(|prefix| line.starts_with(prefix))
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Position {
    key: Option<(TranscriptItemId, usize)>,
    revision: Option<u64>,
    width: Option<NonZeroU16>,
    row: usize,
    anchor: Option<usize>,
    follow_tail: bool,
}

#[derive(Clone, Debug)]
struct Cache {
    key: Option<(TranscriptItemId, usize, u64)>,
    width: NonZeroU16,
    source: String,
    lines: Vec<usize>,
    bodies: Vec<(Range<usize>, LocalFileProposalBodyStyle)>,
    labels: Vec<Range<usize>>,
    outcome: Option<TranscriptActivityOutcome>,
    pages: TextPages,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ChangesView {
    cache: RefCell<Option<Arc<Cache>>>,
    observation_index: RefCell<Option<ObservationIndex>>,
}

impl ChangesView {
    pub(super) fn sections<'a>(&self, chat: TranscriptSlice<'a>) -> Vec<Section<'a>> {
        let observations = self.observations(chat);
        let mut sections = Vec::new();
        for item in chat.items() {
            let TranscriptBody::Message(message) = item.body();
            if let Some(change) = message.file_change() {
                let mut start = 0;
                let mut offset = 0;
                let mut index = 0;
                let explicit_files = change.body.lines().any(is_file_header);
                let mut saw_file = false;
                for line in change.body.split_inclusive('\n') {
                    let file_header = if explicit_files {
                        is_file_header(line)
                    } else {
                        line.starts_with("diff --git ")
                    };
                    if file_header && saw_file && offset > start {
                        sections.push(Section {
                            key: (item.id(), index),
                            revision: item.revision(),
                            source: SectionSource::Reported(FileChangeView {
                                body: &change.body[start..offset],
                                ..change
                            }),
                        });
                        start = offset;
                        index += 1;
                    }
                    saw_file |= file_header;
                    offset += line.len();
                }
                sections.push(Section {
                    key: (item.id(), index),
                    revision: item.revision(),
                    source: SectionSource::Reported(FileChangeView {
                        body: &change.body[start..],
                        ..change
                    }),
                });
                continue;
            }
            if let Some(observation) = observations
                .get(&item.id())
                .filter(|observation| observation.revision == item.revision())
            {
                if let Some(presentation) = &observation.publication {
                    sections.push(Section {
                        key: (item.id(), 0),
                        revision: item.revision(),
                        source: SectionSource::Publication {
                            message,
                            presentation: Arc::clone(presentation),
                        },
                    });
                    continue;
                }
                if let Some(path) = &observation.proposal_path {
                    sections.push(Section {
                        key: (item.id(), 0),
                        revision: item.revision(),
                        source: SectionSource::Proposal {
                            message,
                            path: path.clone(),
                        },
                    });
                }
            }
        }
        sections
    }

    fn observations(
        &self,
        chat: TranscriptSlice<'_>,
    ) -> Arc<HashMap<TranscriptItemId, IndexedObservation>> {
        let signature = chat
            .items()
            .iter()
            .map(|item| (item.id(), item.revision()))
            .collect::<Vec<_>>();
        let mut cached = self.observation_index.borrow_mut();
        if cached
            .as_ref()
            .is_none_or(|index| index.signature != signature)
        {
            let previous = cached.take().unwrap_or_default();
            let mut observations = HashMap::new();
            for item in chat.items() {
                if let Some(observation) = previous.observations.get(&item.id())
                    && observation.revision == item.revision()
                {
                    observations.insert(item.id(), observation.clone());
                    continue;
                }
                let TranscriptBody::Message(message) = item.body();
                let output = message.tool_source().and_then(ToolOutput::from_snapshot);
                let proposal_path = output
                    .as_ref()
                    .and_then(local_file_proposal_path)
                    .map(str::to_owned);
                let publication = output.as_ref().and_then(|output| {
                    let evidence = completed_file_publication(
                        output,
                        message.tool_kind(),
                        message.tool_outcome(),
                    )?;
                    Some(Arc::new(file_publication_presentation(&evidence)))
                });
                observations.insert(
                    item.id(),
                    IndexedObservation {
                        revision: item.revision(),
                        proposal_path,
                        publication,
                    },
                );
            }
            *cached = Some(ObservationIndex {
                signature,
                observations: Arc::new(observations),
            });
        }
        Arc::clone(
            &cached
                .as_ref()
                .expect("observation index was populated")
                .observations,
        )
    }

    pub(super) fn render(
        &self,
        selected: Option<&Section<'_>>,
        view: &mut SurfaceView<'_>,
        width: NonZeroU16,
        styles: TranscriptStyles,
        position: &mut Position,
        commands: &[TranscriptScrollCommand],
    ) -> Result<(), TextFlowError> {
        let padding = u16::from(width.get() >= 6);
        let columns =
            NonZeroU16::new(width.get() - padding * 2).expect("padding preserves body cells");
        let key = selected.map(|section| (section.key.0, section.key.1, section.revision));
        let cache = {
            let mut cached = self.cache.borrow_mut();
            if cached
                .as_ref()
                .is_none_or(|cache| cache.key != key || cache.width != columns)
            {
                let SourceParts {
                    source,
                    bodies,
                    labels,
                    outcome,
                } = if let Some(section) = selected {
                    match &section.source {
                        SectionSource::Reported(change) => {
                            let added = change
                                .body
                                .lines()
                                .filter(|line| line.starts_with('+') && !line.starts_with("+++ "))
                                .count();
                            let removed = change
                                .body
                                .lines()
                                .filter(|line| line.starts_with('-') && !line.starts_with("--- "))
                                .count();
                            let mut source = format!("{} · +{added} -{removed}\n", change.heading);
                            let label_start = source.len();
                            source.push_str("diff\n");
                            let start = source.len();
                            source.push_str(change.body.strip_suffix('\n').unwrap_or(change.body));
                            let end = source.len();
                            if let Some(footer) = change.footer.filter(|footer| !footer.is_empty())
                            {
                                source.push('\n');
                                source.push_str(footer);
                            }
                            SourceParts {
                                source,
                                bodies: vec![(start..end, LocalFileProposalBodyStyle::Diff)],
                                labels: once(label_start..start).collect(),
                                outcome: change.outcome,
                            }
                        },
                        SectionSource::Proposal { message, path } => {
                            let output = ToolOutput::from_snapshot(
                                message
                                    .tool_source()
                                    .expect("a proposed section retains its tool source"),
                            )
                            .expect("a proposed section retains a complete tool output");
                            let proposal = local_file_proposal(&output)
                                .expect("a proposed section retains its admitted arguments");
                            debug_assert_eq!(proposal.path, path.as_str());
                            proposal_source(message, &output, &proposal)
                        },
                        SectionSource::Publication {
                            message,
                            presentation,
                        } => {
                            let output = ToolOutput::from_snapshot(
                                message
                                    .tool_source()
                                    .expect("a publication section retains its tool source"),
                            )
                            .expect("a publication section retains a complete tool output");
                            publication_source(message, &output, presentation)
                        },
                    }
                } else {
                    SourceParts {
                        source: "No file changes were reported in this conversation.\nThis view does not inspect the Git worktree.\nF1 returns to Chat.".to_owned(),
                        bodies: Vec::new(),
                        labels: Vec::new(),
                        outcome: None,
                    }
                };
                let fallback_style = bodies
                    .first()
                    .map_or(LocalFileProposalBodyStyle::Diff, |(_, style)| *style);
                let pages = TextPages::with_escaped_fallback(
                    &source,
                    columns,
                    match fallback_style {
                        LocalFileProposalBodyStyle::Diff => "Escaped diff (unrenderable cells)",
                        LocalFileProposalBodyStyle::Plain => {
                            "Escaped proposed content (unrenderable cells)"
                        },
                    },
                )?;
                let lines = once(0)
                    .chain(source.match_indices('\n').map(|(offset, _)| offset + 1))
                    .collect();
                *cached = Some(Arc::new(Cache {
                    key,
                    width: columns,
                    source,
                    lines,
                    bodies,
                    labels,
                    outcome,
                    pages,
                }));
            }
            Arc::clone(cached.as_ref().expect("selected diff was cached"))
        };
        let selected_key = selected.map(|section| section.key);
        if position.key != selected_key {
            *position = Position {
                key: selected_key,
                ..Position::default()
            };
        }
        let revision = selected.map(|section| section.revision);
        if (position.width != Some(columns) || position.revision != revision)
            && let Some(anchor) = position.anchor
        {
            position.row = cache.pages.row_for_source(anchor);
        }
        position.width = Some(columns);
        position.revision = revision;
        let height = NonZeroU16::new(view.size().height).expect("diff body is nonempty");
        let last = cache
            .pages
            .row_count()
            .saturating_sub(usize::from(height.get()));
        position.row = if position.follow_tail {
            last
        } else {
            position.row.min(last)
        };
        for command in commands {
            position.row = match command {
                TranscriptScrollCommand::LineUp => position.row.saturating_sub(1),
                TranscriptScrollCommand::LineDown => position.row.saturating_add(1).min(last),
                TranscriptScrollCommand::PageUp => {
                    position.row.saturating_sub(usize::from(height.get()))
                },
                TranscriptScrollCommand::PageDown => position
                    .row
                    .saturating_add(usize::from(height.get()))
                    .min(last),
                TranscriptScrollCommand::JumpToStart
                | TranscriptScrollCommand::PreviousItem
                | TranscriptScrollCommand::JumpToItem(_) => 0,
                TranscriptScrollCommand::JumpToTail | TranscriptScrollCommand::NextItem => last,
            };
            position.follow_tail = matches!(command, TranscriptScrollCommand::JumpToTail)
                || (position.follow_tail
                    && matches!(
                        command,
                        TranscriptScrollCommand::LineDown | TranscriptScrollCommand::PageDown
                    ));
        }
        if position.anchor.is_none() || !commands.is_empty() || position.follow_tail {
            position.anchor = Some(cache.pages.source_offset(position.row));
        }
        view.clear(styles.background);
        let page = flow_text(cache.pages.window(position.row, height), columns)?;
        let mut row_styles = Vec::new();
        for row in 0..page.height {
            let byte = cache.pages.source_offset(position.row + usize::from(row));
            let line_index = cache
                .lines
                .partition_point(|offset| *offset <= byte)
                .saturating_sub(1);
            let start = cache.lines[line_index];
            let end = cache
                .lines
                .get(line_index + 1)
                .copied()
                .unwrap_or(cache.source.len());
            let body_style = cache
                .bodies
                .iter()
                .find_map(|(range, style)| range.contains(&start).then_some(*style));
            let in_label = cache.labels.iter().any(|range| range.contains(&start));
            let style = if let Some(body_style) = body_style {
                match body_style {
                    LocalFileProposalBodyStyle::Diff => {
                        styles.markdown.diff_line_style(&cache.source[start..end])
                    },
                    LocalFileProposalBodyStyle::Plain => styles.activity.body,
                }
            } else if in_label {
                styles.markdown.code_label
            } else {
                styles.activity.status(cache.outcome)
            };
            if body_style.is_some() || in_label {
                view.subview(Rect::new(Point::new(0, row), Size::new(width.get(), 1)))
                    .expect("diff row fits body")
                    .clear(style);
            }
            row_styles.push(style);
        }
        for glyph in page.glyphs {
            let style = row_styles[usize::from(glyph.point.y)];
            let point = Point::new(glyph.point.x + padding, glyph.point.y);
            let outcome = view.write(point, glyph.grapheme, style);
            debug_assert_ne!(outcome, WriteOutcome::Clipped);
        }
        Ok(())
    }
}

fn proposal_source(
    message: &TranscriptMessage,
    output: &ToolOutput,
    proposal: &LocalFileProposal<'_>,
) -> SourceParts {
    let kind = message
        .tool_kind()
        .expect("a proposed section is a typed tool observation");
    let outcome = message.tool_outcome();
    let prepared = kind == ActivityKind::ToolCall
        && output.result.is_none()
        && output.content_items.is_none()
        && output.error.is_none();
    let mut source = format!(
        "{}\nPath: {}\nActivity outcome: {}\n",
        if prepared {
            "Prepared proposal"
        } else if kind == ActivityKind::ToolCall {
            "Tool call proposal"
        } else {
            "Tool result proposal"
        },
        proposal.path,
        activity_outcome_label(outcome),
    );
    let label_start = source.len();
    source.push_str(proposal.label());
    let label_end = source.len();
    source.push_str("\n\n");
    let body = proposal.append_detail_body(&mut source);
    if prepared {
        source.push_str("\n\nNo execution result in this observation.");
    } else {
        if let Some(result) = &output.result {
            source.push_str("\n\nRecorded result:\n");
            source.push_str(&result.to_string());
        } else {
            source.push_str("\n\nNo result was recorded in this observation.");
        }
        if let Some(content_items) = &output.content_items {
            source.push_str("\n\nRecorded content items:\n");
            source.push_str(&content_items.to_string());
        }
        if let Some(error) = &output.error {
            source.push_str("\n\nRecorded error:\n");
            source.push_str(&error.to_string());
        }
    }
    if let Some(detail) = message
        .tool_outcome_detail()
        .filter(|detail| !detail.is_empty())
    {
        source.push_str("\n\nRecorded activity detail:\n");
        source.push_str(detail);
    }
    SourceParts {
        source,
        bodies: vec![(body, proposal.body_style)],
        labels: once(label_start..label_end).collect(),
        outcome,
    }
}

fn publication_source(
    message: &TranscriptMessage,
    output: &ToolOutput,
    presentation: &FilePublicationPresentation,
) -> SourceParts {
    let outcome = message.tool_outcome();
    let mut source = format!(
        "{}\nPath: {}\nActivity outcome: {}\n{}\n",
        presentation.heading,
        presentation.path,
        activity_outcome_label(outcome),
        presentation.provenance,
    );
    let publication_label_start = source.len();
    source.push_str(match presentation.body_style {
        LocalFileProposalBodyStyle::Diff => "Read before edit → written by edit",
        LocalFileProposalBodyStyle::Plain => "Unavailable reason",
    });
    let publication_label_end = source.len();
    source.push_str("\n\n");
    let publication_body_start = source.len();
    source.push_str(&presentation.body);
    let publication_body_end = source.len();
    let mut bodies = vec![(
        publication_body_start..publication_body_end,
        presentation.body_style,
    )];
    let mut labels: Vec<Range<usize>> =
        once(publication_label_start..publication_label_end).collect();

    if let Some(proposal) = local_file_proposal(output) {
        source.push_str("\n\n");
        let label_start = source.len();
        source.push_str(proposal.label());
        let label_end = source.len();
        source.push_str("\n\n");
        bodies.push((
            proposal.append_detail_body(&mut source),
            proposal.body_style,
        ));
        labels.push(label_start..label_end);
    }

    if let Some(result) = &output.result {
        let mut displayed = result.clone();
        if let Some(fields) = displayed.as_object_mut() {
            fields.remove("publicationEvidence");
        }
        source.push_str("\n\nRecorded result:\n");
        append_bounded_detail(&mut source, &displayed.to_string(), 16 * 1024);
    }
    if let Some(content_items) = &output.content_items {
        source.push_str("\n\nRecorded content items:\n");
        append_bounded_detail(&mut source, &content_items.to_string(), 16 * 1024);
    }
    if let Some(error) = &output.error {
        source.push_str("\n\nRecorded error:\n");
        append_bounded_detail(&mut source, &error.to_string(), 16 * 1024);
    }
    if let Some(detail) = message
        .tool_outcome_detail()
        .filter(|detail| !detail.is_empty())
    {
        source.push_str("\n\nRecorded activity detail:\n");
        append_bounded_detail(&mut source, detail, 16 * 1024);
    }

    SourceParts {
        source,
        bodies,
        labels,
        outcome,
    }
}

fn append_bounded_detail(source: &mut String, detail: &str, maximum: usize) {
    if detail.len() <= maximum {
        source.push_str(detail);
        return;
    }
    let end = detail
        .grapheme_indices(true)
        .take_while(|(start, grapheme)| start + grapheme.len() <= maximum)
        .map(|(start, grapheme)| start + grapheme.len())
        .last()
        .unwrap_or(0);
    source.push_str(&detail[..end]);
    source.push_str("\n… recorded detail omitted after 16 KiB …");
}

fn activity_outcome_label(outcome: Option<TranscriptActivityOutcome>) -> &'static str {
    match outcome {
        None => "Pending",
        Some(TranscriptActivityOutcome::Completed) => "Completed",
        Some(TranscriptActivityOutcome::Failed) => "Failed",
        Some(TranscriptActivityOutcome::Interrupted) => "Interrupted",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        appearance::AppearanceState,
        surface::{Attributes, Color, Surface},
    };

    // 한 칸 폭의 이스케이프 표시는 원문·diff 색상을 보존하고 폭 복원 시 읽던 위치로 돌아간다.
    #[test]
    fn escaped_diff_keeps_source_styles_and_resize_anchors() {
        let body = "+한글 e\u{301} 👩‍💻\n-removed\n+tail\n";
        let section = Section {
            key: (TranscriptItemId::new(1), 0),
            revision: 1,
            source: SectionSource::Reported(FileChangeView {
                heading: "Changes",
                body,
                outcome: None,
                footer: None,
            }),
        };
        let view = ChangesView::default();
        let mut position = Position::default();
        let styles = AppearanceState::default()
            .pin()
            .snapshot()
            .styles()
            .transcript;
        let mut render = |width, commands: &[TranscriptScrollCommand]| {
            let size = Size::new(width, 2);
            let mut surface = Surface::new(size).unwrap();
            view.render(
                Some(&section),
                &mut surface.view(Rect::new(Point::new(0, 0), size)).unwrap(),
                NonZeroU16::new(width).unwrap(),
                styles,
                &mut position,
                commands,
            )
            .unwrap();
            (
                surface,
                position,
                Arc::clone(view.cache.borrow().as_ref().unwrap()),
            )
        };
        let (wide, original, _) = render(
            24,
            &[
                TranscriptScrollCommand::LineDown,
                TranscriptScrollCommand::LineDown,
            ],
        );
        let (narrow, at, escaped) = render(1, &[]);
        assert_eq!(at.anchor, original.anchor);
        assert_eq!(
            &escaped.source[escaped.bodies[0].0.clone()],
            body.trim_end_matches('\n')
        );
        let shown = escaped
            .pages
            .window(0, NonZeroU16::new(u16::MAX).unwrap())
            .replace('\n', "");
        assert!(
            shown.starts_with("Escaped diff (unrenderable cells)"),
            "{shown}"
        );
        assert!(shown.contains("\\u{d55c}\\u{ae00}"), "{shown}");
        assert_eq!(
            narrow.cell(Point::new(0, 0)).unwrap().style(),
            styles.markdown.diff_added
        );
        let (restored, restored_at, _) = render(24, &[]);
        assert_eq!(restored, wide);
        assert_eq!(restored_at.anchor, original.anchor);
        render(1, &[]);
        let (_, inside_escape, _) = render(
            1,
            &[
                TranscriptScrollCommand::LineDown,
                TranscriptScrollCommand::LineDown,
            ],
        );
        assert_eq!(inside_escape.anchor, escaped.source.find('한'));
        let (restored, restored_at, _) = render(24, &[]);
        assert_eq!(restored, wide);
        assert_eq!(restored_at.anchor, inside_escape.anchor);
    }

    // 줄바꿈으로 + 기호가 없는 행에도 원래 추가 행의 배경·강조를 적용한다. 테마 변경은
    // 소스 캐시를 재사용하면서 즉시 반영하고, 폭 왕복은 읽던 원문 위치를 유지한다.
    #[test]
    fn wrapped_diff_rows_keep_semantic_styles_and_cached_source_positions() {
        let body = format!("+{}\n-removed\n@@ metadata\n", "x".repeat(100));
        let section = Section {
            key: (TranscriptItemId::new(1), 0),
            revision: 1,
            source: SectionSource::Reported(FileChangeView {
                heading: "Changes",
                body: &body,
                outcome: None,
                footer: None,
            }),
        };
        let view = ChangesView::default();
        let mut position = Position::default();
        let mut styles = AppearanceState::default()
            .pin()
            .snapshot()
            .styles()
            .transcript;
        let mut render = |width, styles, commands: &[TranscriptScrollCommand]| {
            let size = Size::new(width, 2);
            let mut surface = Surface::new(size).unwrap();
            view.render(
                Some(&section),
                &mut surface.view(Rect::new(Point::new(0, 0), size)).unwrap(),
                NonZeroU16::new(width).unwrap(),
                styles,
                &mut position,
                commands,
            )
            .unwrap();
            (
                surface,
                position,
                Arc::clone(view.cache.borrow().as_ref().unwrap()),
            )
        };
        let (_, _, first) = render(24, styles, &[]);
        let (wide, at, cached) = render(
            24,
            styles,
            &[
                TranscriptScrollCommand::LineDown,
                TranscriptScrollCommand::LineDown,
                TranscriptScrollCommand::LineDown,
            ],
        );
        assert!(Arc::ptr_eq(&first, &cached));
        for row in 0..2 {
            for column in [0, 1, 23] {
                assert_eq!(
                    wide.cell(Point::new(column, row)).unwrap().style(),
                    styles.markdown.diff_added
                );
            }
        }
        render(12, styles, &[]);
        let (restored, position, restored_cache) = render(24, styles, &[]);
        assert_eq!(position.row, at.row);
        assert_eq!(position.anchor, at.anchor);
        assert_eq!(restored, wide);
        styles.markdown.diff_added.foreground = Color::Indexed(123);
        styles.markdown.diff_added.attributes = Attributes::BOLD;
        let (changed, _, latest) = render(24, styles, &[]);
        assert!(Arc::ptr_eq(&restored_cache, &latest));
        assert_eq!(
            changed.cell(Point::new(1, 0)).unwrap().style(),
            styles.markdown.diff_added
        );
    }
}
