use super::{InlineFramePlan, InlineViewport};
use crate::surface::{Point, Size};

mod diff;
mod restore;

// 첫 frame은 현재 높이의 viewport와 그 바로 아래 anchor를 새로 배치해야 한다.
#[test]
fn first_frame_initializes_an_owned_viewport() {
    let mut viewport = InlineViewport::default();

    let pending = viewport.begin_frame(Size::new(80, 3));

    assert_eq!(
        pending.plan(),
        InlineFramePlan::Initialize {
            current: Size::new(80, 3),
            cursor: Point::new(0, 3),
        }
    );
}

// 성공한 같은 크기 frame은 기존 anchor와 소유 영역 안에서 diff 갱신할 수 있다.
#[test]
fn committed_same_size_frame_can_update_in_place() {
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(Size::new(80, 3)).commit();

    let pending = viewport.begin_frame(Size::new(80, 3));

    assert_eq!(
        pending.plan(),
        InlineFramePlan::Update {
            current: Size::new(80, 3),
            previous_cursor: Point::new(0, 3),
            cursor: Point::new(0, 3),
        }
    );
}

// 높이가 바뀌면 이전·현재 중 더 큰 전체 행을 소유한 채 새 anchor 위치까지 조정한다.
#[test]
fn height_change_reconciles_the_larger_row_footprint() {
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(Size::new(80, 5)).commit();

    let pending = viewport.begin_frame(Size::new(80, 2));

    assert_eq!(
        pending.plan(),
        InlineFramePlan::Reconcile {
            previous: Size::new(80, 5),
            current: Size::new(80, 2),
            owned_rows: 5,
            previous_cursor: Point::new(0, 5),
            cursor: Point::new(0, 2),
        }
    );
}

// 터미널 geometry event는 크기 값이 같아도 이전 frame을 무효화해 전체 재조정을 요구한다.
#[test]
fn geometry_invalidation_requires_reconciliation_at_the_same_size() {
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(Size::new(80, 3)).commit();
    viewport.invalidate_frame();

    let pending = viewport.begin_frame(Size::new(80, 3));

    assert_eq!(
        pending.plan(),
        InlineFramePlan::Reconcile {
            previous: Size::new(80, 3),
            current: Size::new(80, 3),
            owned_rows: 3,
            previous_cursor: Point::new(0, 3),
            cursor: Point::new(0, 3),
        }
    );
}

// anchor 신뢰를 잃으면 예전 행을 지우지 않고 그 아래에 새 viewport를 배치한다.
#[test]
fn lost_anchor_abandons_old_rows_before_reanchoring() {
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(Size::new(80, 4)).commit();
    viewport.abandon_anchor();

    let pending = viewport.begin_frame(Size::new(100, 2));

    assert_eq!(
        pending.plan(),
        InlineFramePlan::Reanchor {
            abandoned_rows: 4,
            current: Size::new(100, 2),
            cursor: Point::new(0, 2),
        }
    );
}

// 출력 성공을 commit하지 않으면 부분 write 가능성을 보수적으로 보고 다음 frame을 재고정한다.
#[test]
fn uncommitted_frame_is_treated_as_an_uncertain_anchor() {
    let mut viewport = InlineViewport::default();
    drop(viewport.begin_frame(Size::new(80, 3)));

    let pending = viewport.begin_frame(Size::new(80, 2));

    assert_eq!(
        pending.plan(),
        InlineFramePlan::Reanchor {
            abandoned_rows: 3,
            current: Size::new(80, 2),
            cursor: Point::new(0, 2),
        }
    );
}

// 재고정 출력도 실패할 수 있으므로 불확실한 기존·새 행 범위를 모두 보존한다.
#[test]
fn failed_reanchor_preserves_the_larger_uncertain_footprint() {
    let mut viewport = InlineViewport::default();
    drop(viewport.begin_frame(Size::new(80, 2)));
    drop(viewport.begin_frame(Size::new(80, 5)));

    let pending = viewport.begin_frame(Size::new(80, 1));

    assert_eq!(
        pending.plan(),
        InlineFramePlan::Reanchor {
            abandoned_rows: 5,
            current: Size::new(80, 1),
            cursor: Point::new(0, 1),
        }
    );
}

// 유효하지 않은 caret은 출력 계획을 시작하기 전에 거부되어 직전 frame 신뢰를 보존한다.
#[test]
fn invalid_caret_preserves_the_trusted_frame() {
    let size = Size::new(8, 3);
    let previous_cursor = Point::new(2, 1);
    let mut viewport = InlineViewport::default();
    viewport
        .begin_frame_at(size, previous_cursor)
        .unwrap()
        .commit();

    let error = viewport.begin_frame_at(size, Point::new(8, 1)).unwrap_err();

    assert_eq!(
        error,
        super::InlineFrameError::CursorOutOfBounds {
            cursor: Point::new(8, 1),
            size,
        }
    );
    assert_eq!(
        viewport
            .begin_frame_at(size, Point::new(3, 1))
            .unwrap()
            .plan(),
        InlineFramePlan::Update {
            current: size,
            previous_cursor,
            cursor: Point::new(3, 1),
        }
    );
}
