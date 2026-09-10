//! Read-only file sections from typed, retained Chat change activities.

use std::{cell::RefCell, iter::once, num::NonZeroU16, ops::Range, sync::Arc};

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    surface::{Point, Rect, Size, SurfaceView, WriteOutcome, cell_width},
    text::flow::{TextFlowError, TextPages, flow_text},
    transcript::{
        FileChangeView, TranscriptActivityOutcome, TranscriptBody, TranscriptItemId,
        TranscriptScrollCommand, TranscriptSlice, TranscriptStyles,
    },
};

pub(super) fn header(
    selected: usize,
    count: usize,
    width: u16,
    section: Option<Section<'_>>,
) -> String {
    let index = if count == 0 { 0 } else { selected + 1 };
    let path = section
        .and_then(|section| section.change.body.lines().next())
        .and_then(|line| {
            ["add: ", "update: ", "delete: "]
                .into_iter()
                .find_map(|prefix| line.strip_prefix(prefix))
        })
        .filter(|path| !path.is_empty() && !path.chars().any(char::is_control));
    if let Some((path, path_width)) =
        path.and_then(|path| cell_width(path).ok().map(|width| (path, width)))
    {
        let columns = usize::from(width);
        for (prefix, suffix) in [
            (
                format!("Changes {index}/{count} | "),
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
        format!("Changes {index}/{count} | Left/Right files | F1 Chat"),
        format!("Changes {index}/{count} | <> | F1 Chat"),
        format!("Changes {index}/{count} F1 Chat"),
        format!("Changes {index}/{count}"),
        "Changes".to_owned(),
        "D".to_owned(),
    ]
    .into_iter()
    .find(|text| text.len() <= usize::from(width))
    .unwrap_or_default()
}

#[derive(Clone, Copy)]
pub(super) struct Section<'a> {
    pub(super) key: (TranscriptItemId, usize),
    pub(super) revision: u64,
    pub(super) change: FileChangeView<'a>,
}

pub(super) fn sections(chat: TranscriptSlice<'_>) -> Vec<Section<'_>> {
    let mut sections = Vec::new();
    for item in chat.items() {
        let TranscriptBody::Message(message) = item.body();
        let Some(change) = message.file_change() else {
            continue;
        };
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
                    change: FileChangeView {
                        body: &change.body[start..offset],
                        ..change
                    },
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
            change: FileChangeView {
                body: &change.body[start..],
                ..change
            },
        });
    }
    sections
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
    body: Range<usize>,
    label: Range<usize>,
    outcome: Option<TranscriptActivityOutcome>,
    pages: TextPages,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ChangesView {
    cache: RefCell<Option<Arc<Cache>>>,
}

impl ChangesView {
    pub(super) fn render(
        &self,
        selected: Option<Section<'_>>,
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
                let (source, body, label, outcome) = if let Some(section) = selected {
                    let change = section.change;
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
                    if let Some(footer) = change.footer.filter(|footer| !footer.is_empty()) {
                        source.push('\n');
                        source.push_str(footer);
                    }
                    (source, start..end, label_start..start, change.outcome)
                } else {
                    (
                        "No file changes in this conversation yet.\nF1 returns to Chat.".to_owned(),
                        0..0,
                        0..0,
                        None,
                    )
                };
                let pages = TextPages::with_escaped_fallback(
                    &source,
                    columns,
                    "Escaped diff (unrenderable cells)",
                )?;
                let lines = once(0)
                    .chain(source.match_indices('\n').map(|(offset, _)| offset + 1))
                    .collect();
                *cached = Some(Arc::new(Cache {
                    key,
                    width: columns,
                    source,
                    lines,
                    body,
                    label,
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
                TranscriptScrollCommand::JumpToStart | TranscriptScrollCommand::PreviousItem => 0,
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
            let in_body = cache.body.contains(&start);
            let style = if in_body {
                styles.markdown.diff_line_style(&cache.source[start..end])
            } else if cache.label.contains(&start) {
                styles.markdown.code_label
            } else {
                styles.activity.status(cache.outcome)
            };
            if in_body || cache.label.contains(&start) {
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
            change: FileChangeView {
                heading: "Changes",
                body,
                outcome: None,
                footer: None,
            },
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
                Some(section),
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
            &escaped.source[escaped.body.clone()],
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
            change: FileChangeView {
                heading: "Changes",
                body: &body,
                outcome: None,
                footer: None,
            },
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
                Some(section),
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
