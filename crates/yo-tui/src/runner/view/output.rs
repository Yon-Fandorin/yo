//! Paged literal inspection of retained tool output, without attachment or execution I/O.

use std::{cell::RefCell, num::NonZeroU16, sync::Arc};

use yo_core::ToolOutput;

use crate::{
    surface::{Style, SurfaceView, WriteOutcome},
    text::flow::{TextFlowError, TextPages, flow_text},
    transcript::{TranscriptBody, TranscriptItemId, TranscriptScrollCommand, TranscriptSlice},
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Position {
    selected: usize,
    key: Option<TranscriptItemId>,
    count: usize,
    row: usize,
    rows: usize,
    follow_tail: bool,
    source_anchor: Option<usize>,
    width: Option<NonZeroU16>,
    revision: Option<u64>,
    truncated: bool,
}

impl Position {
    pub(super) fn latest() -> Self {
        Self {
            selected: usize::MAX,
            ..Self::default()
        }
    }

    pub(super) fn select(&mut self, previous: bool) -> bool {
        let selected = if previous {
            self.selected.saturating_sub(1)
        } else {
            self.selected
                .saturating_add(1)
                .min(self.count.saturating_sub(1))
        };
        if selected == self.selected {
            return false;
        }
        self.selected = selected;
        self.key = None;
        self.row = 0;
        self.source_anchor = None;
        self.follow_tail = false;
        true
    }

    pub(super) fn header(self, width: u16) -> String {
        let index = if self.count == 0 {
            0
        } else {
            self.selected + 1
        };
        let row = if self.rows == 0 { 0 } else { self.row + 1 };
        let label = if self.truncated {
            "Partial output"
        } else {
            "Output"
        };
        [
            format!(
                "{label} {index}/{} | row {row}/{} | Up/Down PgUp/PgDn | <> tools | F1 Chat",
                self.count, self.rows
            ),
            format!(
                "{label} {index}/{} | {row}/{} | <> tools | F1 Chat",
                self.count, self.rows
            ),
            if self.truncated {
                format!("Partial {index}/{} | F1 Chat", self.count)
            } else {
                format!("Output {index}/{} F1 Chat", self.count)
            },
            format!(
                "{} {index}/{}",
                if self.truncated { "Partial" } else { "Output" },
                self.count
            ),
            if self.truncated { "Partial" } else { "Output" }.to_owned(),
            if self.truncated { "!" } else { "O" }.to_owned(),
        ]
        .into_iter()
        .find(|text| text.len() <= usize::from(width))
        .unwrap_or_default()
    }
}

#[derive(Clone, Debug)]
struct Cache {
    key: Option<(TranscriptItemId, u64)>,
    width: NonZeroU16,
    pages: TextPages,
    truncated: bool,
}

#[derive(Clone, Debug, Default)]
pub(super) struct OutputView {
    cache: RefCell<Option<Arc<Cache>>>,
}

impl OutputView {
    pub(super) fn render(
        &self,
        chat: TranscriptSlice<'_>,
        view: &mut SurfaceView<'_>,
        width: NonZeroU16,
        style: Style,
        position: &mut Position,
        commands: &[TranscriptScrollCommand],
    ) -> Result<(), TextFlowError> {
        let tools = chat
            .items()
            .iter()
            .filter_map(|item| {
                let TranscriptBody::Message(message) = item.body();
                message.tool_source().map(|source| (item, source))
            })
            .collect::<Vec<_>>();
        position.count = tools.len();
        if let Some(index) = tools
            .iter()
            .position(|(item, _)| Some(item.id()) == position.key)
        {
            position.selected = index;
        }
        position.selected = position.selected.min(tools.len().saturating_sub(1));
        let selected = tools.get(position.selected);
        let selected_key = selected.map(|(item, _)| item.id());
        if position.key != selected_key {
            position.source_anchor = None;
            position.row = 0;
        }
        position.key = selected_key;
        let key = selected.map(|(item, _)| (item.id(), item.revision()));
        let cache = {
            let mut cached = self.cache.borrow_mut();
            if cached
                .as_ref()
                .is_none_or(|cache| cache.key != key || cache.width != width)
            {
                let source = selected.map_or("No retained tool output yet.", |(_, source)| *source);
                let output = ToolOutput::from_snapshot(source);
                // Only typed boolean observations establish omission. A path or words
                // in tool text do not prove either completeness or local availability.
                let truncated = output
                    .as_ref()
                    .and_then(|output| output.result.as_ref())
                    .is_some_and(|result| {
                        if let Some(truncated) = result
                            .get("retainedOutput")
                            .filter(|value| {
                                value.as_object().is_some_and(|fields| fields.len() == 1)
                            })
                            .and_then(|value| value.get("truncated"))
                            .and_then(|value| value.as_bool())
                        {
                            return truncated;
                        }
                        result.get("truncated").and_then(|value| value.as_bool()) == Some(true)
                            || result
                                .pointer("/details/truncation/truncated")
                                .and_then(|value| value.as_bool())
                                == Some(true)
                    });
                let source = output
                    .as_ref()
                    .map_or(source, |output| output.plain_text.as_str());
                *cached = Some(Arc::new(Cache {
                    key,
                    width,
                    pages: TextPages::with_escaped_fallback(
                        source,
                        width,
                        "Escaped output (unrenderable cells)",
                    )?,
                    truncated,
                }));
            }
            Arc::clone(cached.as_ref().expect("selected output was cached"))
        };
        position.truncated = cache.truncated;
        let revision = key.map(|(_, revision)| revision);
        if (position.width != Some(width) || position.revision != revision)
            && let Some(anchor) = position.source_anchor
        {
            position.row = cache.pages.row_for_source(anchor);
        }
        position.width = Some(width);
        position.revision = revision;
        position.rows = cache.pages.row_count();
        let height = NonZeroU16::new(view.size().height).expect("output body is nonempty");
        let last = position.rows.saturating_sub(usize::from(height.get()));
        position.row = if position.follow_tail {
            last
        } else {
            position.row.min(last)
        };
        for command in commands {
            match command {
                TranscriptScrollCommand::LineUp => position.row = position.row.saturating_sub(1),
                TranscriptScrollCommand::LineDown => {
                    position.row = position.row.saturating_add(1).min(last)
                },
                TranscriptScrollCommand::PageUp => {
                    position.row = position.row.saturating_sub(usize::from(height.get()))
                },
                TranscriptScrollCommand::PageDown => {
                    position.row = position
                        .row
                        .saturating_add(usize::from(height.get()))
                        .min(last)
                },
                TranscriptScrollCommand::JumpToStart | TranscriptScrollCommand::PreviousItem => {
                    position.row = 0
                },
                TranscriptScrollCommand::JumpToTail | TranscriptScrollCommand::NextItem => {
                    position.row = last
                },
            }
            position.follow_tail = matches!(command, TranscriptScrollCommand::JumpToTail)
                || (position.follow_tail
                    && matches!(
                        command,
                        TranscriptScrollCommand::LineDown | TranscriptScrollCommand::PageDown
                    ));
        }
        if position.source_anchor.is_none() || !commands.is_empty() || position.follow_tail {
            position.source_anchor = Some(cache.pages.source_offset(position.row));
        }
        let page = flow_text(cache.pages.window(position.row, height), width)?;
        for glyph in page.glyphs {
            let outcome = view.write(glyph.point, glyph.grapheme, style);
            debug_assert_ne!(outcome, WriteOutcome::Clipped);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use yo_core::ActivityKind;

    use super::*;
    use crate::{
        surface::{Point, Rect, Size, Surface},
        transcript::TranscriptState,
    };

    // 스크롤은 같은 배치를 재사용하고 폭·선택 항목 revision 변경만 캐시를 갱신한다.
    // 갱신된 출력의 끝 따라가기와 읽던 위치 유지도 별개로 적용한다.
    #[test]
    fn cache_tracks_source_and_width_while_navigation_tracks_the_reader() {
        let mut chat = TranscriptState::new();
        let id = TranscriptItemId::new(1);
        chat.start_typed_activity_message(id, ActivityKind::ToolResult)
            .unwrap();
        chat.append_text(id, "Tool\nfirst\nsecond\nthird\nfourth")
            .unwrap();
        let output = OutputView::default();
        let mut position = Position::latest();
        let mut render =
            |chat: &TranscriptState, width: u16, commands: &[TranscriptScrollCommand]| {
                let size = Size::new(width, 2);
                let mut surface = Surface::new(size).unwrap();
                output
                    .render(
                        chat.slice(0..chat.items().len()),
                        &mut surface.view(Rect::new(Point::new(0, 0), size)).unwrap(),
                        NonZeroU16::new(width).unwrap(),
                        Style::default(),
                        &mut position,
                        commands,
                    )
                    .unwrap();
                (
                    Arc::clone(output.cache.borrow().as_ref().unwrap()),
                    position,
                )
            };
        let (first, _) = render(&chat, 24, &[]);
        let (scrolled, detached) = render(&chat, 24, &[TranscriptScrollCommand::LineDown]);
        assert!(Arc::ptr_eq(&first, &scrolled));
        assert_eq!(detached.row, 1);
        chat.append_text(id, "\nfifth").unwrap();
        let (updated, detached) = render(&chat, 24, &[]);
        assert!(!Arc::ptr_eq(&scrolled, &updated));
        assert_eq!(detached.row, 1);
        assert_eq!(
            updated.pages.window(4, NonZeroU16::new(1).unwrap()),
            "fifth"
        );
        let (_, tail) = render(&chat, 24, &[TranscriptScrollCommand::JumpToTail]);
        assert_eq!(tail.row, 3);
        chat.append_text(id, "\nsixth").unwrap();
        let (latest, tail) = render(&chat, 24, &[]);
        assert_eq!(tail.row, 4);
        let (resized, _) = render(&chat, 3, &[]);
        assert!(!Arc::ptr_eq(&latest, &resized));
        assert!(resized.pages.row_count() > latest.pages.row_count());
    }
    // 폭 변경으로 줄 수가 달라져도 읽던 원문 위치를 유지하고, 사용자 이동만 새 기준점을 만든다.
    #[test]
    fn detached_output_restores_source_position_after_width_round_trip() {
        use crate::surface::CellContent;

        let mut chat = TranscriptState::new();
        let id = TranscriptItemId::new(1);
        chat.start_typed_activity_message(id, ActivityKind::ToolResult)
            .unwrap();
        chat.append_text(
            id,
            &format!("Tool\n{}TARGET{}", "a".repeat(160), "z".repeat(400)),
        )
        .unwrap();
        let output = OutputView::default();
        let mut position = Position::latest();
        let mut render = |width: u16, commands: &[TranscriptScrollCommand]| {
            let size = Size::new(width, 2);
            let mut surface = Surface::new(size).unwrap();
            output
                .render(
                    chat.slice(0..chat.items().len()),
                    &mut surface.view(Rect::new(Point::new(0, 0), size)).unwrap(),
                    NonZeroU16::new(width).unwrap(),
                    Style::default(),
                    &mut position,
                    commands,
                )
                .unwrap();
            let first = (0..width)
                .filter_map(
                    |x| match surface.cell(Point::new(x, 0)).unwrap().content() {
                        CellContent::Grapheme { text, .. } => Some(text.as_ref()),
                        _ => None,
                    },
                )
                .collect::<String>();
            (position, first)
        };
        let (wide, text) = render(
            80,
            &[
                TranscriptScrollCommand::LineDown,
                TranscriptScrollCommand::LineDown,
            ],
        );
        assert_eq!(wide.source_anchor, Some(160));
        assert!(text.starts_with("TARGET"));
        let (narrow, text) = render(24, &[]);
        assert_eq!(narrow.row, 6);
        assert_eq!(narrow.source_anchor, Some(160));
        assert!(text.contains("TARGET"));
        let (restored, text) = render(80, &[]);
        assert_eq!(restored.row, wide.row);
        assert!(text.starts_with("TARGET"));
        render(24, &[]);
        let (moved, _) = render(24, &[TranscriptScrollCommand::LineDown]);
        assert_eq!(moved.source_anchor, Some(168));
        let (restored, _) = render(80, &[]);
        assert_eq!(restored.source_anchor, Some(168));
    }
    // 탭 하나가 여러 행으로 펼쳐져도 같은 폭의 재그리기는 탭 첫 행으로 되돌아가지 않는다.
    #[test]
    fn repaint_keeps_navigation_inside_an_expanded_tab() {
        let mut chat = TranscriptState::new();
        let id = TranscriptItemId::new(1);
        chat.start_typed_activity_message(id, ActivityKind::ToolResult)
            .unwrap();
        chat.append_text(id, "Tool\n\tXYZ").unwrap();
        let output = OutputView::default();
        let mut position = Position::latest();
        for commands in [vec![TranscriptScrollCommand::LineDown], vec![]] {
            let size = Size::new(1, 1);
            let mut surface = Surface::new(size).unwrap();
            output
                .render(
                    chat.slice(0..chat.items().len()),
                    &mut surface.view(Rect::new(Point::new(0, 0), size)).unwrap(),
                    NonZeroU16::MIN,
                    Style::default(),
                    &mut position,
                    &commands,
                )
                .unwrap();
            assert_eq!(position.row, 1);
            assert_eq!(position.source_anchor, Some(0));
        }
    }
}
