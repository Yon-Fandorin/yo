use super::{
    FullscreenFrameError, FullscreenFramePlan, FullscreenRenderer, FullscreenViewport, Point,
    RecordingWriter, Size, Surface,
};

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
