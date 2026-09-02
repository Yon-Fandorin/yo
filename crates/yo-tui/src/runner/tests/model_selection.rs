use std::time::Duration;

use yo_core::{
    ActivityKind, ActivityRequestRef, AgentEvent, DurabilityGapCause, JournalDurability, RequestId,
    TurnOutcome,
    session_repository::{DurableCutoff, RepositorySequence},
};

use super::{activity, key, nonzero, turn};
use crate::{
    appearance::AppearanceState,
    input::event::{InputEvent, KeyCode},
    runner::{
        AgentAction,
        state::{StateEffect, TuiState},
    },
    surface::Size,
};

fn model_controller(current_model: &str) -> yo_core::ModelSelectionController {
    model_controller_for("qwencloud", current_model)
}

fn model_controller_for(
    current_provider: &str,
    current_model: &str,
) -> yo_core::ModelSelectionController {
    model_controller_from_entries(
        [
            ("openrouter", "default", "free-model", "OpenRouter"),
            ("qwencloud", "default", "qwen3.8max", "Qwen Cloud"),
        ],
        current_provider,
        current_model,
    )
}

fn extended_model_controller() -> yo_core::ModelSelectionController {
    model_controller_from_entries(
        [
            ("openrouter", "default", "free-model", "OpenRouter"),
            ("qwencloud", "default", "qwen3.8max", "Qwen Cloud"),
            ("anthropic", "default", "claude-sonnet", "Anthropic"),
        ],
        "qwencloud",
        "qwen3.8max",
    )
}

fn model_controller_from_entries(
    entries: impl IntoIterator<Item = (&'static str, &'static str, &'static str, &'static str)>,
    current_provider: &str,
    current_model: &str,
) -> yo_core::ModelSelectionController {
    let entries = entries
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
    assert_eq!(state.editor().text(), "/model free-model");

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
    let selected = state.take_model_selection().unwrap();
    assert_eq!(
        selected.managed().unwrap().provider().as_str(),
        "openrouter"
    );
    assert_eq!(selected.managed().unwrap().account().as_str(), "default");
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
                key(KeyCode::Down, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Redraw
    );
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
    assert_eq!(
        selected.managed().unwrap().provider().as_str(),
        "openrouter"
    );
    assert_eq!(selected.managed().unwrap().account().as_str(), "default");
    assert_eq!(selected.model().as_str(), "free-model");
}

// command palette에서 /model을 선택해도 기존 model picker와 완전한 좌표 acceptance 경로를
// 그대로 사용하며, 접두어 draft는 picker가 열리기 전에 비운다.
#[test]
fn command_palette_model_selection_reuses_the_model_picker() {
    let mut state = TuiState::new();
    state.enable_model_selection(model_controller("qwen3.8max"));
    state
        .handle(InputEvent::Paste("/m".to_owned()), Duration::ZERO)
        .unwrap();
    let command_frame = state
        .prepare_frame(Size::new(80, 16), &AppearanceState::default().pin())
        .unwrap();
    assert!(command_frame.overlay_presented);
    state.commit_frame(&command_frame);

    assert_eq!(
        state
            .handle(
                key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Redraw
    );
    assert!(state.editor().text().is_empty());

    let model_frame = state
        .prepare_frame(Size::new(80, 16), &AppearanceState::default().pin())
        .unwrap();
    assert!(model_frame.overlay_presented);
    state.commit_frame(&model_frame);
    assert_eq!(
        state
            .handle(
                key(KeyCode::Down, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Redraw
    );
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
        state
            .take_model_selection()
            .unwrap()
            .managed()
            .unwrap()
            .provider()
            .as_str(),
        "openrouter"
    );
}

// active Turn 중 선택한 model은 현재 Turn을 교체하지 않고 다음 Turn 예약으로 남는다.
// exact steer는 기존 TurnRef를 유지하며, durable TurnFinished가 관찰된 뒤에만 host 교체가 준비된다.
#[test]
fn active_turn_model_selection_is_reserved_until_durable_completion() {
    let mut state = TuiState::new();
    state.enable_model_selection(model_controller("qwen3.8max"));
    state
        .observe_durability(JournalDurability::Durable {
            journal_sequence: None,
            repository_sequence: RepositorySequence::new(1),
        })
        .unwrap();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
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
    assert!(state.editor().text().is_empty());
    let output = state
        .session_output(&AppearanceState::default().pin())
        .unwrap()
        .unwrap();
    assert!(
        output.contains("will be applied to the next Turn"),
        "{output}"
    );

    state
        .handle(InputEvent::Paste("keep going".to_owned()), Duration::ZERO)
        .unwrap();
    let StateEffect::Dispatch(AgentAction::Steer {
        turn: steered,
        submission,
    }) = state
        .handle(
            key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
            Duration::ZERO,
        )
        .unwrap()
    else {
        panic!("current Turn input must remain an exact steer");
    };
    assert_eq!(steered, turn());
    assert_eq!(submission.input().as_str(), "keep going");
    assert_eq!(state.take_model_selection(), None);

    assert_eq!(
        state
            .observe(AgentEvent::TurnFinished {
                turn: turn(),
                outcome: TurnOutcome::Completed,
            })
            .unwrap(),
        StateEffect::Exit
    );
    assert!(state.model_switch_ready());
    assert_eq!(
        state
            .take_model_selection()
            .unwrap()
            .managed()
            .unwrap()
            .provider()
            .as_str(),
        "openrouter"
    );
}

// active Turn에서도 bare /model picker는 열리고, row acceptance가 즉시 host를 교체하지
// 않고 동일한 다음-Turn reservation 경로로 합류한다.
#[test]
fn active_turn_model_picker_acceptance_creates_a_reservation() {
    let mut state = TuiState::new();
    state.enable_model_selection(model_controller("qwen3.8max"));
    state
        .observe_durability(JournalDurability::Durable {
            journal_sequence: None,
            repository_sequence: RepositorySequence::new(1),
        })
        .unwrap();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
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
    assert!(frame.overlay_presented);
    state.commit_frame(&frame);
    assert_eq!(
        state
            .handle(
                key(KeyCode::Down, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Redraw
    );
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

    assert_eq!(
        state
            .observe(AgentEvent::TurnFinished {
                turn: turn(),
                outcome: TurnOutcome::Completed,
            })
            .unwrap(),
        StateEffect::Exit
    );
    assert_eq!(
        state
            .take_model_selection()
            .unwrap()
            .managed()
            .unwrap()
            .provider()
            .as_str(),
        "openrouter"
    );
}

// 예약 뒤 다른 model을 고르면 latest selection이 target을 교체하고, 현재 model을 다시
// 고르면 예약 자체가 취소되어 Turn 완료 뒤에도 host 교체가 일어나지 않는다.
#[test]
fn later_model_selection_replaces_or_cancels_the_reservation() {
    let mut state = TuiState::new();
    state.enable_model_selection(extended_model_controller());
    state
        .observe_durability(JournalDurability::Durable {
            journal_sequence: None,
            repository_sequence: RepositorySequence::new(1),
        })
        .unwrap();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();

    for command in [
        "/model openrouter::free-model",
        "/model anthropic::claude-sonnet",
    ] {
        state
            .handle(InputEvent::Paste(command.to_owned()), Duration::ZERO)
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
    }
    assert_eq!(
        state
            .observe(AgentEvent::TurnFinished {
                turn: turn(),
                outcome: TurnOutcome::Completed,
            })
            .unwrap(),
        StateEffect::Exit
    );
    assert_eq!(
        state
            .take_model_selection()
            .unwrap()
            .managed()
            .unwrap()
            .provider()
            .as_str(),
        "anthropic"
    );

    let mut state = TuiState::new();
    state.enable_model_selection(model_controller("qwen3.8max"));
    state
        .observe_durability(JournalDurability::Durable {
            journal_sequence: None,
            repository_sequence: RepositorySequence::new(2),
        })
        .unwrap();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    for command in ["/model openrouter::free-model", "/model qwen3.8max"] {
        state
            .handle(InputEvent::Paste(command.to_owned()), Duration::ZERO)
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
    }
    assert_eq!(
        state
            .observe(AgentEvent::TurnFinished {
                turn: turn(),
                outcome: TurnOutcome::Completed,
            })
            .unwrap(),
        StateEffect::Redraw
    );
    assert_eq!(state.take_model_selection(), None);
}

// memory-only 완료는 예약 적용의 durable 경계를 증명하지 못하므로 예약을 버리고 기존
// controller를 유지하며, 사용자에게 실패 이유를 보인다.
#[test]
fn nondurable_turn_completion_fails_a_model_reservation_visibly() {
    for durability in [
        JournalDurability::MemoryOnly,
        JournalDurability::Gap {
            durable_cutoff: DurableCutoff::KnownEmpty,
            cause: DurabilityGapCause::Capacity,
        },
    ] {
        let mut state = TuiState::new();
        state.enable_model_selection(model_controller("qwen3.8max"));
        state.observe_durability(durability).unwrap();
        state
            .observe(AgentEvent::TurnStarted { turn: turn() })
            .unwrap();
        state
            .handle(
                InputEvent::Paste("/model openrouter::free-model".to_owned()),
                Duration::ZERO,
            )
            .unwrap();
        state
            .handle(
                key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap();

        assert_eq!(
            state
                .observe(AgentEvent::TurnFinished {
                    turn: turn(),
                    outcome: TurnOutcome::Completed,
                })
                .unwrap(),
            StateEffect::Redraw
        );
        assert_eq!(state.take_model_selection(), None);
        let output = state
            .session_output(&AppearanceState::default().pin())
            .unwrap()
            .unwrap();
        assert!(
            output.contains("durable Turn completion could not be established"),
            "{output}"
        );
    }
}

// pending Activity 중 /model은 Activity 응답으로 소비되지 않고 다음 Turn 예약을 만든다.
// 이어지는 평문은 원래 request correlation으로 응답하며 예약은 그대로 남는다.
#[test]
fn pending_activity_keeps_model_selection_local_and_the_next_reply_correlated() {
    let mut state = TuiState::new();
    state.enable_model_selection(model_controller("qwen3.8max"));
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    let request_activity = activity(1);
    let request_id = RequestId::new(nonzero(11));
    state
        .observe(AgentEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::UserInputRequest { request_id },
        })
        .unwrap();
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
    assert!(state.has_pending_request());
    assert_eq!(state.take_model_selection(), None);

    state
        .handle(InputEvent::Paste("continue".to_owned()), Duration::ZERO)
        .unwrap();
    assert_eq!(
        state
            .handle(
                key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Dispatch(AgentAction::RespondToUserInput {
            request: ActivityRequestRef::new(request_activity, request_id),
            input: "continue".to_owned(),
        })
    );
    assert_eq!(state.take_model_selection(), None);
}
