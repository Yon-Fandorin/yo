use super::{
    super::{
        OverlayBindings, PanelPaintError, SelectionEntry, SelectionPanel, SelectionPanelAppearance,
        SelectionPanelGlyphs, SelectionPanelStyles,
    },
    support::{enabled, snapshot},
};
use crate::{
    appearance::AppearanceState,
    surface::{
        Attributes, CellContent, Color, Grapheme, Point, Rect, Size, Style, Surface, WriteOutcome,
    },
};

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

pub(super) fn render(panel: &SelectionPanel, size: Size) -> Option<(Surface, Size)> {
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

pub(super) fn row(surface: &Surface, y: u16) -> String {
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

// activity title status는 글자를 바꾸지 않고 같은 연속 phase로 밝기를 이동한다.
// 따라서 두 frame의 문구와 geometry는 같고, shell과 같은 16ms motion demand를 낸다.
#[test]
fn activity_title_status_moves_a_style_only_sheen_without_relayout() {
    let panel = SelectionPanel::new(
        snapshot(vec![enabled("src", "src/")])
            .with_activity_title_status("Searching")
            .unwrap(),
    );

    let (first, first_size, first_period) = render_with_motion(
        &panel,
        Size::new(48, 6),
        std::time::Duration::from_millis(500),
    )
    .unwrap();
    let (second, second_size, second_period) = render_with_motion(
        &panel,
        Size::new(48, 6),
        std::time::Duration::from_millis(1_000),
    )
    .unwrap();

    assert_eq!(row(&first, 0), row(&second, 0));
    assert_eq!(first_size, second_size);
    assert_eq!(first_period, Some(std::time::Duration::from_millis(16)));
    assert_eq!(second_period, first_period);
    assert_ne!(first, second);
    assert!((14..23).any(|x| {
        first.cell(Point::new(x, 0)).unwrap().style()
            != second.cell(Point::new(x, 0)).unwrap().style()
    }));
}

// 일반 static status는 timer를 요구하지 않지만 한 grapheme activity status는 marker처럼
// pulse 밝기가 바뀔 수 있으므로 16ms motion demand를 유지한다.
#[test]
fn static_status_stays_still_while_one_grapheme_activity_pulses() {
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
        Some(std::time::Duration::from_millis(16))
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
// 선택된 이름은 marker와 같은 selected 역할로 강조하되 context와 종류는 보조 계층을 유지한다.
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
        appearance().styles.selected
    );
    assert_eq!(
        surface.cell(Point::new(15, 1)).unwrap().style(),
        appearance().styles.detail
    );
    assert_eq!(
        surface.cell(Point::new(59, 1)).unwrap().style(),
        appearance().styles.detail
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
