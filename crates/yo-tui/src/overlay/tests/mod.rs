use super::{
    OverlayBindings, PanelSnapshot, PromptOverlaySlot, SelectionEntry, SelectionPanel,
    SelectionPanelAppearance, SelectionPanelGlyphs, SelectionPanelStyles, SlotError,
    selection::PanelPaintError, slot::OverlayInputEffect,
};
use crate::{
    appearance::AppearanceState,
    input::event::{InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers, KeyState},
    surface::{
        Attributes, CellContent, Color, Grapheme, Point, Rect, Size, Style, Surface, WriteOutcome,
    },
};

mod validation;

fn enabled(id: &str, label: &str) -> SelectionEntry {
    SelectionEntry::enabled(id, label, None)
}

fn snapshot(entries: Vec<SelectionEntry>) -> PanelSnapshot {
    PanelSnapshot::new("Commands", entries).unwrap()
}

fn appearance() -> SelectionPanelAppearance {
    let plain = Style::default();
    SelectionPanelAppearance {
        styles: SelectionPanelStyles {
            activity: crate::appearance::ActivityStyles {
                marker: Style::new(Color::Indexed(6), Color::Default, Attributes::empty()),
                muted: Style::new(Color::Default, Color::Default, Attributes::DIM),
                trail: Style::new(Color::Indexed(6), Color::Default, Attributes::DIM),
                peak: Style::new(Color::Indexed(6), Color::Default, Attributes::empty()),
            },
            background: plain,
            frame: Style::new(Color::Default, Color::Default, Attributes::DIM),
            title: Style::new(Color::Default, Color::Default, Attributes::BOLD),
            key_hint: Style::new(Color::Default, Color::Default, Attributes::BOLD),
            hint: Style::new(Color::Default, Color::Default, Attributes::DIM),
            label: plain,
            detail: Style::new(Color::Default, Color::Default, Attributes::DIM),
            selected: Style::new(Color::Default, Color::Default, Attributes::BOLD),
            disabled: Style::new(Color::Default, Color::Default, Attributes::DIM),
        },
        glyphs: SelectionPanelGlyphs::rich(),
    }
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> InputEvent {
    InputEvent::Key(KeyEvent {
        code,
        modifiers,
        action: KeyAction::Press,
        state: KeyState::NONE,
    })
}

fn render(panel: &SelectionPanel, size: Size) -> Option<(Surface, Size)> {
    render_for_turn(panel, size, false)
}

fn render_for_turn(
    panel: &SelectionPanel,
    size: Size,
    turn_active: bool,
) -> Option<(Surface, Size)> {
    let prepared = panel.prepare(size, appearance(), &OverlayBindings::default(), turn_active)?;
    let prepared_size = prepared.size();
    let mut surface = Surface::new(prepared_size).unwrap();
    let mut view = surface
        .view(Rect::new(Point::new(0, 0), prepared_size))
        .unwrap();
    prepared.paint(&mut view).unwrap();
    Some((surface, prepared_size))
}

fn render_with_motion(
    panel: &SelectionPanel,
    size: Size,
    elapsed: std::time::Duration,
) -> Option<(Surface, Size, Option<std::time::Duration>)> {
    let appearance_state = AppearanceState::default();
    let pin = appearance_state.pin();
    let prepared = panel.prepare_with_motion(
        size,
        appearance(),
        &OverlayBindings::default(),
        false,
        pin.snapshot().activity_motion_frame(elapsed),
    )?;
    let prepared_size = prepared.size();
    let motion_period = prepared.motion_period();
    let mut surface = Surface::new(prepared_size).unwrap();
    let mut view = surface
        .view(Rect::new(Point::new(0, 0), prepared_size))
        .unwrap();
    prepared.paint(&mut view).unwrap();
    Some((surface, prepared_size, motion_period))
}

fn row(surface: &Surface, y: u16) -> String {
    let mut rendered = String::new();
    for x in 0..surface.size().width {
        match surface.cell(Point::new(x, y)).unwrap().content() {
            CellContent::Blank => rendered.push(' '),
            CellContent::Continuation { .. } => {},
            CellContent::Grapheme { text, .. } => rendered.push_str(text),
        }
    }
    rendered.trim_end().to_owned()
}

// activity title status는 글자를 바꾸지 않고 같은 120ms phase로 peak와 trail을 이동한다.
// 따라서 두 frame의 문구와 geometry는 같고, shell과 같은 역할 style만 한 칸씩 전진한다.
#[test]
fn activity_title_status_moves_a_style_only_sheen_without_relayout() {
    let panel = SelectionPanel::new(
        snapshot(vec![enabled("src", "src/")])
            .with_activity_title_status("Searching")
            .unwrap(),
    );

    let (first, first_size, first_period) =
        render_with_motion(&panel, Size::new(48, 6), std::time::Duration::ZERO).unwrap();
    let (second, second_size, second_period) = render_with_motion(
        &panel,
        Size::new(48, 6),
        std::time::Duration::from_millis(120),
    )
    .unwrap();

    assert_eq!(row(&first, 0), row(&second, 0));
    assert_eq!(first_size, second_size);
    assert_eq!(first_period, Some(std::time::Duration::from_millis(120)));
    assert_eq!(second_period, first_period);
    assert_ne!(first, second);
    let activity = appearance().styles.activity;
    assert_eq!(
        first.cell(Point::new(14, 0)).unwrap().style(),
        activity.peak
    );
    assert_eq!(
        first.cell(Point::new(15, 0)).unwrap().style(),
        activity.trail
    );
    assert_eq!(
        second.cell(Point::new(14, 0)).unwrap().style(),
        activity.trail
    );
    assert_eq!(
        second.cell(Point::new(15, 0)).unwrap().style(),
        activity.peak
    );
}

// static status와 한 grapheme뿐인 activity status는 이후 phase가 화면을 바꿀 수 없으므로
// timer를 요구하지 않는다. 한 frame profile과 같은 no-op 경계다.
#[test]
fn static_or_single_grapheme_status_does_not_demand_motion() {
    let static_panel = SelectionPanel::new(
        snapshot(vec![enabled("src", "src/")])
            .with_title_status("Ready")
            .unwrap(),
    );
    let single_panel = SelectionPanel::new(
        snapshot(vec![enabled("src", "src/")])
            .with_activity_title_status("S")
            .unwrap(),
    );

    assert_eq!(
        render_with_motion(&static_panel, Size::new(48, 6), std::time::Duration::ZERO)
            .unwrap()
            .2,
        None
    );
    assert_eq!(
        render_with_motion(&single_panel, Size::new(48, 6), std::time::Duration::ZERO)
            .unwrap()
            .2,
        None
    );
}

// panel은 title·현재 key hint·선택 marker·상세 설명을 같은 폭 안에 그리고,
// 선택된 enabled 항목의 opaque identity를 그대로 유지한다.
#[test]
fn renders_rib_shaped_selection_panel_from_semantic_entries() {
    let panel = SelectionPanel::new(snapshot(vec![
        SelectionEntry::enabled("resume", "Resume session", Some("continue locally".into())),
        SelectionEntry::disabled("remote", "Remote session", None, "not connected"),
    ]));

    let (surface, size) = render_for_turn(&panel, Size::new(54, 8), true).unwrap();

    assert_eq!(size, Size::new(54, 4));
    assert!(row(&surface, 0).contains("Commands"));
    assert!(row(&surface, 0).contains("[Esc] close · [^C] interrupt"));
    assert!(row(&surface, 1).contains("› Resume session"));
    assert!(row(&surface, 1).contains("continue locally"));
    assert!(row(&surface, 2).contains("Remote session"));
    assert!(row(&surface, 2).contains("not connected"));
    assert_eq!(panel.selected_identity().unwrap().as_str(), "resume");
}

// workspace형 후보는 이름과 부모 경로를 왼쪽 읽기 흐름으로 붙이고 종류만 오른쪽 끝에 둔다.
// 선택 강조는 marker가 담당하므로 파일 이름 자체에는 별도 selected 색을 입히지 않는다.
#[test]
fn renders_codex_shaped_path_context_with_a_trailing_kind() {
    let panel = SelectionPanel::new(snapshot(vec![
        SelectionEntry::enabled_with_context(
            "main",
            "main.rs",
            Some("crates/yo-cli/src/".into()),
            Some("File".into()),
        ),
        SelectionEntry::enabled_with_context(
            "directory",
            "workspace/",
            Some("crates/yo-core/src/".into()),
            Some("Dir".into()),
        ),
    ]));

    let (surface, _) = render(&panel, Size::new(64, 4)).unwrap();

    assert!(row(&surface, 1).contains("› main.rs     crates/yo-cli/src/"));
    assert!(row(&surface, 1).ends_with("File│"));
    assert!(row(&surface, 2).contains("  workspace/  crates/yo-core/src/"));
    assert_eq!(
        surface.cell(Point::new(3, 1)).unwrap().style(),
        Style::default()
    );
}

// provenance filter는 bottom-left 한 영역에만 나타나며 선택한 항목을 강조한다.
// 좁은 폭에서는 frame과 오른쪽 hidden count를 침범하지 않고 안전하게 잘린다.
#[test]
fn renders_optional_filters_only_in_the_bottom_left_footer() {
    let panel = SelectionPanel::new(
        snapshot(vec![enabled("one", "One")])
            .with_filter_bar(["All", "Workspace", "User", "System", "Admin"], 1)
            .unwrap(),
    );

    let (wide, _) = render(&panel, Size::new(58, 4)).unwrap();
    assert!(
        row(&wide, 2).contains("← All · Workspace · User · System · Admin →"),
        "{}",
        row(&wide, 2)
    );
    assert!(!row(&wide, 0).contains("Workspace"));
    let (narrow, _) = render(&panel, Size::new(28, 4)).unwrap();
    assert!(row(&narrow, 2).starts_with("╰─← All · Workspace"));
    assert!(row(&narrow, 2).ends_with('╯'));
}

// 좁은 panel은 detail과 disabled reason을 먼저 버리고 label만 grapheme 경계에서 줄여,
// 한글 wide cell이나 frame 밖을 침범하지 않는다.
#[test]
fn narrow_panel_drops_secondary_text_before_grapheme_safe_truncation() {
    let panel = SelectionPanel::new(snapshot(vec![SelectionEntry::enabled(
        "wide",
        "가나다라마바사",
        Some("secondary detail".into()),
    )]));

    let (surface, size) = render(&panel, Size::new(16, 4)).unwrap();

    assert_eq!(size, Size::new(16, 3));
    assert_eq!(row(&surface, 1), "│› 가나다라마… │");
    assert!(!row(&surface, 1).contains("secondary"));
}

// border 두 행과 항목 한 행을 담지 못하는 목적지는 hidden으로 판정되어 paint 대상이
// 만들어지지 않는다.
#[test]
fn insufficient_destination_hides_without_preparing_paint() {
    let panel = SelectionPanel::new(snapshot(vec![enabled("one", "One")]));

    let bindings = OverlayBindings::default();
    assert_eq!(
        panel.prepare(Size::new(20, 2), appearance(), &bindings, false),
        None
    );
    assert_eq!(
        panel.prepare(Size::new(2, 8), appearance(), &bindings, false),
        None
    );
}

// active Turn에서 Esc close와 Ctrl+C interrupt hint를 함께 표시할 폭이 없으면 panel은
// work row를 억지로 가리지 않도록 hidden으로 판정된다.
#[test]
fn mandatory_active_turn_hints_participate_in_panel_fitting() {
    let panel = SelectionPanel::new(snapshot(vec![enabled("one", "One")]));
    let bindings = OverlayBindings::default();

    assert!(
        panel
            .prepare(Size::new(28, 6), appearance(), &bindings, false)
            .is_some()
    );
    assert_eq!(
        panel.prepare(Size::new(28, 6), appearance(), &bindings, true),
        None
    );
}

// 목적지 밖에서 시작한 wide grapheme footprint가 panel 영역과 교차하면 clear 전에
// SurfaceConflict로 거절해 기존 셀을 하나도 바꾸지 않는다.
#[test]
fn crossing_destination_footprint_preserves_surface_atomically() {
    let panel = SelectionPanel::new(snapshot(vec![enabled("one", "One")]));
    let prepared = panel
        .prepare(
            Size::new(16, 3),
            appearance(),
            &OverlayBindings::default(),
            false,
        )
        .unwrap();
    let mut surface = Surface::new(Size::new(17, 3)).unwrap();
    {
        let mut full = surface
            .view(Rect::new(Point::new(0, 0), Size::new(17, 3)))
            .unwrap();
        assert_eq!(
            full.write(
                Point::new(0, 1),
                Grapheme::try_from("가").unwrap(),
                Style::default(),
            ),
            WriteOutcome::Written
        );
    }
    let before = surface.clone();

    let error = {
        let mut destination = surface
            .view(Rect::new(Point::new(1, 0), Size::new(16, 3)))
            .unwrap();
        prepared.paint(&mut destination).unwrap_err()
    };

    assert_eq!(error, PanelPaintError::SurfaceConflict);
    assert_eq!(surface, before);
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
