use std::time::Duration;

use super::{
    ExitReason, RunOutcome,
    state::{StateEffect, StateError, TuiState},
    unix::prepare_resize,
};
use crate::{
    input::event::{InputEvent, KeyAction, KeyCode, KeyEvent as YoKeyEvent, KeyState},
    surface::{CellContent, Point, Size},
    terminal::mode::inline::{InlineFramePlan, InlineViewport},
};

fn key(code: KeyCode, modifiers: crate::input::event::KeyModifiers) -> InputEvent {
    InputEvent::Key(YoKeyEvent {
        code,
        modifiers,
        action: KeyAction::Press,
        state: KeyState::NONE,
    })
}

fn rendered_row(state: &TuiState, size: Size, y: u16) -> String {
    let frame = state.prepare_frame(size).unwrap();
    (0..size.width)
        .map(
            |x| match frame.surface.cell(Point::new(x, y)).unwrap().content() {
                CellContent::Blank | CellContent::Continuation { .. } => ' ',
                CellContent::Grapheme { text, .. } => text.chars().next().unwrap(),
            },
        )
        .collect::<String>()
        .trim_end()
        .to_owned()
}

// 입력 편집은 화면 상태만 바꾸고 아직 연결되지 않은 에이전트 응답을 만들지 않는다.
#[test]
fn edits_prompt_without_creating_transcript_items() {
    let mut state = TuiState::new();

    assert_eq!(
        state
            .handle(InputEvent::Paste("질문".to_owned()), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.editor().text(), "질문");
    assert!(state.transcript().items().is_empty());
}

// Enter로 제출한 입력은 user transcript에 한 번 남고 prompt는 비워진다.
#[test]
fn submitted_prompt_becomes_one_user_transcript_item() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("question".to_owned()), Duration::ZERO)
        .unwrap();

    assert_eq!(
        state
            .handle(
                key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Redraw
    );

    assert_eq!(state.editor().text(), "");
    assert_eq!(state.transcript().items().len(), 1);
    assert_eq!(rendered_row(&state, Size::new(12, 3), 0), "❯ question");
}

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
    let previous = crate::surface::Surface::new(old).unwrap();
    let current = crate::surface::Surface::new(next).unwrap();
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

// 비어 있는 prompt의 Ctrl+D는 runner가 정상 종료할 명시적인 effect다.
#[test]
fn empty_ctrl_d_requests_normal_exit() {
    let mut state = TuiState::new();

    assert_eq!(
        state
            .handle(
                key(
                    KeyCode::Character('d'),
                    crate::input::event::KeyModifiers::CONTROL,
                ),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Exit
    );
}

// transcript ID가 더는 증가할 수 없으면 제출 내용을 중복 ID로 넣지 않고 실패한다.
#[test]
fn item_id_overflow_preserves_empty_transcript() {
    let mut state = TuiState::new();
    state.set_next_item_id(u64::MAX);
    state
        .handle(InputEvent::Paste("질문".to_owned()), Duration::ZERO)
        .unwrap();

    assert_eq!(
        state.handle(
            key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
            Duration::ZERO,
        ),
        Err(StateError::ItemIdOverflow)
    );
    assert!(state.transcript().items().is_empty());
    assert_eq!(state.editor().text(), "질문");
}

// public outcome은 프로세스를 직접 종료하지 않고 정상 종료 이유를 반환한다.
#[test]
fn public_outcome_exposes_user_exit_reason() {
    assert_eq!(
        RunOutcome::user_requested().reason(),
        ExitReason::UserRequested
    );
}

// host 종료 요청은 OS signal identity를 노출하지 않고 별도 정상 종료 이유로 반환한다.
#[test]
fn public_outcome_exposes_host_termination_reason() {
    assert_eq!(
        RunOutcome::termination_requested().reason(),
        ExitReason::TerminationRequested
    );
}
