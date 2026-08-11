use std::time::Duration;

use yo_core::{AgentCommand, TranscriptRecord, UserInput};

use super::{
    super::{key, turn},
    support::{function, render_and_commit},
};
use crate::{
    appearance::AppearanceState,
    input::event::{KeyAction, KeyCode, KeyModifiers},
    runner::state::{StateEffect, TuiState},
    surface::Size,
};

// Chat, Transcript, Request에서 각각 분리된 viewport를 움직인 뒤 mode를 왕복하면 같은
// anchor일 때 각 first-visible-row가 복원되어 다른 view의 scroll이 덮어쓰지 않는다.
#[test]
fn switching_restores_each_view_local_scroll_state() {
    let mut state = TuiState::new();
    for index in 0..12 {
        state
            .observe_record(TranscriptRecord::CommandCommitted(
                AgentCommand::StartTurn {
                    turn: turn(),
                    input: UserInput::from(format!("question {index}")),
                },
            ))
            .unwrap();
    }
    let size = Size::new(12, 5);
    render_and_commit(&mut state, size);
    state
        .handle(key(KeyCode::PageUp, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    render_and_commit(&mut state, size);

    state
        .handle(function(2, KeyAction::Press), Duration::ZERO)
        .unwrap();
    render_and_commit(&mut state, size);
    state
        .handle(key(KeyCode::PageUp, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    render_and_commit(&mut state, size);

    state
        .handle(function(3, KeyAction::Press), Duration::ZERO)
        .unwrap();
    render_and_commit(&mut state, size);
    state
        .handle(key(KeyCode::PageUp, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    render_and_commit(&mut state, size);
    let detached = state.views().view_positions();
    assert!(detached.0 > 0);
    assert!(detached.1 > 0);
    assert!(detached.2 > 0);

    for mode in [1, 2, 3] {
        state
            .handle(function(mode, KeyAction::Press), Duration::ZERO)
            .unwrap();
        render_and_commit(&mut state, size);
    }
    assert_eq!(state.views().view_positions(), detached);
}

// 유효한 frame을 commit한 뒤 준비가 실패해도 committed viewport와 미소비 scroll 의도는 그대로
// 남아, 재시도한 유효 frame에서만 다음 viewport가 적용된다.
#[test]
fn failed_frame_preparation_keeps_committed_viewport_and_pending_scroll() {
    let mut state = TuiState::new();
    for index in 0..12 {
        state
            .observe_record(TranscriptRecord::CommandCommitted(
                AgentCommand::StartTurn {
                    turn: turn(),
                    input: UserInput::from(format!("question {index}")),
                },
            ))
            .unwrap();
    }
    let size = Size::new(18, 5);
    let first = state
        .prepare_frame(size, &AppearanceState::default().pin())
        .unwrap();
    state.commit_frame(&first);
    let committed = state.views().view_positions();

    assert_eq!(
        state.handle(key(KeyCode::PageUp, KeyModifiers::NONE), Duration::ZERO),
        Ok(StateEffect::Redraw)
    );
    assert!(state.views().chat_has_pending_scroll());
    assert!(
        state
            .prepare_frame(Size::new(size.width, 0), &AppearanceState::default().pin(),)
            .is_err()
    );
    assert_eq!(state.views().view_positions(), committed);
    assert!(state.views().chat_has_pending_scroll());

    let retry = state
        .prepare_frame(size, &AppearanceState::default().pin())
        .unwrap();
    state.commit_frame(&retry);
    assert!(state.views().view_positions().0 < committed.0);
}
