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

// account section의 자연 header가 위로 밀려나면 같은 header를 첫 행에 고정하고,
// 선택이 다음 section으로 넘어간 순간 새 account header로 교체한다.
#[test]
fn section_header_sticks_without_duplication_and_swaps_at_the_boundary() {
    let mut entries = vec![SelectionEntry::section("codex", "Codex · yon@example.com")];
    entries
        .extend((0..5).map(|index| enabled(&format!("codex-{index}"), &format!("Codex {index}"))));
    entries.push(SelectionEntry::section("grok", "Grok · yon@example.com"));
    entries.extend((0..3).map(|index| enabled(&format!("grok-{index}"), &format!("Grok {index}"))));
    let mut panel = SelectionPanel::new(snapshot(entries));
    for _ in 0..4 {
        panel.next();
    }

    let (codex, _) = render(&panel, Size::new(42, 7)).unwrap();
    let codex_rows = (1..codex.size().height - 1)
        .map(|y| row(&codex, y))
        .collect::<Vec<_>>();
    assert_eq!(
        codex_rows
            .iter()
            .filter(|line| line.contains("Codex · yon@example.com"))
            .count(),
        1
    );
    assert!(codex_rows[0].contains("Codex · yon@example.com"));
    assert!(codex_rows.last().unwrap().contains("› Codex 4"));

    panel.next();
    let (grok, _) = render(&panel, Size::new(42, 7)).unwrap();
    let grok_rows = (1..grok.size().height - 1)
        .map(|y| row(&grok, y))
        .collect::<Vec<_>>();
    assert!(grok_rows[0].contains("Grok · yon@example.com"));
    assert_eq!(
        grok_rows
            .iter()
            .filter(|line| line.contains("Grok · yon@example.com"))
            .count(),
        1
    );
    assert!(grok_rows.iter().all(|line| !line.contains("Codex ·")));
}

// 다음 section header 하나만 남는 창은 header를 고아로 표시하지 않고 숨김으로 남겨,
// account와 그 첫 상태 또는 model 행이 항상 함께 나타나게 한다.
#[test]
fn viewport_never_leaves_a_section_header_as_its_last_content_row() {
    let panel = SelectionPanel::new(snapshot(vec![
        SelectionEntry::section("codex", "Codex · yon@example.com"),
        enabled("codex-0", "Codex 0"),
        enabled("codex-1", "Codex 1"),
        SelectionEntry::section("grok", "Grok · yon@example.com"),
        enabled("grok-0", "Grok 0"),
    ]));

    let (surface, size) = render(&panel, Size::new(42, 6)).unwrap();

    assert_eq!(size.height, 5);
    assert!(row(&surface, 1).contains("Codex · yon@example.com"));
    assert!(row(&surface, 2).contains("› Codex 0"));
    assert!(row(&surface, 3).contains("Codex 1"));
    assert!(!row(&surface, 3).contains("Grok ·"));
    assert!(row(&surface, 4).contains("2↓"));
}

// 선택 가능한 행이 없는 snapshot도 창 끝의 다음 account header만 홀로 노출하지 않는다.
// section을 제거한 만큼 hidden count에 남겨 account와 소유 행을 함께 보여 준다.
#[test]
fn unselected_viewport_also_trims_an_orphaned_section_header() {
    let panel = SelectionPanel::new(snapshot(vec![
        SelectionEntry::section("codex", "Codex · yon@example.com"),
        SelectionEntry::disabled("codex-0", "Codex 0", None, "offline"),
        SelectionEntry::disabled("codex-1", "Codex 1", None, "offline"),
        SelectionEntry::section("grok", "Grok · yon@example.com"),
        SelectionEntry::disabled("grok-0", "Grok 0", None, "offline"),
    ]));

    let (surface, size) = render(&panel, Size::new(42, 6)).unwrap();

    assert_eq!(size.height, 5);
    assert!(row(&surface, 1).contains("Codex · yon@example.com"));
    assert!(row(&surface, 2).contains("Codex 0"));
    assert!(row(&surface, 3).contains("Codex 1"));
    assert!(!row(&surface, 3).contains("Grok ·"));
    assert!(row(&surface, 4).contains("2↓"));
}

// section ownership과 선택 index는 snapshot 수립 때 계산되므로, 깊은 catalog의 마지막
// enabled model을 그릴 때도 준비 경로는 전체 앞부분을 다시 역방향 탐색하지 않는다.
#[test]
fn deep_selected_entry_keeps_its_precomputed_owning_section() {
    let mut entries = vec![SelectionEntry::section(
        "large-account",
        "Large account · yon@example.com",
    )];
    entries.extend((0..4_096).map(|index| {
        SelectionEntry::disabled(
            format!("disabled-{index}"),
            format!("Disabled {index}"),
            None,
            "offline",
        )
    }));
    entries.push(enabled("current", "Current model"));
    let panel = SelectionPanel::new(snapshot(entries));

    let (surface, size) = render(&panel, Size::new(42, 7)).unwrap();

    assert_eq!(size.height, 7);
    assert!(row(&surface, 1).contains("Large account · yon@example.com"));
    assert!(row(&surface, 5).contains("› Current model"));
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
    let token = slot.open(snapshot(vec![enabled("one", "One")])).unwrap();
    slot.set_presented(true);

    assert_eq!(
        slot.handle(&key(KeyCode::Character('c'), KeyModifiers::CONTROL)),
        OverlayInputEffect::Unhandled
    );
    assert!(slot.panel().is_some());
    assert_eq!(
        slot.handle(&key(KeyCode::Escape, KeyModifiers::NONE)),
        OverlayInputEffect::Dismissed(token)
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
    let refreshed = slot.presentation().unwrap();
    assert_eq!(
        slot.handle(&key(KeyCode::Enter, KeyModifiers::NONE)),
        OverlayInputEffect::Consumed
    );
    assert!(slot.commit_presentation(refreshed, true));
    let OverlayInputEffect::Accepted(receipt) =
        slot.handle(&key(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("fresh snapshot must accept its preserved enabled selection");
    };
    assert_eq!(receipt.identity(), "one");
}

// 이미 표시된 instance를 refresh하면 이전 frame receipt는 새 snapshot의 selection gate를
// 해제할 수 없고, 정확히 새 revision이 표시된 뒤에만 Enter가 다시 acceptance된다.
#[test]
fn stale_presentation_receipt_cannot_release_a_refreshed_panel() {
    let mut slot = PromptOverlaySlot::default();
    let token = slot.open(snapshot(vec![enabled("one", "One")])).unwrap();
    let first = slot.presentation().unwrap();
    assert!(slot.commit_presentation(first, true));

    slot.refresh(token, snapshot(vec![enabled("two", "Two")]))
        .unwrap();
    let refreshed = slot.presentation().unwrap();
    assert_ne!(refreshed, first);
    assert!(!slot.commit_presentation(first, true));
    assert_eq!(
        slot.handle(&key(KeyCode::Enter, KeyModifiers::NONE)),
        OverlayInputEffect::Consumed
    );

    assert!(slot.commit_presentation(refreshed, true));
    let OverlayInputEffect::Accepted(receipt) =
        slot.handle(&key(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("matching refreshed presentation must release acceptance");
    };
    assert_eq!(receipt.identity(), "two");
}

// layout에 가려져 한 번도 보이지 않은 panel의 refresh는 selection 입력을 새로 소유하지
// 않는다. matching hidden commit 뒤에도 editor가 Enter와 Esc를 계속 받는다.
#[test]
fn hidden_refresh_remains_unpresented_and_yields_input() {
    let mut slot = PromptOverlaySlot::default();
    let token = slot.open(snapshot(vec![enabled("one", "One")])).unwrap();
    let hidden = slot.presentation().unwrap();
    assert!(slot.commit_presentation(hidden, false));

    slot.refresh(token, snapshot(vec![enabled("two", "Two")]))
        .unwrap();
    let refreshed = slot.presentation().unwrap();
    assert_eq!(refreshed, hidden);
    assert!(slot.commit_presentation(refreshed, false));
    assert_eq!(
        slot.handle(&key(KeyCode::Enter, KeyModifiers::NONE)),
        OverlayInputEffect::Unhandled
    );
    assert_eq!(
        slot.handle(&key(KeyCode::Escape, KeyModifiers::NONE)),
        OverlayInputEffect::Unhandled
    );
}
