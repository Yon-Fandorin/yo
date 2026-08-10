use std::time::Duration;

use super::key;
use crate::{
    appearance::AppearanceState,
    input::event::{InputEvent, KeyCode},
    runner::state::{StateEffect, TuiState},
    surface::Size,
};

fn model_controller(current_model: &str) -> yo_core::ModelSelectionController {
    model_controller_for("qwencloud", current_model)
}

fn model_controller_for(
    current_provider: &str,
    current_model: &str,
) -> yo_core::ModelSelectionController {
    let entries = [
        ("openrouter", "default", "free-model", "OpenRouter"),
        ("qwencloud", "default", "qwen3.8max", "Qwen Cloud"),
    ]
    .into_iter()
    .map(|(provider, account, model, provider_label)| {
        yo_core::ModelCatalogEntry::new(
            yo_core::EffectiveModelBinding::new(
                yo_core::ProviderId::new(provider).unwrap(),
                yo_core::AccountId::new(account).unwrap(),
                yo_core::ModelId::new(model).unwrap(),
                yo_core::ApiDialect::OpenAiResponses,
                yo_core::NormalizedEndpoint::parse("https://example.test/v1").unwrap(),
            ),
            Some(provider_label.to_owned()),
            Some("Default".to_owned()),
            None,
            yo_core::ModelContextProfile::new(1_000, 100, "utf8-bytes/v1").unwrap(),
        )
        .unwrap()
    })
    .collect();
    let current = yo_core::ModelSelection::new(
        yo_core::ProviderId::new(current_provider).unwrap(),
        yo_core::AccountId::new("default").unwrap(),
        yo_core::ModelId::new(current_model).unwrap(),
    );
    yo_core::ModelSelectionController::new(
        yo_core::ModelCatalog::new(entries).unwrap(),
        Some(current),
    )
}

// 직접 `/model Model`은 같은 ID가 다른 Provider에 있어도 현재 namespace 밖을 탐색하지
// 않고, qualified reference만 다른 완전한 좌표를 선택하는지 검증한다.
#[test]
fn direct_model_command_resolves_only_inside_the_current_provider_and_account() {
    let mut state = TuiState::new();
    state.enable_model_selection(model_controller("qwen3.8max"));
    state
        .handle(
            InputEvent::Paste("/model free-model".to_owned()),
            Duration::ZERO,
        )
        .unwrap();

    assert_eq!(
        state
            .handle(
                key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.take_model_selection(), None);

    state
        .handle(
            InputEvent::Paste("/model openrouter::free-model".to_owned()),
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(
        state
            .handle(
                key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Exit
    );
    let selected = state.take_model_selection().unwrap();
    assert_eq!(selected.provider().as_str(), "openrouter");
    assert_eq!(selected.account().as_str(), "default");
    assert_eq!(selected.model().as_str(), "free-model");
}

// picker acceptance는 display label이 아니라 Provider·Account·Model 전체 좌표를 반환한다.
#[test]
fn grouped_model_picker_returns_the_complete_selected_binding() {
    let mut state = TuiState::new();
    state.enable_model_selection(model_controller("qwen3.8max"));
    state
        .handle(InputEvent::Paste("/model".to_owned()), Duration::ZERO)
        .unwrap();
    assert_eq!(
        state
            .handle(
                key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Redraw
    );
    let frame = state
        .prepare_frame(Size::new(80, 16), &AppearanceState::default().pin())
        .unwrap();
    state.commit_frame(&frame);
    assert_eq!(
        state
            .handle(
                key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Exit
    );
    let selected = state.take_model_selection().unwrap();
    assert_eq!(selected.provider().as_str(), "openrouter");
    assert_eq!(selected.account().as_str(), "default");
    assert_eq!(selected.model().as_str(), "free-model");
}

// picker에서 이미 현재인 model row를 다시 고르면 새 switch 요청을 만들지 않고, 현재 상태를
// 유지한 채 redraw만 반환한다.
#[test]
fn model_picker_reselecting_the_current_model_does_not_request_a_switch() {
    let mut state = TuiState::new();
    state.enable_model_selection(model_controller_for("openrouter", "free-model"));
    state
        .handle(InputEvent::Paste("/model".to_owned()), Duration::ZERO)
        .unwrap();
    assert_eq!(
        state
            .handle(
                key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Redraw
    );
    let frame = state
        .prepare_frame(Size::new(80, 16), &AppearanceState::default().pin())
        .unwrap();
    state.commit_frame(&frame);

    assert_eq!(
        state
            .handle(
                key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.take_model_selection(), None);
}

// switch가 commit되면 새 controller가 현재 model을 소유하므로 같은 direct reference와 picker
// row의 재선택은 교체 요청을 부활시키지 않는다.
#[test]
fn committed_model_replacement_becomes_current_for_direct_and_picker_paths() {
    let mut state = TuiState::new();
    state.enable_model_selection(model_controller("qwen3.8max"));
    state
        .handle(
            InputEvent::Paste("/model openrouter::free-model".to_owned()),
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(
        state
            .handle(
                key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Exit
    );
    assert_eq!(
        state.take_model_selection().unwrap().model().as_str(),
        "free-model"
    );

    state.commit_model_switch(
        model_controller_for("openrouter", "free-model"),
        "OpenRouter".to_owned(),
        None,
    );

    state
        .handle(
            InputEvent::Paste("/model openrouter::free-model".to_owned()),
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(
        state
            .handle(
                key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.take_model_selection(), None);

    state
        .handle(InputEvent::Paste("/model".to_owned()), Duration::ZERO)
        .unwrap();
    assert_eq!(
        state
            .handle(
                key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Redraw
    );
    let frame = state
        .prepare_frame(Size::new(80, 16), &AppearanceState::default().pin())
        .unwrap();
    state.commit_frame(&frame);
    assert_eq!(
        state
            .handle(
                key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.take_model_selection(), None);
}

// backend 교체가 실패해도 TuiState의 기존 controller는 유지되어, 이전 model 재선택은 no-op이고
// 다른 configured model은 다시 동일한 switch 요청으로 나간다.
#[test]
fn failed_model_replacement_preserves_the_previous_controller() {
    let mut state = TuiState::new();
    state.enable_model_selection(model_controller("qwen3.8max"));
    state.report_model_switch_failure("backend unavailable".to_owned());

    state
        .handle(
            InputEvent::Paste("/model qwen3.8max".to_owned()),
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(
        state
            .handle(
                key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.take_model_selection(), None);

    state
        .handle(
            InputEvent::Paste("/model openrouter::free-model".to_owned()),
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(
        state
            .handle(
                key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Exit
    );
    assert_eq!(
        state.take_model_selection().unwrap().provider().as_str(),
        "openrouter"
    );
}
