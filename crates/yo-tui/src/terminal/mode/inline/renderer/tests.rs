use std::io::{self, Write};

use super::{InlineRenderError, InlineRenderer};
use crate::{
    surface::{Point, Rect, Size, Style, Surface},
    terminal::mode::inline::{InlineFramePlan, InlineViewport},
};

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

    renderer.render(pending, None, &current).unwrap();

    let output = renderer.into_inner();
    assert_eq!(
        output,
        b"\r\n\n\x1b[2A\x1b[1G\x1b[1G\x1b[0;39;49m A \
          \x1b[1B\x1b[1G   \x1b[1B\x1b[1G"
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

    renderer.render(pending, Some(&previous), &current).unwrap();

    let output = renderer.into_inner();
    assert!(
        output
            .windows(4)
            .filter(|bytes| *bytes == b"\x1b[2K")
            .count()
            == 2
    );
    assert!(output.ends_with(b"\x1b[1A\x1b[1G"));
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

    renderer.render(pending, Some(&previous), &current).unwrap();

    assert!(renderer.into_inner().starts_with(b"\r\n\n\x1b[3A\x1b[1G"));
}

// anchor를 잃은 복구는 버린 행 아래로 먼저 이동한 후 새 viewport 행을 별도로 확보한다.
#[test]
fn reanchor_skips_abandoned_rows_before_allocating_the_new_viewport() {
    let current = Surface::new(Size::new(2, 1)).unwrap();
    let mut viewport = InlineViewport::default();
    drop(viewport.begin_frame(Size::new(2, 2)));
    let pending = viewport.begin_frame(current.size());
    let mut renderer = InlineRenderer::new(Vec::new());

    renderer.render(pending, None, &current).unwrap();

    assert!(
        renderer
            .into_inner()
            .starts_with(b"\r\n\n\r\n\x1b[1A\x1b[1G")
    );
}

// 높이와 폭이 0이면 행을 확보하거나 위아래로 이동하지 않고 anchor 열만 정규화한다.
#[test]
fn zero_sized_viewport_emits_no_row_movement() {
    let current = Surface::new(Size::new(0, 0)).unwrap();
    let mut viewport = InlineViewport::default();
    let pending = viewport.begin_frame(current.size());
    let mut renderer = InlineRenderer::new(Vec::new());

    renderer.render(pending, None, &current).unwrap();

    assert_eq!(renderer.into_inner(), b"\x1b[1G\x1b[1G");
}

// flush 실패는 frame을 commit하지 않아 다음 시도가 안전한 reanchor 계획을 선택한다.
#[test]
fn flush_failure_leaves_the_viewport_untrusted() {
    let current = Surface::new(Size::new(2, 1)).unwrap();
    let mut viewport = InlineViewport::default();
    let pending = viewport.begin_frame(current.size());
    let mut renderer = InlineRenderer::new(FlushFailingWriter(Vec::new()));

    let error = renderer.render(pending, None, &current).unwrap_err();

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

    let error = renderer.render(pending, None, &current).unwrap_err();

    assert!(matches!(error, InlineRenderError::Output(_)));
    assert!(matches!(
        viewport.begin_frame(current.size()).plan(),
        InlineFramePlan::Reanchor { .. }
    ));
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
