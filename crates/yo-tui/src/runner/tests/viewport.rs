use std::time::Duration;

use crate::{
    input::event::InputEvent,
    runner::{
        state::{StateEffect, TuiState},
        unix::prepare_resize,
    },
    surface::{Point, Size, Surface},
    terminal::mode::inline::{InlineFramePlan, InlineViewport},
};

// Resize는 editor의 Ctrl+C 연속 입력 상태를 건드리지 않고 geometry effect로 분리된다.
#[test]
fn resize_is_forwarded_without_mutating_prompt_state() {
    let mut state = TuiState::new();

    assert_eq!(
        state
            .handle(InputEvent::Resize(Size::new(120, 40)), Duration::ZERO)
            .unwrap(),
        StateEffect::Resize(Size::new(120, 40))
    );
    assert!(state.editor().text().is_empty());
}

// 일반 resize는 이전 frame을 근거로 같은 inline 영역에서 전체 재조정한다.
#[test]
fn resize_reconciles_the_owned_inline_viewport_with_the_previous_frame() {
    let old = Size::new(80, 3);
    let next = Size::new(100, 2);
    let mut viewport = InlineViewport::default();
    viewport.begin_frame(old).commit();
    let previous = Surface::new(old).unwrap();
    let current = Surface::new(next).unwrap();
    let mut size = old;

    prepare_resize(&mut viewport, &mut size, next);

    assert_eq!(size, next);
    let pending = viewport.begin_frame(next);
    assert_eq!(
        pending.plan(),
        InlineFramePlan::Reconcile {
            previous: old,
            current: next,
            owned_rows: old.height.max(next.height),
            previous_cursor: Point::new(0, old.height),
            cursor: Point::new(0, next.height),
        }
    );
    let diff = pending.diff(Some(&previous), &current).unwrap();
    assert_eq!(diff.previous_size(), old);
    assert_eq!(diff.current_size(), next);
    assert_eq!(diff.spans().len(), usize::from(next.height));
}
