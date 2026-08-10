use super::{
    super::{OverlayInputEffect, PromptOverlaySlot, SelectionEntry, SelectionPanel, SlotError},
    render::{render, row},
    support::{enabled, snapshot},
};
use crate::{
    input::event::{InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState},
    surface::Size,
};

fn key(code: KeyCode, modifiers: KeyModifiers) -> InputEvent {
    InputEvent::Key(KeyEvent {
        code,
        modifiers,
        action: KeyAction::Press,
        state: KeyState::NONE,
    })
}

// visible cap보다 긴 목록에서 선택을 아래로 옮기면 선택 행이 window 안에 남고,
// bottom border에는 가려진 위·아래 항목 수가 명시된다.
#[test]
fn selected_identity_drives_visible_window_and_hidden_counts() {
    let entries = (0..12)
        .map(|index| enabled(&format!("id-{index}"), &format!("Entry {index}")))
        .collect();
    let mut panel = SelectionPanel::new(snapshot(entries));
    for _ in 0..9 {
        panel.next();
    }

    let (surface, size) = render(&panel, Size::new(30, 7)).unwrap();

    assert_eq!(size.height, 7);
    assert!(row(&surface, 5).contains("› Entry 9"));
    assert!(row(&surface, 6).contains("↑5 · 2↓"));
}

// 새 snapshot에 같은 enabled identity가 있으면 selection을 보존하고, 사라지거나
// disabled가 되면 provider 순서의 첫 enabled 항목으로 이동한다.
#[test]
fn refresh_preserves_only_a_still_enabled_identity() {
    let mut panel = SelectionPanel::new(snapshot(vec![enabled("a", "A"), enabled("b", "B")]));
    panel.next();
    panel.refresh(snapshot(vec![enabled("b", "B2"), enabled("c", "C")]));
    assert_eq!(panel.selected_identity().unwrap().as_str(), "b");

    panel.refresh(snapshot(vec![
        SelectionEntry::disabled("b", "B", None, "blocked"),
        enabled("c", "C"),
    ]));
    assert_eq!(panel.selected_identity().unwrap().as_str(), "c");
}

// slot을 교체하면 이전 token의 refresh·close·accept는 모두 stale로 거절되고 현재
// panel의 selection이나 생명주기를 바꾸지 않는다.
#[test]
fn replacement_token_rejects_every_stale_operation() {
    let mut slot = PromptOverlaySlot::default();
    let stale = slot.open(snapshot(vec![enabled("old", "Old")])).unwrap();
    let current = slot.open(snapshot(vec![enabled("new", "New")])).unwrap();

    assert_eq!(
        slot.refresh(stale, snapshot(vec![enabled("x", "X")])),
        Err(SlotError::StaleToken)
    );
    assert_eq!(slot.close(stale), Err(SlotError::StaleToken));
    assert_eq!(slot.accept(stale), Err(SlotError::StaleToken));
    assert_eq!(slot.accept(current).unwrap().identity(), "new");
    assert_eq!(slot.accept(current), Err(SlotError::StaleToken));
}

// all-disabled panel은 표시와 navigation을 소비하지만 selection을 만들지 않으며 accept도
// 닫지 않고 typed no-selection 오류를 반환한다.
#[test]
fn all_disabled_panel_remains_displayable_without_acceptance() {
    let mut slot = PromptOverlaySlot::default();
    let token = slot
        .open(snapshot(vec![SelectionEntry::disabled(
            "offline", "Remote", None, "offline",
        )]))
        .unwrap();
    slot.set_presented(true);

    assert_eq!(
        slot.handle(&key(KeyCode::Down, KeyModifiers::NONE)),
        OverlayInputEffect::Redraw
    );
    assert_eq!(slot.accept(token), Err(SlotError::NoSelection));
    assert!(slot.panel().is_some());
}

// visible panel의 Esc는 panel만 닫고, Ctrl+C는 slot이 가로채지 않아 active Turn의
// interrupt 정책으로 계속 전달될 수 있다.
#[test]
fn escape_dismisses_visible_panel_while_ctrl_c_bypasses_it() {
    let mut slot = PromptOverlaySlot::default();
    slot.open(snapshot(vec![enabled("one", "One")])).unwrap();
    slot.set_presented(true);

    assert_eq!(
        slot.handle(&key(KeyCode::Character('c'), KeyModifiers::CONTROL)),
        OverlayInputEffect::Unhandled
    );
    assert!(slot.panel().is_some());
    assert_eq!(
        slot.handle(&key(KeyCode::Escape, KeyModifiers::NONE)),
        OverlayInputEffect::Redraw
    );
    assert!(slot.panel().is_none());
}

// hidden panel은 state에 남아 있어도 key first-refusal을 갖지 않아 editor와 transcript가
// 원래 입력을 처리할 수 있다.
#[test]
fn hidden_panel_does_not_claim_navigation_or_dismissal() {
    let mut slot = PromptOverlaySlot::default();
    slot.open(snapshot(vec![enabled("one", "One")])).unwrap();
    slot.set_presented(false);

    assert_eq!(
        slot.handle(&key(KeyCode::Escape, KeyModifiers::NONE)),
        OverlayInputEffect::Unhandled
    );
    assert!(!slot.wants_input(&key(KeyCode::Down, KeyModifiers::NONE)));
    assert!(slot.panel().is_some());
}

// filter가 없는 기존 overlay는 Left/Right를 가로채지 않고 editor나 다른 화면 정책에
// 그대로 전달하지만, filter가 있는 panel은 좌우 전환 결과를 명시적으로 반환한다.
#[test]
fn left_and_right_are_claimed_only_by_panels_with_filters() {
    let mut plain = PromptOverlaySlot::default();
    plain.open(snapshot(vec![enabled("one", "One")])).unwrap();
    plain.set_presented(true);
    assert!(!plain.wants_input(&key(KeyCode::Left, KeyModifiers::NONE)));
    assert_eq!(
        plain.handle(&key(KeyCode::Left, KeyModifiers::NONE)),
        OverlayInputEffect::Unhandled
    );

    let mut filtered = PromptOverlaySlot::default();
    filtered
        .open(
            snapshot(vec![enabled("one", "One")])
                .with_filter_bar(["All", "User"], 0)
                .unwrap(),
        )
        .unwrap();
    filtered.set_presented(true);
    assert_eq!(
        filtered.handle(&key(KeyCode::Right, KeyModifiers::NONE)),
        OverlayInputEffect::FilterChanged(1)
    );
}

// repeat navigation은 연속 이동을 허용하지만 Enter repeat는 한 선택을 중복 소비하지 않는다.
#[test]
fn acceptance_requires_a_press_and_is_single_consumer() {
    let mut slot = PromptOverlaySlot::default();
    let token = slot.open(snapshot(vec![enabled("one", "One")])).unwrap();
    slot.set_presented(true);
    let repeat_enter = InputEvent::Key(KeyEvent {
        code: KeyCode::Enter,
        modifiers: KeyModifiers::NONE,
        action: KeyAction::Repeat,
        state: KeyState::NONE,
    });

    assert_eq!(slot.handle(&repeat_enter), OverlayInputEffect::Consumed);
    let OverlayInputEffect::Accepted(receipt) =
        slot.handle(&key(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("Enter press must return one acceptance receipt");
    };
    assert_eq!(receipt.token(), token);
    assert_eq!(receipt.identity(), "one");
    assert_eq!(slot.accept(token), Err(SlotError::StaleToken));
}

// replacement 검색 중인 snapshot은 기존 enabled 항목과 선택을 그대로 보여 주지만 Enter를
// 소비만 한다. 일치하는 결과가 도착해 fresh로 돌아간 뒤에만 같은 선택을 accept한다.
#[test]
fn pending_snapshot_gate_blocks_acceptance_without_changing_entry_availability() {
    let mut slot = PromptOverlaySlot::default();
    let token = slot.open(snapshot(vec![enabled("one", "One")])).unwrap();
    slot.set_presented(true);
    slot.set_pending(token, "Searching…").unwrap();

    assert_eq!(
        slot.handle(&key(KeyCode::Enter, KeyModifiers::NONE)),
        OverlayInputEffect::Consumed
    );
    assert!(slot.is_open());
    assert_eq!(
        slot.panel().unwrap().selected_identity().unwrap().as_str(),
        "one"
    );

    slot.refresh(token, snapshot(vec![enabled("one", "One")]))
        .unwrap();
    let OverlayInputEffect::Accepted(receipt) =
        slot.handle(&key(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("fresh snapshot must accept its preserved enabled selection");
    };
    assert_eq!(receipt.identity(), "one");
}
