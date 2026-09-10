use std::time::Duration;

use super::{TuiState, key, turn};
use crate::{
    appearance::AppearanceState,
    input::event::{InputEvent, KeyCode, KeyModifiers},
    runner::state::StateEffect,
    surface::Size,
};

fn send(state: &mut TuiState, text: &str) -> StateEffect {
    state
        .handle(InputEvent::Paste(text.to_owned()), Duration::ZERO)
        .unwrap();
    let frame = state
        .prepare_frame(Size::new(88, 25), &AppearanceState::default().pin())
        .unwrap();
    state.commit_frame(&frame);
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
}

// /preview 입력은 실제 agent dispatch로 새지 않고, 합성 대화는 원본 출력에 들어가지 않는다.
#[test]
fn preview_is_interactive_isolated_and_returns_to_real_session() {
    let mut state = TuiState::new();
    state
        .observe_record(yo_core::TranscriptRecord::CommandCommitted(
            yo_core::AgentCommand::StartTurn {
                turn: turn(),
                input: "original conversation".into(),
            },
        ))
        .unwrap();
    let pin = AppearanceState::default().pin();
    let original = state.session_output(&pin).unwrap();
    assert_eq!(send(&mut state, "/preview"), StateEffect::Redraw);
    assert!(state.preview_active());
    assert_eq!(send(&mut state, "hello sandbox"), StateEffect::Redraw);
    let frame = state
        .prepare_frame_for_geometry(
            Size::new(88, 25),
            &AppearanceState::default().pin(),
            Duration::ZERO,
            0,
        )
        .unwrap();
    assert!(frame.publication.is_none());
    assert_eq!(state.session_output(&pin).unwrap(), original);
    assert_eq!(send(&mut state, "/preview"), StateEffect::Redraw);
    assert!(!state.preview_active());
    assert_eq!(state.session_output(&pin).unwrap(), original);
    assert!(matches!(
        send(&mut state, "real message"),
        StateEffect::Dispatch(_)
    ));
}

// 실제 작업 중에는 preview 진입을 막아 이미 실행 중인 작업을 숨기지 않는다.
#[test]
fn active_real_turn_cannot_be_hidden_by_preview() {
    let mut state = TuiState::new();
    state
        .observe(yo_core::AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    assert_eq!(send(&mut state, "/preview"), StateEffect::Redraw);
    assert!(!state.preview_active());
}
