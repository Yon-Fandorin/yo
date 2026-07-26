use std::io::{self, Write};

use super::super::{InlineRenderError, InlineRenderer, InlineRestoreOutcome};
use crate::{surface::Size, terminal::mode::inline::InlineViewport};

// 정상 종료는 소유한 두 행만 지우고 former viewport 첫 행, 즉 persistent output 바로 아래에 둔다.
#[test]
fn owned_viewport_is_cleared_and_cursor_returns_below_persistent_output() {
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(Size::new(4, 2)).commit();
    let pending = viewport.begin_restore();
    let mut renderer = InlineRenderer::new(Vec::new());

    let outcome = renderer.restore(pending).unwrap();

    assert_eq!(outcome, InlineRestoreOutcome::Cleared);
    assert_eq!(
        renderer.into_inner(),
        b"\x1b[2A\x1b[1G\x1b[1G\x1b[2K\
          \x1b[1B\x1b[1G\x1b[2K\x1b[1A\x1b[1G"
    );
}

// clear 출력이 일부만 성공하면 재시도는 미확인 영역을 지우지 않고 상태를 명시적으로 보고한다.
#[test]
fn partial_clear_failure_becomes_a_non_erasing_retry() {
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(Size::new(4, 2)).commit();
    let pending = viewport.begin_restore();
    let mut failing = InlineRenderer::new(PartialFailingWriter { remaining: 3 });

    let error = failing.restore(pending).unwrap_err();

    assert!(matches!(error, InlineRenderError::Output(_)));
    let retry = viewport.begin_restore();
    let mut recorder = InlineRenderer::new(Vec::new());
    let outcome = recorder.restore(retry).unwrap();
    assert_eq!(
        outcome,
        InlineRestoreOutcome::LeftUntrusted { abandoned_rows: 2 }
    );
    assert!(recorder.into_inner().is_empty());
}

// clear bytes 뒤 flush가 실패해도 성공으로 확정하지 않고, 무출력 보고 후 멱등 상태로 수렴한다.
#[test]
fn flush_failure_becomes_an_untrusted_report_then_nothing() {
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(Size::new(4, 2)).commit();
    let pending = viewport.begin_restore();
    let mut failing = InlineRenderer::new(FlushFailingWriter);

    let error = failing.restore(pending).unwrap_err();

    assert!(matches!(error, InlineRenderError::Output(_)));

    let retry = viewport.begin_restore();
    let mut recorder = InlineRenderer::new(Vec::new());
    let outcome = recorder.restore(retry).unwrap();
    assert_eq!(
        outcome,
        InlineRestoreOutcome::LeftUntrusted { abandoned_rows: 2 }
    );
    assert!(recorder.into_inner().is_empty());

    let final_retry = viewport.begin_restore();
    let mut final_recorder = InlineRenderer::new(Vec::new());
    assert_eq!(
        final_recorder.restore(final_retry).unwrap(),
        InlineRestoreOutcome::Nothing
    );
    assert!(final_recorder.into_inner().is_empty());
}

// 높이 0 viewport는 지울 행 없이 현재 위치의 열만 정규화한다.
#[test]
fn zero_height_restore_only_normalizes_the_anchor_column() {
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(Size::new(0, 0)).commit();
    let pending = viewport.begin_restore();
    let mut renderer = InlineRenderer::new(Vec::new());

    let outcome = renderer.restore(pending).unwrap();

    assert_eq!(outcome, InlineRestoreOutcome::Cleared);
    assert_eq!(renderer.into_inner(), b"\x1b[1G\x1b[1G");
}

struct PartialFailingWriter {
    remaining: usize,
}

struct FlushFailingWriter;

impl Write for FlushFailingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "flush"))
    }
}

impl Write for PartialFailingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "clear"));
        }
        let written = self.remaining.min(bytes.len());
        self.remaining -= written;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
