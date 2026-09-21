use std::num::NonZeroU64;

use yo_core::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef, ActivityUpdate,
    AgentEvent, RequestId, SessionId, TranscriptRecord, TurnId, TurnOutcome, TurnRef,
};

use super::{CommandEffect, CopyAnswer, MAX_TEXT_BYTES, PendingRequest, StateEffect, TuiState};

fn completed_answer(state: &mut TuiState, number: u64, text: String) {
    let session: SessionId = "01890f00-0000-7000-8000-000000000001".parse().unwrap();
    let turn = TurnRef::new(session, TurnId::new(NonZeroU64::new(number).unwrap()));
    let activity = ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap()));
    for event in [
        AgentEvent::ActivityStarted {
            activity,
            kind: ActivityKind::AgentMessage,
        },
        AgentEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextSnapshot(text),
        },
        AgentEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed,
        },
        AgentEvent::TurnFinished {
            turn,
            outcome: TurnOutcome::Completed,
        },
    ] {
        state
            .chat
            .observe_record(&TranscriptRecord::EventCommitted(event))
            .unwrap();
    }
}

// 완료 답변이 없으면 출력 요청이 없고, 완료 답변은 원문만 전송하며 대기 요청은 유지한다.
#[test]
fn copy_command_selects_only_completed_answer_without_dispatching() {
    let mut state = TuiState::new();
    assert_eq!(
        state
            .execute_command(CommandEffect::CopyAnswer, "/copy", "/copy")
            .unwrap(),
        StateEffect::Redraw
    );
    completed_answer(
        &mut state,
        1,
        "한글 🦀\n```rust\nfn main() {}\n```".to_owned(),
    );
    let session: SessionId = "01890f00-0000-7000-8000-000000000001".parse().unwrap();
    let turn = TurnRef::new(session, TurnId::new(NonZeroU64::new(2).unwrap()));
    let request = ActivityRequestRef::new(
        ActivityRef::new(turn, ActivityId::new(NonZeroU64::new(1).unwrap())),
        RequestId::new(NonZeroU64::new(1).unwrap()),
    );
    state
        .pending_requests
        .push_back(PendingRequest::Approval(request));

    assert_eq!(
        state
            .execute_command(CommandEffect::CopyAnswer, "/copy", "/copy")
            .unwrap(),
        StateEffect::CopyToClipboard("한글 🦀\n```rust\nfn main() {}\n```".to_owned())
    );
    assert_eq!(
        state.pending_requests.front(),
        Some(&PendingRequest::Approval(request))
    );
}

// 너무 긴 답변은 출력 효과를 만들지 않고 사용자가 한도를 볼 수 있게 알린다.
#[test]
fn copy_command_rejects_first_excess_source_byte() {
    let mut state = TuiState::new();
    completed_answer(&mut state, 1, "x".repeat(MAX_TEXT_BYTES + 1));
    assert_eq!(
        state
            .execute_command(CommandEffect::CopyAnswer, "/copy", "/copy")
            .unwrap(),
        StateEffect::Redraw
    );
    assert!(matches!(
        state.chat.last_completed_answer(),
        Some(CopyAnswer::Text(text)) if text.len() > MAX_TEXT_BYTES
    ));
}

// 마지막 답변이 이미지 블록뿐이면 내부 message-content JSON을 클립보드로 내보내지 않는다.
#[test]
fn copy_command_rejects_non_text_answer() {
    let mut state = TuiState::new();
    let image = yo_core::MessageContent {
        block: serde_json::json!({"type":"image","mimeType":"image/png","data":"aGVsbG8="}),
    }
    .to_snapshot()
    .unwrap();
    completed_answer(&mut state, 1, image);
    assert_eq!(
        state.chat.last_completed_answer(),
        Some(CopyAnswer::NonText)
    );
    assert_eq!(
        state
            .execute_command(CommandEffect::CopyAnswer, "/copy", "/copy")
            .unwrap(),
        StateEffect::Redraw
    );
}
