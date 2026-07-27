use super::super::{InlineRestorePlan, InlineViewport};
use crate::surface::{Point, Size};

// 신뢰 가능한 viewport 종료는 정확히 그 소유 크기를 지우는 계획을 만든다.
#[test]
fn owned_viewport_plans_an_owned_clear() {
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(Size::new(80, 3)).commit();

    let pending = viewport.begin_restore();

    assert_eq!(
        pending.plan(),
        InlineRestorePlan::ClearOwned {
            size: Size::new(80, 3),
            cursor: Point::new(0, 3),
        }
    );
}

// clear가 완료되기 전에 실패하면 다음 복구는 같은 행을 다시 지우지 않고 미확인으로 남긴다.
#[test]
fn interrupted_clear_never_reauthorizes_erasing_the_region() {
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(Size::new(80, 3)).commit();
    drop(viewport.begin_restore());

    let retry = viewport.begin_restore();

    assert_eq!(
        retry.plan(),
        InlineRestorePlan::LeaveUntrusted { abandoned_rows: 3 }
    );
}

// 성공한 복구 commit은 상태를 비워 반복 호출을 무해하게 만든다.
#[test]
fn committed_restore_is_idempotent() {
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(Size::new(80, 3)).commit();
    viewport.begin_restore().commit();

    let retry = viewport.begin_restore();

    assert_eq!(retry.plan(), InlineRestorePlan::Nothing);
}
