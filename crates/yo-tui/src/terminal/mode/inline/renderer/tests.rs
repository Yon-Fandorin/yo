use std::io::{self, Write};

use super::{InlineRenderError, InlineRenderer};
use crate::{
    surface::{Point, Rect, Size, Style, Surface},
    terminal::mode::inline::{InlineFramePlan, InlineViewport},
};

const TERMINAL_SIZE: Size = Size::new(80, 24);

mod publication;
mod restore;

fn surface(size: Size, point: Point, text: &str) -> Surface {
    let mut surface = Surface::new(size).unwrap();
    let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
    view.write(
        point,
        crate::surface::Grapheme::try_from(text).unwrap(),
        Style::default(),
    );
    surface
}

// 초기 frame은 행을 확보한 뒤 anchor 기준 상대 이동으로 그리고 다시 anchor로 돌아온다.
#[test]
fn initial_render_uses_only_relative_row_and_column_controls() {
    let current = surface(Size::new(3, 2), Point::new(1, 0), "A");
    let mut viewport = InlineViewport::default();
    let pending = viewport.begin_frame(current.size());
    let mut renderer = InlineRenderer::new(Vec::new());

    renderer
        .render(pending, None, &current, None, TERMINAL_SIZE)
        .unwrap();

    let output = renderer.into_inner();
    assert_eq!(
        output,
        b"\x1b[?25l\r\n\n\x1b[2A\x1b[1G\x1b[1G\x1b[0;39;49m A \
          \x1b[1B\x1b[1G   \x1b[1B\x1b[1G\x1b[?25h"
    );
}

// shrink는 이전 viewport에서 새 viewport 밖에 남은 소유 행만 지우고 새 anchor로 복귀한다.
#[test]
fn shrinking_clears_only_the_owned_surplus_rows() {
    let previous = Surface::new(Size::new(2, 3)).unwrap();
    let current = Surface::new(Size::new(2, 1)).unwrap();
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(previous.size()).commit();
    let pending = viewport.begin_frame(current.size());
    assert!(matches!(pending.plan(), InlineFramePlan::Reconcile { .. }));
    let mut renderer = InlineRenderer::new(Vec::new());

    renderer
        .render(pending, Some(&previous), &current, None, TERMINAL_SIZE)
        .unwrap();

    let output = renderer.into_inner();
    assert!(
        output
            .windows(4)
            .filter(|bytes| *bytes == b"\x1b[2K")
            .count()
            == 2
    );
    assert!(output.ends_with(b"\x1b[1A\x1b[1G\x1b[?25h"));
}

// grow는 기존 anchor 아래에 늘어난 행만 확보한 뒤 새 전체 높이만큼 상대 이동한다.
#[test]
fn growing_allocates_only_the_added_rows() {
    let previous = Surface::new(Size::new(2, 1)).unwrap();
    let current = Surface::new(Size::new(2, 3)).unwrap();
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(previous.size()).commit();
    let pending = viewport.begin_frame(current.size());
    let mut renderer = InlineRenderer::new(Vec::new());

    renderer
        .render(pending, Some(&previous), &current, None, TERMINAL_SIZE)
        .unwrap();

    assert!(
        renderer
            .into_inner()
            .starts_with(b"\x1b[?25l\r\n\n\x1b[3A\x1b[1G")
    );
}

// anchor를 잃은 복구는 버린 행 아래로 먼저 이동한 후 새 viewport 행을 별도로 확보한다.
#[test]
fn reanchor_skips_abandoned_rows_before_allocating_the_new_viewport() {
    let current = Surface::new(Size::new(2, 1)).unwrap();
    let mut viewport = InlineViewport::default();
    drop(viewport.begin_frame(Size::new(2, 2)));
    let pending = viewport.begin_frame(current.size());
    let mut renderer = InlineRenderer::new(Vec::new());

    renderer
        .render(pending, None, &current, None, TERMINAL_SIZE)
        .unwrap();

    assert!(
        renderer
            .into_inner()
            .starts_with(b"\x1b[?25l\r\n\n\r\n\x1b[1A\x1b[1G")
    );
}

// 높이와 폭이 0이면 행을 확보하거나 위아래로 이동하지 않고 anchor 열만 정규화한다.
#[test]
fn zero_sized_viewport_emits_no_row_movement() {
    let current = Surface::new(Size::new(0, 0)).unwrap();
    let mut viewport = InlineViewport::default();
    let pending = viewport.begin_frame(current.size());
    let mut renderer = InlineRenderer::new(Vec::new());

    renderer
        .render(pending, None, &current, None, TERMINAL_SIZE)
        .unwrap();

    assert_eq!(renderer.into_inner(), b"\x1b[?25l\x1b[1G\x1b[1G\x1b[?25h");
}

// flush 실패는 frame을 commit하지 않아 다음 시도가 안전한 reanchor 계획을 선택한다.
#[test]
fn flush_failure_leaves_the_viewport_untrusted() {
    let current = Surface::new(Size::new(2, 1)).unwrap();
    let mut viewport = InlineViewport::default();
    let pending = viewport.begin_frame(current.size());
    let mut renderer = InlineRenderer::new(FlushFailingWriter(Vec::new()));

    let error = renderer
        .render(pending, None, &current, None, TERMINAL_SIZE)
        .unwrap_err();

    assert!(matches!(error, InlineRenderError::Output(_)));
    assert!(matches!(
        viewport.begin_frame(current.size()).plan(),
        InlineFramePlan::Reanchor { .. }
    ));
}

// bytes 일부만 기록된 write 실패도 성공으로 commit하지 않고 다음 frame을 재고정한다.
#[test]
fn partial_write_failure_leaves_the_viewport_untrusted() {
    let current = Surface::new(Size::new(2, 1)).unwrap();
    let mut viewport = InlineViewport::default();
    let pending = viewport.begin_frame(current.size());
    let mut renderer = InlineRenderer::new(PartialFailingWriter { remaining: 2 });

    let error = renderer
        .render(pending, None, &current, None, TERMINAL_SIZE)
        .unwrap_err();

    assert!(matches!(error, InlineRenderError::Output(_)));
    assert!(matches!(
        viewport.begin_frame(current.size()).plan(),
        InlineFramePlan::Reanchor { .. }
    ));
}

// frame write가 한 번 실패해도 주원인을 보존하면서 즉시 cursor-visible 복구를 시도한다.
#[test]
fn recoverable_write_failure_attempts_to_show_the_cursor_immediately() {
    let current = Surface::new(Size::new(2, 1)).unwrap();
    let mut viewport = InlineViewport::default();
    let pending = viewport.begin_frame(current.size());
    let mut renderer = InlineRenderer::new(OneShotFailingWriter {
        bytes: Vec::new(),
        remaining: HIDE_CURSOR_LENGTH,
        failed: false,
    });

    let error = renderer
        .render(pending, None, &current, None, TERMINAL_SIZE)
        .unwrap_err();
    let output = renderer.into_inner().bytes;

    assert!(matches!(error, InlineRenderError::Output(_)));
    assert!(output.ends_with(b"\x1b[?25h"));
}

// 첫 frame 완료 뒤 물리 cursor는 bottom anchor가 아니라 shell이 지정한 prompt caret에 보인다.
#[test]
fn initial_frame_places_the_physical_cursor_at_the_prompt_caret() {
    let current = Surface::new(Size::new(4, 3)).unwrap();
    let mut viewport = InlineViewport::default();
    let pending = viewport
        .begin_frame_at(current.size(), Point::new(2, 1))
        .unwrap();
    let mut renderer = InlineRenderer::new(Vec::new());

    renderer
        .render(pending, None, &current, None, TERMINAL_SIZE)
        .unwrap();

    assert!(renderer.into_inner().ends_with(b"\x1b[1A\x1b[3G\x1b[?25h"));
}

// 다음 frame은 기억한 caret에서 논리 anchor를 거쳐 viewport top으로 돌아와 새 caret을 배치한다.
#[test]
fn steady_frame_returns_from_the_remembered_caret_with_relative_moves() {
    let size = Size::new(4, 3);
    let previous = Surface::new(size).unwrap();
    let current = previous.clone();
    let mut viewport = InlineViewport::default();
    viewport
        .begin_frame_at(size, Point::new(2, 1))
        .unwrap()
        .commit();
    let pending = viewport.begin_frame_at(size, Point::new(1, 0)).unwrap();
    let mut renderer = InlineRenderer::new(Vec::new());

    renderer
        .render(pending, Some(&previous), &current, None, TERMINAL_SIZE)
        .unwrap();

    assert_eq!(
        renderer.into_inner(),
        b"\x1b[?25l\x1b[2B\x1b[3A\x1b[1G\x1b[2G\x1b[?25h"
    );
}

struct FlushFailingWriter(Vec<u8>);

impl Write for FlushFailingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "flush"))
    }
}

struct PartialFailingWriter {
    remaining: usize,
}

const HIDE_CURSOR_LENGTH: usize = b"\x1b[?25l".len();

struct OneShotFailingWriter {
    bytes: Vec<u8>,
    remaining: usize,
    failed: bool,
}

impl Write for PartialFailingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "write"));
        }
        let written = self.remaining.min(bytes.len());
        self.remaining -= written;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Write for OneShotFailingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.failed {
            self.bytes.extend_from_slice(bytes);
            return Ok(bytes.len());
        }
        if self.remaining == 0 {
            self.failed = true;
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "write"));
        }
        let written = self.remaining.min(bytes.len());
        self.remaining -= written;
        self.bytes.extend_from_slice(&bytes[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
