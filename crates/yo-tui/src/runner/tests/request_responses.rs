use std::time::Duration;

use yo_core::{ActivityKind, ActivityRequestRef, AgentEvent, ApprovalDecision, RequestId};

use super::{activity, key, nonzero};
use crate::{
    appearance::AppearanceState,
    input::event::{InputEvent, KeyCode, KeyModifiers},
    runner::{
        AgentAction,
        state::{StateEffect, TuiState},
    },
    surface::Size,
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

// pending Activity가 있어도 /help는 로컬에서 실행되고 request를 답하거나 취소하지 않는다.
#[test]
fn local_help_does_not_answer_an_outstanding_activity() {
    let mut state = TuiState::new();
    let request_activity = activity(1);
    let request_id = RequestId::new(nonzero(9));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::UserInputRequest { request_id },
        })
        .unwrap();
    state
        .handle(InputEvent::Paste("/help".to_owned()), Duration::ZERO)
        .unwrap();

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert!(state.has_pending_request());
    assert!(state.editor().text().is_empty());
    let output = state
        .session_output(&AppearanceState::default().pin())
        .unwrap()
        .unwrap();
    assert!(output.contains("Available commands"), "{output}");
}

// cursor 뒤 text 때문에 palette가 소유하지 않는 slash draft는 known invocation이어도
// local command로 재분류되지 않고 대기 중인 Activity의 correlated response가 된다.
#[test]
fn cursor_ineligible_command_draft_answers_the_outstanding_activity() {
    let mut state = TuiState::new();
    let request_activity = activity(1);
    let request_id = RequestId::new(nonzero(13));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::UserInputRequest { request_id },
        })
        .unwrap();
    state
        .handle(InputEvent::Paste("/help".to_owned()), Duration::ZERO)
        .unwrap();
    state
        .handle(key(KeyCode::Left, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();

    let frame = state
        .prepare_frame(Size::new(80, 16), &AppearanceState::default().pin())
        .unwrap();
    assert!(!frame.overlay_presented);
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::RespondToUserInput {
            request: ActivityRequestRef::new(request_activity, request_id),
            input: "/help".to_owned(),
        })
    );
}

// pending Activity 중 unknown slash draft도 표시된 palette를 Esc로 닫은 경우에만 원래
// Activity의 correlated input response로 전달된다.
#[test]
fn escaped_command_draft_answers_the_outstanding_activity() {
    let mut state = TuiState::new();
    let request_activity = activity(1);
    let request_id = RequestId::new(nonzero(10));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::UserInputRequest { request_id },
        })
        .unwrap();
    state
        .handle(InputEvent::Paste("/foo".to_owned()), Duration::ZERO)
        .unwrap();
    let frame = state
        .prepare_frame(Size::new(80, 16), &AppearanceState::default().pin())
        .unwrap();
    assert!(frame.overlay_presented);
    state.commit_frame(&frame);
    assert_eq!(
        state
            .handle(key(KeyCode::Escape, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::RespondToUserInput {
            request: ActivityRequestRef::new(request_activity, request_id),
            input: "/foo".to_owned(),
        })
    );
}

// /exit는 process lifecycle 명령이므로 pending Activity를 답으로 소비하지 않고 즉시
// 기존 runner 종료 경계를 사용한다.
#[test]
fn exit_remains_an_explicit_process_lifecycle_exception_during_activity() {
    let mut state = TuiState::new();
    let request_activity = activity(1);
    let request_id = RequestId::new(nonzero(12));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::UserInputRequest { request_id },
        })
        .unwrap();
    state
        .handle(InputEvent::Paste("/exit".to_owned()), Duration::ZERO)
        .unwrap();

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Exit
    );
    assert!(state.has_pending_request());
}
