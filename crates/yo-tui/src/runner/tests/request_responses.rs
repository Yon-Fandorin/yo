use std::time::Duration;

use yo_core::{ActivityKind, ActivityRequestRef, AgentEvent, ApprovalDecision, RequestId};

use super::{activity, key, nonzero};
use crate::{
    input::event::{InputEvent, KeyCode, KeyModifiers},
    runner::{
        AgentAction,
        state::{StateEffect, TuiState},
    },
};

// outstanding approval이 있을 때 `y` 제출은 새 Turn이나 steer가 아니라 원래 Activity와
// request ID를 가진 승인 응답 action이 된다.
#[test]
fn converts_yes_into_a_correlated_approval_response() {
    let mut state = TuiState::new();
    let request_activity = activity(1);
    let request_id = RequestId::new(nonzero(7));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::ApprovalRequest { request_id },
        })
        .unwrap();
    state
        .handle(InputEvent::Paste("y".to_owned()), Duration::ZERO)
        .unwrap();

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::RespondToApproval {
            request: ActivityRequestRef::new(request_activity, request_id),
            decision: ApprovalDecision::Approved,
        })
    );
}

// agent가 추가 입력을 요청한 동안 제출한 문자열은 활성 Turn steer가 아니라 원래
// Activity와 request ID를 가진 UserInput 응답 action으로 변환된다.
#[test]
fn converts_text_into_a_correlated_agent_input_response() {
    let mut state = TuiState::new();
    let request_activity = activity(1);
    let request_id = RequestId::new(nonzero(8));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::UserInputRequest { request_id },
        })
        .unwrap();
    state
        .handle(
            InputEvent::Paste("use the second option".to_owned()),
            Duration::ZERO,
        )
        .unwrap();

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::RespondToUserInput {
            request: ActivityRequestRef::new(request_activity, request_id),
            input: "use the second option".to_owned(),
        })
    );
}

// 서로 다른 Activity의 approval·user-input request가 동시에 대기하면 첫 Enter는 queue 앞의
// request에만 응답하고, 다음 Enter는 뒤의 request와 그 request ID를 그대로 상관시킨다.
#[test]
fn multiple_pending_requests_are_answered_fifo_with_their_own_correlations() {
    let mut state = TuiState::new();
    let approval_activity = activity(1);
    let approval_id = RequestId::new(nonzero(7));
    let input_activity = activity(2);
    let input_id = RequestId::new(nonzero(8));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: approval_activity,
            kind: ActivityKind::ApprovalRequest {
                request_id: approval_id,
            },
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityStarted {
            activity: input_activity,
            kind: ActivityKind::UserInputRequest {
                request_id: input_id,
            },
        })
        .unwrap();

    state
        .handle(InputEvent::Paste("y".to_owned()), Duration::ZERO)
        .unwrap();
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::RespondToApproval {
            request: ActivityRequestRef::new(approval_activity, approval_id),
            decision: ApprovalDecision::Approved,
        })
    );
    assert!(state.has_pending_request());

    state
        .handle(
            InputEvent::Paste("use the second option".to_owned()),
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::RespondToUserInput {
            request: ActivityRequestRef::new(input_activity, input_id),
            input: "use the second option".to_owned(),
        })
    );
    assert!(!state.has_pending_request());
}
