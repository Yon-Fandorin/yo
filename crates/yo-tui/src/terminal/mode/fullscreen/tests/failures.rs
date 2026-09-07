use super::{
    FullscreenFrameError, FullscreenFramePlan, FullscreenRenderer, FullscreenViewport, Point,
    RecordingWriter, Size, Surface,
};

#[derive(Default)]
struct RecoveringWriter {
    bytes: Vec<u8>,
    writes: usize,
    flushes: usize,
    fail_write: bool,
}

impl std::io::Write for RecoveringWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.writes += 1;
        if self.fail_write && self.writes == 2 {
            return Err(std::io::Error::other("write failure"));
        }
        let count = if self.fail_write && self.writes == 1 {
            9.min(bytes.len())
        } else {
            bytes.len()
        };
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.flushes += 1;
        if !self.fail_write && self.flushes == 1 {
            return Err(std::io::Error::other("flush failure"));
        }
        Ok(())
    }
}

// 동기화 시작 이후 부분 쓰기·flush가 실패하면 종료 시퀀스를 재시도하되 기존 오류와
// viewport 불신 상태를 유지한다. 출력이 회복되면 다음 완전한 frame으로 정상 재개한다.
#[test]
fn output_failure_releases_synchronized_update_before_full_recovery() {
    for fail_write in [true, false] {
        let size = Size::new(2, 1);
        let current = Surface::new(size).unwrap();
        let mut viewport = FullscreenViewport::default();
        let pending = viewport.begin_frame(size, Point::new(0, 0)).unwrap();
        let mut renderer = FullscreenRenderer::new(RecoveringWriter {
            fail_write,
            ..Default::default()
        });
        let error = renderer.render(pending, None, &current).unwrap_err();
        match error {
            super::super::FullscreenRenderError::Ansi(crate::terminal::AnsiEncodeError::Io(
                error,
            )) if fail_write => assert_eq!(error.to_string(), "write failure"),
            super::super::FullscreenRenderError::Flush(error) if !fail_write => {
                assert_eq!(error.to_string(), "flush failure")
            },
            other => panic!("unexpected failure: {other:?}"),
        }
        let writer = renderer.into_inner();
        assert!(writer.bytes.starts_with(b"\x1b[?2026h"));
        assert!(writer.bytes.ends_with(b"\x1b[?2026l"));
        let recovery = viewport.begin_frame(size, Point::new(0, 0)).unwrap();
        assert!(matches!(
            recovery.plan(),
            FullscreenFramePlan::Complete { .. }
        ));
        FullscreenRenderer::new(writer)
            .render(recovery, None, &current)
            .unwrap();
    }
}

// flush가 실패하면 frame을 신뢰하지 않아 다음 시도는 이전 Surface와 무관하게 전체 갱신한다.
#[test]
fn flush_failure_forces_the_next_complete_frame() {
    let size = Size::new(2, 1);
    let current = Surface::new(size).unwrap();
    let mut viewport = FullscreenViewport::default();
    viewport
        .begin_frame(size, Point::new(0, 0))
        .unwrap()
        .commit();
    let pending = viewport.begin_frame(size, Point::new(0, 0)).unwrap();
    let mut renderer = FullscreenRenderer::new(RecordingWriter {
        fail_flush: true,
        ..RecordingWriter::default()
    });

    assert!(renderer.render(pending, Some(&current), &current).is_err());

    let recovery = viewport.begin_frame(size, Point::new(0, 0)).unwrap();
    assert!(matches!(
        recovery.plan(),
        FullscreenFramePlan::Complete { .. }
    ));
}

// frame 바이트가 일부 기록된 뒤 실패해도 다음 시도는 손상된 화면과 diff하지 않는다.
#[test]
fn partial_write_failure_forces_the_next_complete_frame() {
    let size = Size::new(2, 1);
    let current = Surface::new(size).unwrap();
    let mut viewport = FullscreenViewport::default();
    viewport
        .begin_frame(size, Point::new(0, 0))
        .unwrap()
        .commit();
    let pending = viewport.begin_frame(size, Point::new(0, 0)).unwrap();
    let mut renderer = FullscreenRenderer::new(RecordingWriter {
        bytes_before_failure: Some(2),
        ..RecordingWriter::default()
    });

    assert!(renderer.render(pending, Some(&current), &current).is_err());
    assert!(!renderer.into_inner().bytes.is_empty());

    let recovery = viewport.begin_frame(size, Point::new(0, 0)).unwrap();
    assert!(matches!(
        recovery.plan(),
        FullscreenFramePlan::Complete { .. }
    ));
}

// trusted update에 previous frame이 없으면 쓰기 전에 실패하고 이후 복구는 전체 frame을 선택한다.
#[test]
fn missing_previous_frame_fails_before_output() {
    let size = Size::new(2, 1);
    let current = Surface::new(size).unwrap();
    let mut viewport = FullscreenViewport::default();
    viewport
        .begin_frame(size, Point::new(0, 0))
        .unwrap()
        .commit();
    let pending = viewport.begin_frame(size, Point::new(0, 0)).unwrap();
    let mut renderer = FullscreenRenderer::new(RecordingWriter::default());

    let error = renderer.render(pending, None, &current).unwrap_err();

    assert!(matches!(
        error,
        super::super::FullscreenRenderError::Frame(FullscreenFrameError::PreviousFrameRequired)
    ));
    assert!(renderer.into_inner().bytes.is_empty());
    let recovery = viewport.begin_frame(size, Point::new(0, 0)).unwrap();
    assert!(matches!(
        recovery.plan(),
        FullscreenFramePlan::Complete { .. }
    ));
}
