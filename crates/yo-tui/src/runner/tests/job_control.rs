use std::time::Duration;

use super::key;
use crate::{
    input::event::{
        InputEvent, KeyAction, KeyCode, KeyEvent as YoKeyEvent, KeyModifiers, KeyState,
    },
    runner::{
        state::{StateEffect, TuiState},
        unix::handle_backpressured_input,
    },
};

// Ctrl+Z의 최초 key press는 editor 내용이나 활성 Turn 여부와 무관하게 terminal 소유권
// 세대를 닫는 일시정지 요청으로 분리한다.
#[test]
fn ctrl_z_press_requests_terminal_suspension() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("draft".to_owned()), Duration::ZERO)
        .unwrap();

    assert_eq!(
        state
            .handle(
                key(KeyCode::Character('z'), KeyModifiers::CONTROL),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Suspend
    );
    assert_eq!(state.editor().text(), "draft");
}

// enhanced keyboard protocol이 보내는 Ctrl+Z repeat와 release는 새 일시정지 요청으로
// 세지 않고 최초 press 하나만 경계 신호로 사용한다.
#[test]
fn ctrl_z_repeat_and_release_do_not_request_another_suspension() {
    for action in [KeyAction::Repeat, KeyAction::Release] {
        let mut state = TuiState::new();
        let input = InputEvent::Key(YoKeyEvent {
            code: KeyCode::Character('z'),
            modifiers: KeyModifiers::CONTROL,
            action,
            state: KeyState::NONE,
        });

        assert_eq!(
            state.handle(input, Duration::ZERO).unwrap(),
            StateEffect::Unchanged
        );
    }
}

// Ctrl 외에 Shift나 Alt가 함께 눌린 변형은 shell job-control 명령으로 추측하지 않고
// editor의 미지원 입력으로 남긴다.
#[test]
fn modified_ctrl_z_is_not_treated_as_job_control() {
    for modifiers in [
        KeyModifiers::CONTROL.union(KeyModifiers::SHIFT),
        KeyModifiers::CONTROL.union(KeyModifiers::ALT),
    ] {
        let mut state = TuiState::new();

        assert_eq!(
            state
                .handle(key(KeyCode::Character('z'), modifiers), Duration::ZERO)
                .unwrap(),
            StateEffect::Unchanged
        );
    }
}

// command lane이 가득 차 있어도 Ctrl+Z는 일반 입력처럼 버려지지 않고 terminal 복구
// 경계까지 전달된다.
#[test]
fn backpressure_still_services_ctrl_z_suspension() {
    let mut state = TuiState::new();

    assert_eq!(
        handle_backpressured_input(
            &mut state,
            key(KeyCode::Character('z'), KeyModifiers::CONTROL),
            Duration::ZERO,
            false,
        )
        .unwrap(),
        StateEffect::Suspend
    );
}
