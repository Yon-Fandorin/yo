use std::time::Duration;

use yo_core::{AgentCommand, TranscriptRecord, UserInput};

use super::{key, turn};
use crate::{
    appearance::{AppearanceState, ColorCapability, MotionPreference},
    input::event::{InputEvent, KeyCode, KeyModifiers},
    runner::{
        ExitReason, RunOutcome,
        session::TuiSession,
        state::{StateEffect, TuiState},
        unix::retained_session_output,
    },
};

// 종료용 출력은 저널에 확정된 Chat만 포함하고 아직 작성 중인 prompt는 섞지 않는다.
#[test]
fn session_output_contains_the_current_chat_without_the_prompt() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("question".to_owned()), Duration::ZERO)
        .unwrap();
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("question"),
            },
        ))
        .unwrap();
    state
        .handle(InputEvent::Paste("draft".to_owned()), Duration::ZERO)
        .unwrap();

    let output = state
        .session_output(&AppearanceState::default().pin())
        .unwrap()
        .unwrap();

    assert_eq!(output, "❯ question\n");
}

// 보존용 투영의 u16 행 한계를 넘겨도 이미 끝난 사용자 세션을 실패로 바꾸지 않고 출력을
// 생략한다.
#[test]
fn oversized_session_output_does_not_replace_a_successful_exit() {
    let mut retained = TuiSession::new(ColorCapability::Unknown, MotionPreference::Standard);
    retained
        .parts_mut()
        .state
        .handle(
            InputEvent::Paste("\n".repeat(usize::from(u16::MAX) + 1)),
            Duration::ZERO,
        )
        .unwrap();
    retained
        .parts_mut()
        .state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    retained
        .parts_mut()
        .state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("\n".repeat(usize::from(u16::MAX) + 1)),
            },
        ))
        .unwrap();

    assert_eq!(retained_session_output(&retained), None);
}

// 비어 있는 prompt의 Ctrl+D는 runner가 정상 종료할 명시적인 effect다.
#[test]
fn empty_ctrl_d_requests_normal_exit() {
    let mut state = TuiState::new();

    assert_eq!(
        state
            .handle(
                key(KeyCode::Character('d'), KeyModifiers::CONTROL),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Exit
    );
}

// public outcome은 프로세스를 직접 종료하지 않고 정상 종료 이유를 반환한다.
#[test]
fn public_outcome_exposes_user_exit_reason() {
    assert_eq!(
        RunOutcome::user_requested(None).reason(),
        ExitReason::UserRequested
    );
}

// host 종료 요청은 OS signal identity를 노출하지 않고 별도 정상 종료 이유로 반환한다.
#[test]
fn public_outcome_exposes_host_termination_reason() {
    assert_eq!(
        RunOutcome::termination_requested(None).reason(),
        ExitReason::TerminationRequested
    );
}
