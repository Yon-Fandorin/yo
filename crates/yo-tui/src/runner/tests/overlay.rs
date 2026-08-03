use super::*;
use crate::overlay::{PanelSnapshot, SelectionEntry, SlotError};

fn overlay_snapshot() -> PanelSnapshot {
    PanelSnapshot::new(
        "Commands",
        vec![
            SelectionEntry::enabled("one", "First", None),
            SelectionEntry::enabled("two", "Second", None),
        ],
    )
    .unwrap()
}

fn present_overlay(state: &mut TuiState, size: Size) {
    state.open_overlay(overlay_snapshot()).unwrap();
    let frame = state
        .prepare_frame(size, &AppearanceState::default().pin())
        .unwrap();
    state.commit_frame(&frame);
}

// visible overlay가 있는 active Turn에서 Esc는 overlay만 닫고 interrupt를 만들지 않으며,
// 이어지는 Ctrl+C는 여전히 agent interrupt intent로 전달된다.
#[test]
fn visible_overlay_dismissal_precedes_active_turn_escape_interrupt() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    present_overlay(&mut state, Size::new(40, 12));

    assert_eq!(
        state
            .handle(
                key(KeyCode::Escape, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Redraw
    );
    let frame = state
        .prepare_frame(Size::new(40, 12), &AppearanceState::default().pin())
        .unwrap();
    assert!(!frame.overlay_presented);
    assert_eq!(
        state
            .handle(
                key(
                    KeyCode::Character('c'),
                    crate::input::event::KeyModifiers::CONTROL,
                ),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Dispatch(AgentAction::Interrupt)
    );
}

// provider가 첫 receipt를 회수하기 전에 두 번째 panel도 accept하면 두 identity가 accept
// 순서대로 남고, 모두 회수한 뒤에는 중복 effect가 없다.
#[test]
fn accepted_overlay_receipts_wait_for_their_provider_in_fifo_order() {
    let mut state = TuiState::new();
    present_overlay(&mut state, Size::new(40, 12));

    assert_eq!(
        state
            .handle(
                key(KeyCode::Enter, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Redraw
    );
    state
        .open_overlay(
            PanelSnapshot::new(
                "Commands",
                vec![SelectionEntry::enabled("two", "Second", None)],
            )
            .unwrap(),
        )
        .unwrap();
    let frame = state
        .prepare_frame(Size::new(40, 12), &AppearanceState::default().pin())
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

    assert_eq!(state.take_overlay_acceptance().unwrap().identity(), "one");
    assert_eq!(state.take_overlay_acceptance().unwrap().identity(), "two");
    assert_eq!(state.take_overlay_acceptance(), None);
}

// 폭이 너무 좁어 panel이 hidden인 frame에서는 Esc first-refusal이 비활성화되어 active
// Turn의 기존 interrupt 정책이 그대로 동작한다.
#[test]
fn hidden_overlay_does_not_steal_active_turn_escape() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();
    present_overlay(&mut state, Size::new(2, 4));

    assert_eq!(
        state
            .handle(
                key(KeyCode::Escape, crate::input::event::KeyModifiers::NONE),
                Duration::ZERO,
            )
            .unwrap(),
        StateEffect::Dispatch(AgentAction::Interrupt)
    );
}

// backpressure 중에도 visible overlay의 이동·닫기 입력은 normal command lane을 기다리지
// 않고 state까지 도달하며, Chat transcript scroll은 같은 key를 중복 소비하지 않는다.
#[test]
fn backpressure_still_services_visible_overlay_navigation() {
    let mut state = TuiState::new();
    present_overlay(&mut state, Size::new(40, 12));

    assert_eq!(
        handle_backpressured_input(
            &mut state,
            key(KeyCode::Down, crate::input::event::KeyModifiers::NONE),
            Duration::ZERO,
            false,
        )
        .unwrap(),
        StateEffect::Redraw
    );
    assert!(!state.views().chat_has_pending_scroll());
    assert_eq!(
        handle_backpressured_input(
            &mut state,
            key(KeyCode::Escape, crate::input::event::KeyModifiers::NONE),
            Duration::ZERO,
            false,
        )
        .unwrap(),
        StateEffect::Redraw
    );
}

// command lane이 막힌 동안에도 F2 global view switch는 visible·hidden overlay보다 먼저
// state에 도달하고, 두 경우 모두 slot을 닫은 Transcript view로 전환한다.
#[test]
fn backpressure_preserves_global_view_switch_priority_over_overlay() {
    for size in [Size::new(40, 12), Size::new(2, 4)] {
        let mut state = TuiState::new();
        present_overlay(&mut state, size);

        assert_eq!(
            handle_backpressured_input(
                &mut state,
                key(
                    KeyCode::Function(2),
                    crate::input::event::KeyModifiers::NONE,
                ),
                Duration::ZERO,
                false,
            )
            .unwrap(),
            StateEffect::Redraw
        );
        assert_eq!(
            state.views().active(),
            super::super::view::ObservabilityView::Transcript
        );
        assert_eq!(
            state.open_overlay(overlay_snapshot()),
            Err(SlotError::ChatNotVisible)
        );
    }
}

// Chat에서 연 panel은 Transcript로 전환할 때 닫혀, Chat으로 돌아와도 이전 panel이
// 보이지 않으며 보이지 않는 입력 owner로 부활하지 않는다.
#[test]
fn switching_away_from_chat_closes_overlay_without_resurrection() {
    let mut state = TuiState::new();
    present_overlay(&mut state, Size::new(40, 12));

    for function in [2, 1] {
        assert_eq!(
            state
                .handle(
                    key(
                        KeyCode::Function(function),
                        crate::input::event::KeyModifiers::NONE,
                    ),
                    Duration::ZERO,
                )
                .unwrap(),
            StateEffect::Redraw
        );
    }
    let frame = state
        .prepare_frame(Size::new(40, 12), &AppearanceState::default().pin())
        .unwrap();
    assert!(!frame.overlay_presented);
}

// agent가 approval interaction을 게시하면 prompt 입력 소유권을 넘기기 전에 기존 overlay를
// 닫아, 선택 panel이 request 응답을 가리지 않는다.
#[test]
fn agent_requested_interaction_closes_prompt_overlay() {
    let mut state = TuiState::new();
    present_overlay(&mut state, Size::new(40, 12));

    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(9),
            kind: ActivityKind::ApprovalRequest {
                request_id: RequestId::new(nonzero(3)),
            },
        })
        .unwrap();

    let frame = state
        .prepare_frame(Size::new(40, 12), &AppearanceState::default().pin())
        .unwrap();
    assert!(!frame.overlay_presented);
    assert_eq!(
        state.open_overlay(overlay_snapshot()),
        Err(SlotError::AgentInteractionPending)
    );
}

// prompt overlay는 Chat 전용이므로 Transcript에서 새 panel을 열 수 없고, Chat으로
// 돌아온 뒤에는 같은 snapshot을 정상적으로 열 수 있다.
#[test]
fn non_chat_view_rejects_new_prompt_overlay() {
    let mut state = TuiState::new();
    state
        .handle(
            key(
                KeyCode::Function(2),
                crate::input::event::KeyModifiers::NONE,
            ),
            Duration::ZERO,
        )
        .unwrap();

    assert_eq!(
        state.open_overlay(overlay_snapshot()),
        Err(SlotError::ChatNotVisible)
    );
    state
        .handle(
            key(
                KeyCode::Function(1),
                crate::input::event::KeyModifiers::NONE,
            ),
            Duration::ZERO,
        )
        .unwrap();
    assert!(state.open_overlay(overlay_snapshot()).is_ok());
}
