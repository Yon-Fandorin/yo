use std::{num::NonZeroU64, time::Duration};

use yo_core::{ActivityKind, ActivityRequestRef, AgentEvent, ApprovalDecision, RequestId};

use super::{activity, key};
use crate::{
    input::event::{InputEvent, KeyCode, KeyModifiers},
    runner::{
        AgentAction,
        state::{StateEffect, TuiState},
        unix::handle_backpressured_input,
    },
};

// command lane이 가득 차도 빈 editor의 Ctrl+D는 보통 경로와 똑같이 즉시 정상 종료로
// 해석되어 provider stop과 terminal cleanup 경로에 도달한다.
#[test]
fn backpressure_still_services_empty_ctrl_d_exit() {
    let mut state = TuiState::new();

    assert_eq!(
        handle_backpressured_input(
            &mut state,
            key(KeyCode::Character('d'), KeyModifiers::CONTROL),
            Duration::ZERO,
            false,
        )
        .unwrap(),
        StateEffect::Exit
    );
}

// normal command lane이 가득 차도 이미 관찰한 approval request의 입력과 Enter는 계속
// 처리되어 correlated response를 urgent lane으로 보낼 수 있다.
#[test]
fn backpressure_still_services_a_pending_approval_response() {
    let mut state = TuiState::new();
    let request_activity = activity(1);
    let request_id = RequestId::new(NonZeroU64::new(1).unwrap());
    state
        .observe(AgentEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::ApprovalRequest { request_id },
        })
        .unwrap();
    assert_eq!(
        handle_backpressured_input(
            &mut state,
            InputEvent::Paste("y".to_owned()),
            Duration::ZERO,
            true,
        )
        .unwrap(),
        StateEffect::Redraw
    );

    assert_eq!(
        handle_backpressured_input(
            &mut state,
            key(KeyCode::Enter, KeyModifiers::NONE),
            Duration::ZERO,
            true,
        )
        .unwrap(),
        StateEffect::Dispatch(AgentAction::RespondToApproval {
            request: ActivityRequestRef::new(request_activity, request_id),
            decision: ApprovalDecision::Approved,
        })
    );
}

// 이미 다른 urgent control이 TUI 재시도 slot을 차지한 동안에는 다음 approval 입력을
// 소비하지 않아, state에서 request를 제거한 뒤 response를 잃는 상황을 만들지 않는다.
#[test]
fn backpressure_pauses_request_input_while_another_control_is_retained() {
    let mut state = TuiState::new();
    let request_activity = activity(1);
    let request_id = RequestId::new(NonZeroU64::new(1).unwrap());
    state
        .observe(AgentEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::ApprovalRequest { request_id },
        })
        .unwrap();

    assert_eq!(
        handle_backpressured_input(
            &mut state,
            InputEvent::Paste("y".to_owned()),
            Duration::ZERO,
            false,
        )
        .unwrap(),
        StateEffect::Unchanged
    );
    assert!(state.has_pending_request());
}
