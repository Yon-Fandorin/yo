use super::super::{InlineFrameError, InlineViewport};
use crate::surface::{Size, Surface};

// 신뢰 가능한 같은 크기 frame은 실제 cell 차이만 계산하는 증분 diff를 사용한다.
#[test]
fn trusted_update_uses_an_incremental_diff() {
    let previous = Surface::new(Size::new(3, 2)).unwrap();
    let current = previous.clone();
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(previous.size()).commit();

    let pending = viewport.begin_frame(current.size());
    let diff = pending.diff(Some(&previous), &current).unwrap();

    assert!(diff.is_empty());
}

// geometry가 무효화되면 같은 크기·같은 cell이어도 모든 행을 다시 그린다.
#[test]
fn invalidated_geometry_uses_a_complete_diff() {
    let previous = Surface::new(Size::new(3, 2)).unwrap();
    let current = previous.clone();
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(previous.size()).commit();
    viewport.invalidate_frame();

    let pending = viewport.begin_frame(current.size());
    let diff = pending.diff(Some(&previous), &current).unwrap();

    assert_eq!(
        diff.spans()
            .iter()
            .map(|span| (span.row(), span.start_column(), span.end_column()))
            .collect::<Vec<_>>(),
        [(0, 0, 3), (1, 0, 3)]
    );
}

// 실제 크기 변경은 이전·현재 geometry를 보존하면서 최신 frame의 모든 행을 방출한다.
#[test]
fn resized_viewport_preserves_both_sizes_in_a_complete_diff() {
    let previous = Surface::new(Size::new(4, 3)).unwrap();
    let current = Surface::new(Size::new(2, 2)).unwrap();
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(previous.size()).commit();

    let pending = viewport.begin_frame(current.size());
    let diff = pending.diff(Some(&previous), &current).unwrap();

    assert_eq!(diff.previous_size(), Size::new(4, 3));
    assert_eq!(diff.current_size(), Size::new(2, 2));
    assert_eq!(
        diff.spans()
            .iter()
            .map(|span| (span.row(), span.start_column(), span.end_column()))
            .collect::<Vec<_>>(),
        [(0, 0, 2), (1, 0, 2)]
    );
}

// 재고정은 버린 영역의 cell을 기준으로 삼지 않고 최신 frame 전체를 다시 그린다.
#[test]
fn reanchor_uses_a_complete_diff_without_a_previous_frame() {
    let current = Surface::new(Size::new(2, 2)).unwrap();
    let mut viewport = InlineViewport::default();
    drop(viewport.begin_frame(Size::new(4, 3)));

    let pending = viewport.begin_frame(current.size());
    let diff = pending.diff(None, &current).unwrap();

    assert_eq!(diff.spans().len(), 2);
    assert_eq!(diff.current_size(), current.size());
}

// plan 이후 다른 크기의 current frame을 섞으면 출력 전에 구조화된 오류로 거부한다.
#[test]
fn current_size_drift_is_rejected_before_diffing() {
    let mut viewport = InlineViewport::default();
    let pending = viewport.begin_frame(Size::new(3, 2));
    let current = Surface::new(Size::new(4, 2)).unwrap();

    let error = pending.diff(None, &current).unwrap_err();

    assert_eq!(
        error,
        InlineFrameError::CurrentSizeMismatch {
            expected: Size::new(3, 2),
            actual: Size::new(4, 2),
        }
    );
}

// 증분 갱신은 정확한 직전 frame 없이는 안전하게 계산할 수 없으므로 명시적으로 실패한다.
#[test]
fn trusted_update_requires_the_previous_frame() {
    let current = Surface::new(Size::new(3, 2)).unwrap();
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(current.size()).commit();

    let pending = viewport.begin_frame(current.size());
    let error = pending.diff(None, &current).unwrap_err();

    assert_eq!(error, InlineFrameError::PreviousFrameRequired);
}

// 상태가 기억한 크기와 다른 previous frame은 잘못된 증분 기준이므로 거부한다.
#[test]
fn previous_size_drift_is_rejected_before_diffing() {
    let current = Surface::new(Size::new(3, 2)).unwrap();
    let wrong_previous = Surface::new(Size::new(2, 2)).unwrap();
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(current.size()).commit();

    let pending = viewport.begin_frame(current.size());
    let error = pending.diff(Some(&wrong_previous), &current).unwrap_err();

    assert_eq!(
        error,
        InlineFrameError::PreviousSizeMismatch {
            expected: Size::new(3, 2),
            actual: Size::new(2, 2),
        }
    );
}
