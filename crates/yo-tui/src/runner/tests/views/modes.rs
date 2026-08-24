use std::{num::NonZeroU64, time::Duration};

use yo_core::{ActivityKind, AgentEvent, RequestId, TranscriptRecord};

use super::{
    super::{activity, key},
    support::{function, observed_conversation, render_and_commit},
};
use crate::{
    input::event::{InputEvent, KeyAction, KeyCode, KeyModifiers},
    runner::{
        state::{StateEffect, TuiState},
        view::{ObservabilityView, RequestUnavailableReason},
    },
    surface::Size,
};

// 같은 Journal 입력에서 Chat은 간결한 작업 표현을 유지하고 Transcript는 command/event와
// Activity 수명주기를 모두 보이며 Request는 마지막 정확한 문맥의 typed 부재를 표시한다.
#[test]
fn three_modes_render_distinct_visible_projections_from_one_journal() {
    let mut state = observed_conversation();
    let chat = render_and_commit(&mut state, Size::new(72, 12));
    assert!(!chat.contains("Chat ·"));
    assert!(chat.contains("❯ inspect the repository"));
    assert!(chat.contains("• Running tool…"));
    assert!(!chat.contains("event.activity_started"));

    assert_eq!(
        state.handle(function(2, KeyAction::Press), Duration::ZERO),
        Ok(StateEffect::Redraw)
    );
    let transcript = render_and_commit(&mut state, Size::new(80, 40));
    assert!(transcript.contains("Transcript · context 5/5 · F1 Chat · F2 Transcript · F3 Request"));
    assert!(transcript.contains("command.start_turn"));
    assert!(transcript.contains("event.activity_started"));
    assert!(transcript.contains("kind=tool_call"));
    assert!(transcript.contains("event.activity_finished"));
    assert!(transcript.contains("outcome=completed"));
    assert!(transcript.contains("[observation boundary]"));
    assert!(transcript.contains("JournalSequence"));
    assert!(transcript.contains("Request Audit detail"));

    assert_eq!(
        state.handle(function(3, KeyAction::Press), Duration::ZERO),
        Ok(StateEffect::Redraw)
    );
    let request = render_and_commit(&mut state, Size::new(80, 12));
    assert!(request.contains("Request · context 5/5 · F1 Chat · F2 Transcript · F3 Request"));
    assert!(request.contains("Live Session Request diagnostic"));
    assert!(request.contains("context_highlight=none(reason=no-direct-request)"));
    assert!(request.contains("no correlation records have been committed"));
}

// 기본 binding은 F1/F2/F3 press에서만 정확한 mode로 전환하고, 이미 활성인 mode의 같은
// 키·release와 더는 할당하지 않은 F4는 상태를 바꾸지 않는다.
#[test]
fn function_keys_switch_exactly_and_idempotently() {
    let mut state = TuiState::new();
    assert_eq!(state.views().active(), ObservabilityView::Chat);

    assert_eq!(
        state.handle(function(2, KeyAction::Press), Duration::ZERO),
        Ok(StateEffect::Redraw)
    );
    assert_eq!(state.views().active(), ObservabilityView::Transcript);
    assert_eq!(
        state.handle(function(2, KeyAction::Press), Duration::ZERO),
        Ok(StateEffect::Unchanged)
    );
    assert_eq!(
        state.handle(function(2, KeyAction::Release), Duration::ZERO),
        Ok(StateEffect::Unchanged)
    );
    assert_eq!(
        state.handle(function(3, KeyAction::Press), Duration::ZERO),
        Ok(StateEffect::Redraw)
    );
    assert_eq!(state.views().active(), ObservabilityView::Request);
    assert_eq!(
        state.handle(function(4, KeyAction::Press), Duration::ZERO),
        Ok(StateEffect::Unchanged)
    );
    assert_eq!(state.views().active(), ObservabilityView::Request);
    assert_eq!(
        state.handle(function(1, KeyAction::Press), Duration::ZERO),
        Ok(StateEffect::Redraw)
    );
    assert_eq!(state.views().active(), ObservabilityView::Chat);
}

// Transcript와 Request가 활성일 때 paste와 Enter는 editor를 건드리거나 Submit/응답 dispatch를
// 만들지 않고 소비되며 Chat으로 돌아오면 기존 draft가 그대로 편집 가능하다.
#[test]
fn non_chat_modes_enforce_read_only_input_without_dispatch() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("draft".to_owned()), Duration::ZERO)
        .unwrap();

    for mode in [2, 3] {
        state
            .handle(function(mode, KeyAction::Press), Duration::ZERO)
            .unwrap();
        assert_eq!(
            state.handle(InputEvent::Paste(" ignored".to_owned()), Duration::ZERO),
            Ok(StateEffect::Unchanged)
        );
        assert_eq!(
            state.handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO),
            Ok(StateEffect::Unchanged)
        );
        assert_eq!(state.editor().text(), "draft");
    }

    state
        .handle(function(1, KeyAction::Press), Duration::ZERO)
        .unwrap();
    let StateEffect::Dispatch(crate::runner::AgentAction::Submit(submission)) = state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
    else {
        panic!("Chat Enter should queue one immutable submission");
    };
    assert_eq!(submission.input().as_str(), "draft");
    assert_eq!(state.editor().text(), "draft");
}

// 현재 문맥의 직접 Activity request 유무는 highlight로만 구분하며, 두 경우 모두 전체
// Session trace와 별개의 Audit reader 부재를 정직하게 표시합니다.
#[test]
fn request_distinguishes_no_association_from_unavailable_audit_detail() {
    let mut no_request = observed_conversation();
    no_request
        .handle(function(3, KeyAction::Press), Duration::ZERO)
        .unwrap();
    assert_eq!(
        no_request.views().request_reason(),
        RequestUnavailableReason::NoAssociatedRequest
    );

    let mut unavailable = TuiState::new();
    let request_id = RequestId::new(NonZeroU64::new(7).unwrap());
    unavailable
        .observe_record(TranscriptRecord::EventCommitted(
            AgentEvent::ActivityStarted {
                activity: activity(3),
                kind: ActivityKind::ApprovalRequest { request_id },
            },
        ))
        .unwrap();
    render_and_commit(&mut unavailable, Size::new(72, 12));
    unavailable
        .handle(function(3, KeyAction::Press), Duration::ZERO)
        .unwrap();

    assert_eq!(
        unavailable.views().request_reason(),
        RequestUnavailableReason::RequestAuditDetailUnavailable
    );
    let request = render_and_commit(&mut unavailable, Size::new(72, 12));
    assert!(request.contains("request_audit_detail=unavailable(reason=no-audit-reader)"));
    assert!(request.contains("context_highlight=direct-activity-request"));
    assert!(request.contains("activity=3 request=7"));
    assert!(request.contains("no correlation records have been committed"));
}
