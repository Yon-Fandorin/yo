use std::time::Duration;

use yo_core::{AgentCommand, SubmissionId, TranscriptRecord, UserInput};

use super::{key, rendered_row, turn};
use crate::{
    input::event::{InputEvent, KeyCode, KeyModifiers},
    runner::{
        AgentAction,
        state::{StateEffect, TuiState},
    },
    surface::Size,
};

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

// Enter는 immutable snapshot을 queue에 보내도 prompt를 즉시 비우지 않는다. 같은 ID의
// admission Accepted가 온 뒤에만 현재 draft를 비우고, Journal commit은 Chat에 한 번 나타난다.
#[test]
fn committed_submitted_prompt_becomes_one_user_transcript_item() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("question".to_owned()), Duration::ZERO)
        .unwrap();

    let StateEffect::Dispatch(AgentAction::Submit(submission)) = state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
    else {
        panic!("Enter should queue one immutable submission");
    };

    assert_eq!(submission.input().as_str(), "question");
    assert_eq!(state.editor().text(), "question");
    assert!(state.transcript().items().is_empty());

    state
        .observe_submission_outcome(yo_core::SubmissionOutcome::Accepted {
            id: submission.id(),
        })
        .unwrap();
    assert_eq!(state.editor().text(), "");

    state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("question"),
            },
        ))
        .unwrap();

    assert_eq!(state.transcript().items().len(), 1);
    assert_eq!(rendered_row(&state, Size::new(12, 3), 0), "❯ question");
}

// active Turn을 보고 작성한 prompt는 generic submit이 아니라 그 TurnRef를 고정한 steer로
// 전달되어, core가 poll 지연 뒤 새 Turn으로 재해석할 수 없다.
#[test]
fn active_turn_submission_carries_the_exact_observed_turn() {
    let mut state = TuiState::new();
    let observed = turn();
    state
        .observe(yo_core::AgentEvent::TurnStarted { turn: observed })
        .unwrap();
    state
        .handle(
            InputEvent::Paste("focus on tests".to_owned()),
            Duration::ZERO,
        )
        .unwrap();

    let StateEffect::Dispatch(AgentAction::Steer { turn, submission }) = state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
    else {
        panic!("active-Turn input must retain its observed TurnRef");
    };
    assert_eq!(turn, observed);
    assert_eq!(submission.input().as_str(), "focus on tests");
}

// admission이 진행되는 동안 사용자가 draft를 고치면 이전 snapshot의 Accepted는 그 새
// 편집을 지우지 않고, 같은 ID 결과를 다시 받아도 이미 소비한 snapshot을 건드리지 않는다.
#[test]
fn accepted_older_submission_preserves_a_newer_draft_and_ignores_duplicates() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("first".to_owned()), Duration::ZERO)
        .unwrap();
    let StateEffect::Dispatch(AgentAction::Submit(submission)) = state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
    else {
        panic!("Enter should queue the first snapshot");
    };
    state
        .handle(InputEvent::Paste(" revised".to_owned()), Duration::ZERO)
        .unwrap();

    let outcome = yo_core::SubmissionOutcome::Accepted {
        id: submission.id(),
    };
    assert_eq!(
        state.observe_submission_outcome(outcome.clone()).unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.editor().text(), "first revised");
    assert_eq!(
        state.observe_submission_outcome(outcome).unwrap(),
        StateEffect::Unchanged
    );
    assert_eq!(state.editor().text(), "first revised");
}

// Rejected는 snapshot과 현재 editor를 모두 소비하지 않으며, 같은 draft의 연속 Enter는
// 첫 admission이 끝나기 전 중복 Backend 요청을 만들지 않는다.
#[test]
fn pending_duplicate_and_rejection_preserve_the_exact_draft() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("question".to_owned()), Duration::ZERO)
        .unwrap();
    let StateEffect::Dispatch(AgentAction::Submit(submission)) = state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
    else {
        panic!("Enter should queue one snapshot");
    };
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );

    state
        .observe_submission_outcome(yo_core::SubmissionOutcome::Rejected {
            id: submission.id(),
            rejection: yo_core::SubmissionRejection::new(
                yo_core::SubmissionRejectionKind::StaleReference,
                "select the reference again",
            ),
        })
        .unwrap();

    assert_eq!(state.editor().text(), "question");
}

// 아직 queue에 없는 receipt는 대기 중인 immutable snapshot을 소비하지 않으며, 같은 draft의
// 다음 Enter도 중복 admission으로 남아 실제 receipt가 올 때까지 snapshot을 보존한다.
#[test]
fn unknown_submission_receipt_does_not_consume_pending_snapshot() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("question".to_owned()), Duration::ZERO)
        .unwrap();
    let StateEffect::Dispatch(AgentAction::Submit(submission)) = state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
    else {
        panic!("Enter should queue one immutable snapshot");
    };

    let unknown = loop {
        let candidate = SubmissionId::new().unwrap();
        if candidate != submission.id() {
            break candidate;
        }
    };
    assert_eq!(
        state
            .observe_submission_outcome(yo_core::SubmissionOutcome::Accepted { id: unknown })
            .unwrap(),
        StateEffect::Unchanged
    );
    assert_eq!(state.editor().text(), "question");
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );

    state
        .observe_submission_outcome(yo_core::SubmissionOutcome::Accepted {
            id: submission.id(),
        })
        .unwrap();
    assert_eq!(state.editor().text(), "");
}
