use std::time::Duration;

use yo_core::{
    ActivityKind, ActivityQuestion, ActivityUpdate, AgentEvent, SubmissionId, SubmissionOutcome,
    TurnId, TurnOutcome, TurnRef,
};

use super::{key, nonzero, turn};
use crate::{
    appearance::AppearanceState,
    input::event::{InputEvent, KeyCode, KeyModifiers},
    runner::{
        AgentAction,
        state::{StateEffect, TuiState},
    },
    surface::Size,
};

fn present_request(state: &mut TuiState) {
    let frame = state
        .prepare_frame(Size::new(80, 24), &AppearanceState::default().pin())
        .unwrap();
    state.commit_frame(&frame);
}

fn arm_with_initial_submission(state: &mut TuiState) -> SubmissionId {
    state.set_notifications_enabled(true);
    state
        .handle(InputEvent::Paste("start".to_owned()), Duration::ZERO)
        .unwrap();
    let StateEffect::Dispatch(AgentAction::Submit(submission)) = state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
    else {
        panic!("the initial prompt must dispatch a submission");
    };
    submission.id()
}

// 사용자가 시작하지 않은 resume history는 completion을 관찰해도 알림을 울리지 않습니다.
#[test]
fn historical_lifecycle_is_silent_until_a_live_dispatch_arms_notifications() {
    let mut state = TuiState::new();
    state.set_notifications_enabled(true);
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    state
        .observe(AgentEvent::TurnFinished {
            turn: turn(),
            outcome: TurnOutcome::Completed,
        })
        .unwrap();

    assert!(!state.take_attention_bell(false));
}

// 복원된 Turn 기록이 새 입력 뒤늦게 도착해도 과거 완료나 요청으로 알리지 않습니다.
#[test]
fn restored_history_cutoff_ignores_interleaved_old_events() {
    let mut state = TuiState::new();
    state.set_notification_history_cutoff(Some(turn()));
    let submission = arm_with_initial_submission(&mut state);
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    state
        .observe_submission_outcome(SubmissionOutcome::Accepted { id: submission })
        .unwrap();
    let old_activity = super::activity(1);
    state
        .observe(AgentEvent::ActivityStarted {
            activity: old_activity,
            kind: ActivityKind::ApprovalRequest {
                request_id: yo_core::RequestId::new(nonzero(1)),
            },
        })
        .unwrap();
    present_request(&mut state);
    assert!(!state.take_attention_bell(false));
    state
        .observe(AgentEvent::ActivityFinished {
            activity: old_activity,
            outcome: yo_core::ActivityOutcome::Completed,
        })
        .unwrap();
    state
        .observe(AgentEvent::TurnFinished {
            turn: turn(),
            outcome: TurnOutcome::Completed,
        })
        .unwrap();
    assert!(!state.take_attention_bell(false));

    let next = TurnRef::new(turn().session_id(), TurnId::new(nonzero(2)));
    state
        .observe(AgentEvent::TurnStarted { turn: next })
        .unwrap();
    state
        .observe(AgentEvent::TurnFinished {
            turn: next,
            outcome: TurnOutcome::Completed,
        })
        .unwrap();
    assert!(state.take_attention_bell(false));
}

// 최초 입력을 Alt+Q로 queue에 넣은 경우에도 실제 dispatch가 알림을 활성화합니다.
#[test]
fn first_queued_submission_arms_completion_bell() {
    let mut state = TuiState::new();
    state.set_notifications_enabled(true);
    state
        .handle(InputEvent::Paste("queued".to_owned()), Duration::ZERO)
        .unwrap();
    assert_eq!(
        state
            .handle(
                key(KeyCode::Character('q'), KeyModifiers::ALT),
                Duration::ZERO
            )
            .unwrap(),
        StateEffect::Redraw
    );
    let Some(AgentAction::Submit(submission)) = state.next_follow_up().unwrap() else {
        panic!("first queued prompt must dispatch");
    };
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    state
        .observe_submission_outcome(SubmissionOutcome::Accepted {
            id: submission.id(),
        })
        .unwrap();
    state
        .observe(AgentEvent::TurnFinished {
            turn: turn(),
            outcome: TurnOutcome::Completed,
        })
        .unwrap();
    assert!(state.take_attention_bell(false));
}

// live Turn이 마지막으로 끝나면 한 번만 알리고, 같은 완료 기록을 다시 받아도 반복하지 않습니다.
#[test]
fn completed_live_turn_rings_once_after_reaching_idle() {
    let mut state = TuiState::new();
    let submission = arm_with_initial_submission(&mut state);
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    state
        .observe_submission_outcome(SubmissionOutcome::Accepted { id: submission })
        .unwrap();
    let finished = AgentEvent::TurnFinished {
        turn: turn(),
        outcome: TurnOutcome::Completed,
    };
    state.observe(finished.clone()).unwrap();

    assert!(state.take_attention_bell(false));
    assert!(!state.take_attention_bell(false));
    state.observe(finished).unwrap();
    assert!(!state.take_attention_bell(false));
}

// completion 직후 queue가 자동으로 admission되면 안정된 idle 전까지 completion BEL을 보류합니다.
#[test]
fn queued_follow_up_suppresses_intermediate_completion_bell() {
    let mut state = TuiState::new();
    let initial_submission = arm_with_initial_submission(&mut state);
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    state
        .observe_submission_outcome(SubmissionOutcome::Accepted {
            id: initial_submission,
        })
        .unwrap();
    state
        .handle(InputEvent::Paste("later".to_owned()), Duration::ZERO)
        .unwrap();
    assert_eq!(
        state
            .handle(
                key(KeyCode::Character('q'), KeyModifiers::ALT),
                Duration::ZERO
            )
            .unwrap(),
        StateEffect::Redraw
    );
    state
        .observe(AgentEvent::TurnFinished {
            turn: turn(),
            outcome: TurnOutcome::Completed,
        })
        .unwrap();

    assert!(!state.take_attention_bell(false));
    assert!(matches!(
        state.next_follow_up().unwrap(),
        Some(AgentAction::Submit(_))
    ));
    assert!(!state.take_attention_bell(false));
}

// UserInputRequest는 typed presentation이 도착한 뒤 front panel이 actionable일 때만 한 번 알립니다.
#[test]
fn question_presentation_arms_one_deduplicated_request_bell() {
    let mut state = TuiState::new();
    let _initial_submission = arm_with_initial_submission(&mut state);
    let request = super::activity(1);
    let request_id = yo_core::RequestId::new(nonzero(1));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: request,
            kind: ActivityKind::UserInputRequest { request_id },
        })
        .unwrap();
    assert!(!state.take_attention_bell(false));
    let presentation = ActivityQuestion {
        plain_text: "Type your answer".to_owned(),
        choices: Vec::new(),
        allow_notes: false,
        is_secret: false,
        storage_offer: None,
        previous_question: false,
        draft: None,
        draft_choice: None,
    }
    .to_snapshot()
    .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: request,
            update: ActivityUpdate::TextSnapshot(presentation.clone()),
        })
        .unwrap();
    assert!(!state.take_attention_bell(false));
    present_request(&mut state);
    assert!(state.take_attention_bell(false));
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: request,
            update: ActivityUpdate::TextSnapshot(presentation),
        })
        .unwrap();
    assert!(!state.take_attention_bell(false));
}

// 비밀 질문은 typed 내용이 화면에 반영되기 전에는 알리지 않고 한 번만 울립니다.
#[test]
fn secret_question_waits_for_committed_frame() {
    let mut state = TuiState::new();
    let _submission = arm_with_initial_submission(&mut state);
    let activity = super::activity(1);
    let request_id = yo_core::RequestId::new(nonzero(1));
    state
        .observe(AgentEvent::ActivityStarted {
            activity,
            kind: ActivityKind::UserInputRequest { request_id },
        })
        .unwrap();
    let presentation = ActivityQuestion {
        plain_text: "Enter a secret".to_owned(),
        choices: Vec::new(),
        allow_notes: false,
        is_secret: true,
        storage_offer: None,
        previous_question: false,
        draft: None,
        draft_choice: None,
    }
    .to_snapshot()
    .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextSnapshot(presentation),
        })
        .unwrap();
    assert!(!state.take_attention_bell(false));
    present_request(&mut state);
    assert!(state.take_attention_bell(false));
    assert!(!state.take_attention_bell(false));
}

// 여러 승인이 쌓이면 현재 요청만 울리고, 다음 요청이 front가 된 뒤 다시 한 번 알립니다.
#[test]
fn queued_approvals_ring_when_each_becomes_actionable() {
    let mut state = TuiState::new();
    let _submission = arm_with_initial_submission(&mut state);
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    let first = super::activity(1);
    let second = super::activity(2);
    state
        .observe(AgentEvent::ActivityStarted {
            activity: first,
            kind: ActivityKind::ApprovalRequest {
                request_id: yo_core::RequestId::new(nonzero(1)),
            },
        })
        .unwrap();
    assert!(!state.take_attention_bell(false));
    present_request(&mut state);
    assert!(state.take_attention_bell(false));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: second,
            kind: ActivityKind::ApprovalRequest {
                request_id: yo_core::RequestId::new(nonzero(2)),
            },
        })
        .unwrap();
    assert!(!state.take_attention_bell(false));
    state
        .observe(AgentEvent::ActivityFinished {
            activity: first,
            outcome: yo_core::ActivityOutcome::Completed,
        })
        .unwrap();
    assert!(!state.take_attention_bell(false));
    present_request(&mut state);
    assert!(state.take_attention_bell(false));
    assert!(!state.take_attention_bell(false));
}
