use std::{num::NonZeroU64, time::Duration};

use yo_core::{
    ActivityKind, ActivityOutcome, ActivityRequestRef, ActivityUpdate, AgentCommand, AgentEvent,
    ApprovalDecision, RequestId, TranscriptRecord, UserInput,
};

use super::{
    super::{activity, key, turn},
    support::{function, render_and_commit},
};
use crate::{
    input::event::{KeyAction, KeyCode, KeyModifiers},
    runner::state::TuiState,
    surface::Size,
};

// Request projection은 RespondToActivity의 정확한 ActivityRequestRef만 사용하므로 인접한
// 다른 request가 있어도 현재 anchor의 session/turn/activity/request identity를 바꾸지 않는다.
#[test]
fn request_anchor_never_falls_through_to_a_nearby_request() {
    let mut state = TuiState::new();
    let first = ActivityRequestRef::new(activity(4), RequestId::new(NonZeroU64::new(8).unwrap()));
    state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::RespondToActivity {
                request: first,
                response: yo_core::ActivityResponse::Approval(ApprovalDecision::Approved),
            },
        ))
        .unwrap();
    state
        .observe_record(TranscriptRecord::EventCommitted(AgentEvent::TurnStarted {
            turn: turn(),
        }))
        .unwrap();
    state
        .handle(function(2, KeyAction::Press), Duration::ZERO)
        .unwrap();
    render_and_commit(&mut state, Size::new(72, 10));
    state
        .handle(function(3, KeyAction::Press), Duration::ZERO)
        .unwrap();

    let request = render_and_commit(&mut state, Size::new(72, 10));
    assert!(request.contains("context_highlight=none(reason=no-direct-request)"));
    assert!(request.contains("event.turn_started"));
    assert!(!request.contains("request=8"));
}

// Chat에 보이지 않는 TurnStarted가 뒤에 추가되어도 마지막으로 실제 표시된 approval
// item의 문맥을 유지하므로 Request anchor가 숨은 Journal record로 밀리지 않는다.
#[test]
fn chat_context_ignores_hidden_records_after_the_visible_item() {
    let mut state = TuiState::new();
    let request_id = RequestId::new(NonZeroU64::new(13).unwrap());
    state
        .observe_record(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityStarted {
                activity: activity(6),
                kind: ActivityKind::ApprovalRequest { request_id },
            },
        ))
        .unwrap();
    state
        .observe_record(TranscriptRecord::EventCommitted(AgentEvent::TurnStarted {
            turn: turn(),
        }))
        .unwrap();

    let chat = render_and_commit(&mut state, Size::new(40, 8));
    assert!(!chat.contains("Chat ·"));
    state
        .handle(function(3, KeyAction::Press), Duration::ZERO)
        .unwrap();
    let request = render_and_commit(&mut state, Size::new(72, 10));

    assert!(request.contains("context_record=1"));
    assert!(request.contains("activity=6 request=13"));
    assert!(!request.contains("event.turn_started"));
}

// approval item의 완료 처리는 phase만 finalize하고 보이는 text를 바꾸지 않으므로 최초
// request correlation을 ActivityFinished record로 덮지 않고 정확한 Request anchor를 보존한다.
#[test]
fn lifecycle_only_finish_preserves_visible_request_correlation() {
    let mut state = TuiState::new();
    let request_id = RequestId::new(NonZeroU64::new(17).unwrap());
    let request_activity = activity(10);
    state
        .observe_record(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityStarted {
                activity: request_activity,
                kind: ActivityKind::ApprovalRequest { request_id },
            },
        ))
        .unwrap();
    state
        .observe_record(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityFinished {
                activity: request_activity,
                outcome: ActivityOutcome::Completed,
            },
        ))
        .unwrap();

    render_and_commit(&mut state, Size::new(48, 8));
    state
        .handle(function(3, KeyAction::Press), Duration::ZERO)
        .unwrap();
    let request = render_and_commit(&mut state, Size::new(72, 10));

    assert!(request.contains("context_record=1"));
    assert!(request.contains("activity=10 request=17"));
    assert!(!request.contains("event.activity_finished"));
}

// 고빈도 delta는 매 record마다 전체 Chat item을 복사·재검색하지 않고 변경된 단일 item만
// typed signal로 갱신하므로 512회 streaming 뒤에도 context map은 한 항목으로 제한된다.
#[test]
fn high_frequency_stream_updates_one_local_chat_context_entry() {
    let mut state = TuiState::new();
    let stream = activity(11);
    state
        .observe_record(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityStarted {
                activity: stream,
                kind: ActivityKind::AgentMessage,
            },
        ))
        .unwrap();
    for _ in 0..512 {
        state
            .observe_record(TranscriptRecord::EventCommitted(
                AgentEvent::ActivityUpdated {
                    activity: stream,
                    update: ActivityUpdate::TextDelta("x".to_owned()),
                },
            ))
            .unwrap();
    }

    assert_eq!(state.views().chat_context_count(), 1);
    let chat = render_and_commit(&mut state, Size::new(20, 5));
    assert!(!chat.contains("Chat ·"));
}

// 줄바꿈된 Chat item에서 한 줄 위로 이동하면 record 수가 아니라 실제 viewport 행으로
// 문맥을 계산하여 화면 상단의 긴 user item을 정확히 anchor한다.
#[test]
fn wrapped_chat_rows_map_to_the_visible_record_context() {
    let mut state = TuiState::new();
    state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("wrapped question ".repeat(8)),
            },
        ))
        .unwrap();
    state
        .observe_record(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityStarted {
                activity: activity(7),
                kind: ActivityKind::ApprovalRequest {
                    request_id: RequestId::new(NonZeroU64::new(14).unwrap()),
                },
            },
        ))
        .unwrap();
    let size = Size::new(18, 12);
    render_and_commit(&mut state, size);
    state
        .handle(key(KeyCode::PageUp, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let chat = render_and_commit(&mut state, size);
    assert!(!chat.contains("Chat ·"));

    state
        .handle(function(3, KeyAction::Press), Duration::ZERO)
        .unwrap();
    let request = render_and_commit(&mut state, Size::new(72, 16));
    assert!(request.contains("context_record=1"));
    assert!(request.contains("context_highlight=none(reason=no-direct-request)"));
    assert!(!request.contains("request=14"));
}

// Transcript page 이동은 wrapped record의 실제 행 범위를 사용하므로 양옆 request가
// 있어도 가운데 보이는 StartTurn만 선택하고 이웃 correlation을 빌리지 않는다.
#[test]
fn transcript_page_movement_uses_visible_rows_and_not_neighbor_records() {
    let mut state = TuiState::new();
    for record in [
        TranscriptRecord::CommandCommitted(AgentCommand::RespondToActivity {
            request: ActivityRequestRef::new(
                activity(8),
                RequestId::new(NonZeroU64::new(15).unwrap()),
            ),
            response: yo_core::ActivityResponse::Approval(ApprovalDecision::Approved),
        }),
        TranscriptRecord::CommandCommitted(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("middle wrapped record ".repeat(8)),
        }),
        TranscriptRecord::CommandCommitted(AgentCommand::RespondToActivity {
            request: ActivityRequestRef::new(
                activity(9),
                RequestId::new(NonZeroU64::new(16).unwrap()),
            ),
            response: yo_core::ActivityResponse::Approval(ApprovalDecision::Declined),
        }),
    ] {
        state.observe_record(record).unwrap();
    }
    state
        .handle(function(2, KeyAction::Press), Duration::ZERO)
        .unwrap();
    let size = Size::new(24, 7);
    render_and_commit(&mut state, size);
    state
        .handle(key(KeyCode::PageUp, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let transcript = render_and_commit(&mut state, size);
    assert!(transcript.contains("Transcript · F1/F2/F3"));

    state
        .handle(function(3, KeyAction::Press), Duration::ZERO)
        .unwrap();
    let request = render_and_commit(&mut state, Size::new(72, 10));
    assert!(request.contains("context_record=2"));
    assert!(request.contains("context_highlight=none(reason=no-direct-request)"));
    assert!(!request.contains("request=15"));
    assert!(!request.contains("request=16"));
}
