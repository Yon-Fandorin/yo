use std::{collections::BTreeMap, time::Duration};

use yo_core::{AgentEvent, SubmissionOutcome, SubmissionRejection, SubmissionRejectionKind};

use super::{key, turn};
use crate::{
    PromptTemplates,
    input::event::{InputEvent, KeyCode, KeyModifiers},
    runner::{
        AgentAction,
        state::{StateEffect, TuiState},
    },
};

fn configured(body: &str) -> TuiState {
    let mut state = TuiState::new();
    state.set_prompt_templates(
        PromptTemplates::new(BTreeMap::from([("review".to_owned(), body.to_owned())])).unwrap(),
    );
    state
}

fn insert(state: &mut TuiState, command: &str) -> StateEffect {
    state
        .handle(InputEvent::Paste(command.to_owned()), Duration::ZERO)
        .unwrap();
    state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
}

// 로딩은 backend 동작 없이 원문을 편집기에 넣고, 별도 Enter만 참조 없는 입력을 제출합니다.
#[test]
fn prompt_insertion_preserves_literal_text_until_explicit_submission() {
    for body in [
        "  한글 👩‍💻\n\t$skill @path $(command) {{value}}\r\n",
        "/exit",
    ] {
        let mut state = configured(body);
        assert_eq!(insert(&mut state, "/prompt review"), StateEffect::Redraw);
        assert_eq!(state.editor().text(), body);
        let effect = state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap();
        let StateEffect::Dispatch(AgentAction::Submit(submission)) = effect else {
            panic!("explicit Enter must submit the literal prompt, got {effect:?}");
        };
        assert_eq!(submission.input().as_str(), body);
        assert!(submission.input().references().is_empty());
    }
}

// 활성 Turn 중에도 로딩은 실행하지 않으며 다음 Enter는 기존 Turn을 대상으로 steer합니다.
#[test]
fn inserted_prompt_preserves_active_turn_routing() {
    let mut state = configured("check the pending change");
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    assert_eq!(insert(&mut state, "/prompt review"), StateEffect::Redraw);
    let effect = state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let StateEffect::Dispatch(AgentAction::Steer {
        turn: target,
        submission,
    }) = effect
    else {
        panic!("active turn input must remain steer, got {effect:?}");
    };
    assert_eq!(target, turn());
    assert_eq!(submission.input().as_str(), "check the pending change");
}

// admission 대기 중 중복 Enter와 거절 후 재시도도 slash 본문을 로컬 명령으로 실행하지 않습니다.
#[test]
fn literal_prompt_survives_pending_admission_and_rejection_retry() {
    let mut state = configured("/exit");
    assert_eq!(insert(&mut state, "/prompt review"), StateEffect::Redraw);
    let StateEffect::Dispatch(AgentAction::Submit(first)) = state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
    else {
        panic!("first Enter must submit literal input");
    };
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Redraw
    );
    state
        .observe_submission_outcome(SubmissionOutcome::Rejected {
            id: first.id(),
            rejection: SubmissionRejection::new(SubmissionRejectionKind::TargetChanged, "retry"),
        })
        .unwrap();
    let StateEffect::Dispatch(AgentAction::Submit(retry)) = state
        .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
        .unwrap()
    else {
        panic!("retry must still submit literal input");
    };
    assert_eq!(retry.input().as_str(), "/exit");
    assert_ne!(retry.id(), first.id());
    state
        .observe_submission_outcome(SubmissionOutcome::Accepted { id: retry.id() })
        .unwrap();
    assert!(state.editor().text().is_empty());
    // 수락된 draft가 지워진 뒤 새로 입력한 /exit는 다시 사용자 로컬 명령입니다.
    state
        .handle(InputEvent::Paste("/exit".to_owned()), Duration::ZERO)
        .unwrap();
    assert_eq!(
        state
            .handle(key(KeyCode::Enter, KeyModifiers::NONE), Duration::ZERO)
            .unwrap(),
        StateEffect::Exit
    );
}

// 없는 이름은 draft를 보존하고 목록 명령은 최대 설정에서도 모델 호출 없이 표시합니다.
#[test]
fn missing_prompt_preserves_draft_and_listing_remains_bounded() {
    let mut state = configured("review");
    assert_eq!(insert(&mut state, "/prompt absent"), StateEffect::Redraw);
    assert_eq!(state.editor().text(), "/prompt absent");

    let mut state = TuiState::new();
    state.set_prompt_templates(
        PromptTemplates::new(
            (0..128)
                .map(|index| (format!("p{index:063}"), "body".to_owned()))
                .collect(),
        )
        .unwrap(),
    );
    assert_eq!(insert(&mut state, "/prompt "), StateEffect::Redraw);
    assert!(state.editor().text().is_empty());
    assert!(!state.transcript().items().is_empty());
}
