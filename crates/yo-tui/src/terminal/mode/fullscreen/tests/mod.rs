use std::io::{self, Write};

use super::{FullscreenFrameError, FullscreenFramePlan, FullscreenRenderer, FullscreenViewport};
use crate::surface::{Grapheme, Point, Rect, Size, Style, Surface, WriteOutcome};

mod failures;

fn surface_with(size: Size, point: Point, text: &str) -> Surface {
    let mut surface = Surface::new(size).unwrap();
    let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
    assert_eq!(
        view.write(point, Grapheme::try_from(text).unwrap(), Style::default()),
        WriteOutcome::Written
    );
    surface
}

#[derive(Debug, Default)]
struct RecordingWriter {
    bytes: Vec<u8>,
    bytes_before_failure: Option<usize>,
    fail_flush: bool,
}

impl Write for RecordingWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let Some(remaining) = self.bytes_before_failure else {
            self.bytes.extend_from_slice(buffer);
            return Ok(buffer.len());
        };
        if remaining == 0 {
            return Err(io::Error::other("injected write failure"));
        }
        let written = remaining.min(buffer.len());
        self.bytes.extend_from_slice(&buffer[..written]);
        self.bytes_before_failure = Some(remaining - written);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.fail_flush {
            Err(io::Error::other("injected flush failure"))
        } else {
            Ok(())
        }
    }
}

// 첫 frame은 전체 Surface를 그린 뒤 shell이 지정한 prompt cursor로 이동한다.
#[test]
fn initial_frame_is_complete_and_places_the_cursor() {
    let current = Surface::new(Size::new(2, 1)).unwrap();
    let mut viewport = FullscreenViewport::default();
    let pending = viewport
        .begin_frame(current.size(), Point::new(1, 0))
        .unwrap();
    let mut renderer = FullscreenRenderer::new(Vec::new());

    renderer.render(pending, None, &current).unwrap();

    assert_eq!(renderer.into_inner(), b"\x1b[1;1H\x1b[0;39;49m  \x1b[1;2H");
}

// 신뢰할 수 있는 같은 크기 frame은 변경된 cell만 쓰고 cursor 위치는 항상 다시 적용한다.
#[test]
fn trusted_same_size_frame_uses_an_incremental_diff() {
    let size = Size::new(2, 1);
    let previous = Surface::new(size).unwrap();
    let current = surface_with(size, Point::new(0, 0), "A");
    let mut viewport = FullscreenViewport::default();
    viewport
        .begin_frame(size, Point::new(0, 0))
        .unwrap()
        .commit();
    let pending = viewport.begin_frame(size, Point::new(1, 0)).unwrap();
    assert!(matches!(pending.plan(), FullscreenFramePlan::Update { .. }));
    let mut renderer = FullscreenRenderer::new(Vec::new());

    renderer.render(pending, Some(&previous), &current).unwrap();

    assert_eq!(renderer.into_inner(), b"\x1b[1;1H\x1b[0;39;49mA\x1b[1;2H");
}

// resize는 이전 geometry를 ANSI encoder에 넘기지 않고 현재 화면 전체를 다시 그린다.
#[test]
fn resize_selects_a_complete_current_frame() {
    let previous = Surface::new(Size::new(2, 1)).unwrap();
    let current = surface_with(Size::new(3, 2), Point::new(2, 1), "X");
    let mut viewport = FullscreenViewport::default();
    viewport
        .begin_frame(previous.size(), Point::new(0, 0))
        .unwrap()
        .commit();
    let pending = viewport
        .begin_frame(current.size(), Point::new(2, 1))
        .unwrap();
    assert!(matches!(
        pending.plan(),
        FullscreenFramePlan::Complete { .. }
    ));
    let mut renderer = FullscreenRenderer::new(Vec::new());

    renderer.render(pending, Some(&previous), &current).unwrap();

    let output = renderer.into_inner();
    assert!(output.windows(6).any(|window| window == b"\x1b[2;1H"));
    assert!(output.ends_with(b"\x1b[2;3H"));
}

// 유효하지 않은 cursor는 출력 계획을 시작하지 않아 기존 frame 신뢰를 유지한다.
#[test]
fn invalid_cursor_preserves_the_previous_frame_trust() {
    let size = Size::new(2, 1);
    let mut viewport = FullscreenViewport::default();
    viewport
        .begin_frame(size, Point::new(0, 0))
        .unwrap()
        .commit();

    assert_eq!(
        viewport.begin_frame(size, Point::new(2, 0)).unwrap_err(),
        FullscreenFrameError::CursorOutOfBounds {
            cursor: Point::new(2, 0),
            size,
        }
    );
    let pending = viewport.begin_frame(size, Point::new(1, 0)).unwrap();
    assert!(matches!(pending.plan(), FullscreenFramePlan::Update { .. }));
}
