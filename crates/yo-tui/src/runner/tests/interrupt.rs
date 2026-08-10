use std::time::Duration;

use yo_core::AgentEvent;

use super::{key, turn};
use crate::{
    input::event::{KeyCode, KeyModifiers},
    runner::{
        AgentAction,
        state::{StateEffect, TuiState},
        unix::handle_backpressured_input,
    },
};

// TurnStarted 뒤 Ctrl+C는 process exit sequence를 시작하지 않고 해당 agent 작업의
// interrupt intent를 한 번 전달한다.
#[test]
fn active_turn_ctrl_c_dispatches_interrupt() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();

    assert_eq!(
        state
            .handle(
                key(KeyCode::Character('c'), KeyModifiers::CONTROL),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Dispatch(AgentAction::Interrupt)
    );
}

// TurnStarted 뒤 Esc도 Ctrl+C와 같은 interrupt intent를 전달하며 종료 동작으로 새지 않는다.
#[test]
fn active_turn_escape_dispatches_interrupt() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();

    assert_eq!(
        state
            .handle(key(KeyCode::Escape, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::Interrupt)
    );
}

// command lane이 가득 찬 동안에도 runner의 제한 입력 경로는 Ctrl+C를 버리지 않고 활성
// Turn interrupt로 해석해 우선 control lane에 전달할 수 있게 한다.
#[test]
fn backpressure_still_services_active_turn_ctrl_c() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();

    assert_eq!(
        handle_backpressured_input(
            &mut state,
            key(KeyCode::Character('c'), KeyModifiers::CONTROL),
            Duration::ZERO,
            false,
        )
        .unwrap(),
        StateEffect::Dispatch(AgentAction::Interrupt)
    );
}

// command lane이 가득 차도 활성 Turn의 Esc는 일반 입력처럼 버려지지 않고 control lane으로 간다.
#[test]
fn backpressure_still_services_active_turn_escape() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();

    assert_eq!(
        handle_backpressured_input(
            &mut state,
            key(KeyCode::Escape, KeyModifiers::NONE),
            Duration::ZERO,
            false,
        )
        .unwrap(),
        StateEffect::Dispatch(AgentAction::Interrupt)
    );
}
