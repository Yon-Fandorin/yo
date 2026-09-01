use std::time::Duration;

use yo_core::{AgentEvent, SubmissionOutcome, SubmissionRejection, SubmissionRejectionKind};

use super::{key, rendered_row, turn};
use crate::{
    appearance::AppearanceState,
    input::event::{InputEvent, KeyCode, KeyModifiers},
    runner::{
        AgentAction,
        state::{StateEffect, TuiState},
    },
    surface::Size,
};

fn present_palette(state: &mut TuiState, size: Size) -> String {
    let frame = state
        .prepare_frame(size, &AppearanceState::default().pin())
        .unwrap();
    assert!(frame.overlay_presented);
    state.commit_frame(&frame);
    (0..size.height)
        .map(|row| rendered_row(state, size, row))
        .collect::<Vec<_>>()
        .join("\n")
}

// 첫 slash 입력은 agent 제출 없이 prompt 위에 현재 로컬 명령 전체를 표시한다.
#[test]
fn slash_opens_the_local_command_palette() {
    let mut state = TuiState::new();
    assert_eq!(
        state
            .handle(InputEvent::Paste("/".to_owned()), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );

    let rendered = present_palette(&mut state, Size::new(80, 16));
    assert!(rendered.contains("Commands"), "{rendered}");
    assert!(rendered.contains("/help"), "{rendered}");
    assert!(rendered.contains("/model"), "{rendered}");
    assert!(rendered.contains("/compact"), "{rendered}");
    assert!(rendered.contains("/exit"), "{rendered}");
    assert_eq!(state.editor().text(), "/");
    assert!(state.transcript().items().is_empty());
}

// 선택 지침이 있는 `/compact`가 idle control intent 하나로 정확히 변환됨을 검증합니다.
#[test]
fn compact_with_guidance_dispatches_one_idle_control_intent() {
    let mut state = TuiState::new();
    state
        .handle(
            InputEvent::Paste("/compact preserve unresolved constraints".to_owned()),
            Duration::ZERO,
        )
        .unwrap();

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::CompactContext {
            guidance: Some("preserve unresolved constraints".to_owned()),
        })
    );
    assert!(state.editor().text().is_empty());
}

// slash 접두어는 일치하는 명령만 남기고 editor draft와 cursor 소유권을 유지한다.
#[test]
fn slash_query_filters_the_palette() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/m".to_owned()), Duration::ZERO)
        .unwrap();

    let rendered = present_palette(&mut state, Size::new(80, 16));
    assert!(rendered.contains("/model"), "{rendered}");
    assert!(!rendered.contains("/help"), "{rendered}");
    assert!(!rendered.contains("/exit"), "{rendered}");
    assert_eq!(state.editor().text(), "/m");
}

// 이미 보인 palette를 query로 좁힌 직후 redraw 전 Enter가 와도 새 선택을 실행하거나
// 부분 command를 agent에 제출하지 않고, 그 snapshot이 표시된 뒤에만 acceptance한다.
#[test]
fn query_refinement_remains_acceptable_before_the_next_frame() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/".to_owned()), Duration::ZERO)
        .unwrap();
    present_palette(&mut state, Size::new(80, 16));

    assert_eq!(
        state
            .handle(InputEvent::Paste("e".to_owned()), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Unchanged
    );
    assert_eq!(state.editor().text(), "/e");
    assert!(state.transcript().items().is_empty());

    let rendered = present_palette(&mut state, Size::new(80, 16));
    assert!(rendered.contains("/exit"), "{rendered}");
    assert!(!rendered.contains("/help"), "{rendered}");
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Exit
    );
}

// cursor 뒤에 다른 draft가 남아 있으면 앞쪽 slash 접두어는 command 입력 소유권을
// 얻지 않아 acceptance가 suffix를 지울 수 없다.
#[test]
fn cursor_prefix_does_not_open_over_a_trailing_draft() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/h keep".to_owned()), Duration::ZERO)
        .unwrap();
    for _ in 0..5 {
        state
            .handle(key(KeyCode::Left, KeyModifiers::NONE), Duration::ZERO)
            .unwrap();
    }

    let frame = state
        .prepare_frame(Size::new(80, 16), &AppearanceState::default().pin())
        .unwrap();
    assert!(!frame.overlay_presented);
    assert_eq!(state.editor().text(), "/h keep");
    assert!(state.transcript().items().is_empty());
}

// cursor 뒤 text 때문에 palette가 소유하지 않는 known/unknown slash draft는 Enter 시점에도
// 다시 command로 분류되지 않고 현재 ordinary prompt owner에 그대로 제출된다.
#[test]
fn cursor_ineligible_slash_drafts_remain_ordinary_submissions() {
    for draft in ["/help", "/foo"] {
        let mut state = TuiState::new();
        state
            .handle(InputEvent::Paste(draft.to_owned()), Duration::ZERO)
            .unwrap();
        state
            .handle(key(KeyCode::Left, KeyModifiers::NONE), Duration::ZERO)
            .unwrap();

        let frame = state
            .prepare_frame(Size::new(80, 16), &AppearanceState::default().pin())
            .unwrap();
        assert!(!frame.overlay_presented);

        let StateEffect::Dispatch(AgentAction::Submit(submission)) = state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap()
        else {
            panic!("cursor-ineligible slash draft must use ordinary submission");
        };
        assert_eq!(submission.input().as_str(), draft);
        assert!(state.transcript().items().is_empty());
    }
}

// Esc는 현재 draft를 지우거나 agent 요청을 만들지 않고 command overlay만 닫는다.
#[test]
fn escape_closes_the_palette_without_submitting() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/".to_owned()), Duration::ZERO)
        .unwrap();
    present_palette(&mut state, Size::new(80, 16));

    assert_eq!(
        state
            .handle(key(KeyCode::Escape, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.editor().text(), "/");
    assert!(state.transcript().items().is_empty());
    assert!(
        !state
            .prepare_frame(Size::new(80, 16), &AppearanceState::default().pin())
            .unwrap()
            .overlay_presented
    );
}

// 선택한 /help는 agent로 제출하지 않고 로컬 notice를 남긴 뒤 prompt를 비운다.
#[test]
fn selected_help_runs_locally() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/h".to_owned()), Duration::ZERO)
        .unwrap();
    present_palette(&mut state, Size::new(80, 16));

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert!(state.editor().text().is_empty());
    assert_eq!(state.transcript().items().len(), 1);
    let output = state
        .session_output(&AppearanceState::default().pin())
        .unwrap()
        .unwrap();
    assert!(output.contains("Available commands"), "{output}");
    assert!(output.contains("/model"), "{output}");
}

// exact command는 palette frame을 기다리지 않아도 registry에서 로컬 실행된다.
#[test]
fn exact_help_runs_locally_before_palette_presentation() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/help".to_owned()), Duration::ZERO)
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
    assert!(output.contains("Available commands"), "{output}");
}

// 앞쪽 공백과 ASCII 대소문자가 있는 exact command도 query scanner가 인정한 같은
// invocation이므로, 첫 palette frame 전후에 관계없이 동일한 로컬 효과를 실행한다.
#[test]
fn normalized_exact_help_is_frame_timing_independent() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("  /HELP".to_owned()), Duration::ZERO)
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
    assert!(output.contains("Available commands"), "{output}");
}

// 아직 표시되지 않은 exact /model이 로컬 admission에 실패해도 제출 과정에서 비워진
// editor 대신 원래 command draft를 복원해 사용자가 그대로 수정할 수 있게 한다.
#[test]
fn unpresented_model_admission_failure_restores_the_exact_draft() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/model".to_owned()), Duration::ZERO)
        .unwrap();

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.editor().text(), "/model");
    let output = state
        .session_output(&AppearanceState::default().pin())
        .unwrap()
        .unwrap();
    assert!(output.contains("No configured model catalog"), "{output}");
}

// 목록 이동 뒤 선택한 /exit는 transcript를 추가하지 않고 기존 runner 종료 효과를 반환한다.
#[test]
fn selected_exit_uses_the_existing_runner_exit_boundary() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/".to_owned()), Duration::ZERO)
        .unwrap();
    present_palette(&mut state, Size::new(80, 16));

    for _ in 0..3 {
        assert_eq!(
            state
                .handle(key(KeyCode::Down, KeyModifiers::NONE), Duration::ZERO)
                .unwrap(),
            StateEffect::Redraw
        );
    }
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Exit
    );
    assert!(state.transcript().items().is_empty());
}

// 아직 한 번도 표시되지 않은 부분 command의 Enter는 agent 제출로 빠지지 않고 로컬
// unknown 결과를 남기며, 사용자가 수정할 수 있도록 원문 draft를 보존한다.
#[test]
fn unpresented_partial_command_is_a_local_unknown() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/e".to_owned()), Duration::ZERO)
        .unwrap();

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.editor().text(), "/e");
    let output = state
        .session_output(&AppearanceState::default().pin())
        .unwrap()
        .unwrap();
    assert!(output.contains("Unknown command `/e`"), "{output}");
}

// 표시된 palette에 일치 항목이 없는 상태에서 Enter를 누르면 panel을 닫고 로컬 unknown을
// 알리되, draft를 agent나 Activity로 보내지 않는다.
#[test]
fn visible_unknown_command_closes_locally_and_preserves_the_draft() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/foo".to_owned()), Duration::ZERO)
        .unwrap();
    present_palette(&mut state, Size::new(80, 16));

    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.editor().text(), "/foo");
    assert!(
        !state
            .prepare_frame(Size::new(80, 16), &AppearanceState::default().pin())
            .unwrap()
            .overlay_presented
    );
    let output = state
        .session_output(&AppearanceState::default().pin())
        .unwrap()
        .unwrap();
    assert!(output.contains("Unknown command `/foo`"), "{output}");
}

// 실제로 보인 palette를 Esc로 닫으면 정확히 그 unchanged draft의 다음 Enter 한 번만
// ordinary agent 제출로 통과한다. 그 admission이 끝난 뒤 같은 draft는 다시 local unknown이다.
#[test]
fn escape_arms_one_exact_agent_submission() {
    let mut state = TuiState::new();
    state
        .handle(InputEvent::Paste("/foo".to_owned()), Duration::ZERO)
        .unwrap();
    present_palette(&mut state, Size::new(80, 16));
    assert_eq!(
        state
            .handle(key(KeyCode::Escape, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );

    let StateEffect::Dispatch(AgentAction::Submit(submission)) = state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
    else {
        panic!("dismissed command draft must pass through once");
    };
    assert_eq!(submission.input().as_str(), "/foo");
    state
        .observe_submission_outcome(SubmissionOutcome::Rejected {
            id: submission.id(),
            rejection: SubmissionRejection::new(
                SubmissionRejectionKind::StaleReference,
                "fixture rejection",
            ),
        })
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
    assert!(output.contains("Unknown command `/foo`"), "{output}");
}

// open됐지만 아직 frame에 표시되지 않은 command palette는 Esc를 소유하지 않는다.
// active Turn의 기존 interrupt 규칙이 그대로 우선한다.
#[test]
fn unpresented_palette_does_not_claim_active_turn_escape() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    state
        .handle(InputEvent::Paste("/foo".to_owned()), Duration::ZERO)
        .unwrap();

    assert_eq!(
        state
            .handle(key(KeyCode::Escape, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Dispatch(AgentAction::Interrupt)
    );
}
