use yo_core::{ActivityKind, ActivityOutcome, ActivityUpdate, AgentEvent, TurnOutcome};

use super::{activity, rendered_row, turn};
use crate::{
    runner::state::{StateEffect, TuiState},
    surface::Size,
};

// agent message의 streaming delta를 먼저 표시하더라도 final snapshot이 다르면 화면 문자열을
// authoritative 결과로 교체하고 완료 뒤 그대로 남긴다.
#[test]
fn renders_the_authoritative_agent_message_snapshot() {
    let mut state = TuiState::new();
    let message = activity(1);
    state
        .observe(AgentEvent::ActivityStarted {
            activity: message,
            kind: ActivityKind::AgentMessage,
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: message,
            update: ActivityUpdate::TextDelta("partial".to_owned()),
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: message,
            update: ActivityUpdate::TextSnapshot("complete answer".to_owned()),
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityFinished {
            activity: message,
            outcome: ActivityOutcome::Completed,
        })
        .unwrap();

    assert_eq!(
        rendered_row(&state, Size::new(24, 3), 0),
        "• complete answer"
    );
}

// non-message Activity의 빈 delta는 label 뒤에 보이지 않는 줄 바꿈을 누적하지 않고
// transcript와 화면 revision을 그대로 유지한다.
#[test]
fn empty_activity_delta_does_not_add_placeholder_lines() {
    let mut state = TuiState::new();
    let tool = activity(1);
    state
        .observe(AgentEvent::ActivityStarted {
            activity: tool,
            kind: ActivityKind::ToolCall,
        })
        .unwrap();
    let before = state.transcript().clone();

    assert_eq!(
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: tool,
                update: ActivityUpdate::TextDelta(String::new()),
            })
            .unwrap(),
        StateEffect::Unchanged
    );

    assert_eq!(state.transcript(), &before);
}

// tool과 file-change Activity는 agent message가 없어도 서로 다른 완료 관찰로 transcript에
// 계속 남아 코딩 작업이 chat text만으로 축소되지 않는다.
#[test]
fn retains_completed_tool_and_file_change_observations() {
    let mut state = TuiState::new();
    let tool = activity(1);
    let file = activity(2);
    for (activity, kind) in [
        (tool, ActivityKind::ToolCall),
        (file, ActivityKind::FileChange),
    ] {
        state
            .observe(AgentEvent::ActivityStarted { activity, kind })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            })
            .unwrap();
    }

    assert_eq!(
        rendered_row(&state, Size::new(30, 12), 0),
        "• Running tool…"
    );
    assert_eq!(
        rendered_row(&state, Size::new(30, 12), 2),
        "• File change observed"
    );
}

// Turn의 실패 event는 활성 상태를 닫고 사용자에게 backend 오류 내용을 별도 transcript
// 항목으로 남긴다.
#[test]
fn renders_turn_failure_and_clears_active_state() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();

    state
        .observe(AgentEvent::TurnFinished {
            turn: turn(),
            outcome: TurnOutcome::Failed(yo_core::Failure::new("provider stopped")),
        })
        .unwrap();

    assert!(!state.turn_active());
    assert_eq!(
        rendered_row(&state, Size::new(36, 3), 0),
        "• Turn failed: provider stopped"
    );
}
