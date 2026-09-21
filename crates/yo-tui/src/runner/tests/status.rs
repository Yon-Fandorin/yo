use std::time::Duration;

use yo_core::{
    ActivityApproval, ActivityKind, ActivityUpdate, AgentEvent, ApprovalChoice, RequestId,
    SessionId,
};

use super::{activity, key, nonzero, turn};
use crate::{
    appearance::AppearanceState,
    input::event::{InputEvent, KeyCode, KeyModifiers},
    runner::{
        AgentAction, TuiSessionInfo,
        state::{StateEffect, TuiState},
    },
    surface::Size,
};

// /status는 현재 Session 식별자를 보여 주되 모델 Turn을 만들지 않고 명령 초안만 소비한다.
#[test]
fn status_shows_live_session_identity_without_dispatch() {
    let session_id: SessionId = "01890f00-0000-7000-8000-000000000001".parse().unwrap();
    let mut state = TuiState::with_session_info(
        TuiSessionInfo::new("host:codex · model unreported", "~/projects/yo")
            .with_session_id(session_id),
    );
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    state
        .handle(InputEvent::Paste("/status".to_owned()), Duration::ZERO)
        .unwrap();

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert!(state.editor().text().is_empty());
    let output = state
        .session_output(&AppearanceState::default().pin())
        .unwrap()
        .unwrap();
    assert!(output.contains(&session_id.to_string()), "{output}");
    assert!(output.contains("host:codex"), "{output}");
    assert!(output.contains("Running"), "{output}");
    assert!(
        output.contains("No completed usage observation"),
        "{output}"
    );
}

// 압축 요청이 진행 중일 때 /status는 Turn 유휴를 세션 유휴로 잘못 표시하지 않는다.
#[test]
fn status_reports_pending_context_compaction() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/compact".to_owned()), Duration::ZERO)
        .unwrap();
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::CompactContext { guidance: None })
    );
    state
        .handle(InputEvent::Paste("/status".to_owned()), Duration::ZERO)
        .unwrap();
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    let output = state
        .session_output(&AppearanceState::default().pin())
        .unwrap()
        .unwrap();
    assert!(output.contains("Compacting context"), "{output}");
}

// 승인 선택 화면의 /status는 승인 응답을 만들지 않고 현재 대기 요청을 유지한다.
#[test]
fn status_during_approval_does_not_answer_the_request() {
    let mut state = TuiState::new();
    let request_id = RequestId::new(nonzero(8));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ApprovalRequest { request_id },
        })
        .unwrap();
    let approval = ActivityApproval {
        related_change: None,
        plain_text: "Command: cargo test".to_owned(),
        choices: vec![ApprovalChoice {
            label: "Decline".to_owned(),
            description: "Continue without running".to_owned(),
            enabled: true,
        }],
        decline_choice: Some(1),
    };
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(approval.to_snapshot().unwrap()),
        })
        .unwrap();
    let pin = AppearanceState::default().pin();
    let frame = state.prepare_frame(Size::new(80, 30), &pin).unwrap();
    state.commit_frame(&frame);
    for character in "/status".chars() {
        state
            .handle(
                key(KeyCode::Character(character), KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap();
    }
    let frame = state.prepare_frame(Size::new(80, 30), &pin).unwrap();
    state.commit_frame(&frame);

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert!(state.has_pending_request());
    assert!(state.editor().text().is_empty());
    let output = state.session_output(&pin).unwrap().unwrap();
    assert!(output.contains("Session status"), "{output}");
    assert!(output.contains("Waiting for input"), "{output}");
    let frame = state.prepare_frame(Size::new(80, 30), &pin).unwrap();
    assert!(
        frame.overlay_presented,
        "approval choices must return after /status"
    );
}
