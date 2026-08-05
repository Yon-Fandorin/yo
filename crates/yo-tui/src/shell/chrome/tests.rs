use std::{num::NonZeroU16, time::Duration};

use super::{
    ShellChromeSnapshot, ShellChromeStyles, StatusGroups, StatusSegment, layout, paint_metrics,
    paint_mode, paint_status_groups, paint_transient,
};
use crate::{
    appearance::{ActivityMotionFrame, ActivityStyles, AppearanceState},
    input::editor::binding::NewlineBinding,
    runner::PresentationMode,
    surface::{Attributes, CellContent, Color, Point, Rect, Size, Style, Surface},
};

fn row(surface: &Surface) -> String {
    (0..surface.size().width)
        .map(
            |x| match surface.cell(Point::new(x, 0)).unwrap().content() {
                CellContent::Blank | CellContent::Continuation { .. } => ' ',
                CellContent::Grapheme { text, .. } => text.chars().next().unwrap(),
            },
        )
        .collect::<String>()
        .trim_end()
        .to_owned()
}

fn snapshot<'a>(backend: &'a str, workspace: &'a str) -> ShellChromeSnapshot<'a> {
    ShellChromeSnapshot {
        turn_active: true,
        backend: Some(backend),
        workspace,
        mode: PresentationMode::Inline,
    }
}

// 충분한 높이는 구분·작업·prompt·metrics·mode를 모두 보존하고 남은 행만 transcript에 준다.
#[test]
fn full_layout_reserves_the_complete_prompt_chrome_stack() {
    let layout = layout(
        Rect::new(Point::new(0, 0), Size::new(80, 12)),
        NonZeroU16::new(3).unwrap(),
        true,
    );

    assert_eq!(layout.transcript.size.height, 5);
    assert_eq!(
        layout.transient,
        Rect::new(Point::new(0, 5), Size::new(80, 2))
    );
    assert_eq!(layout.prompt, Rect::new(Point::new(0, 7), Size::new(80, 3)));
    assert_eq!(
        layout.metrics,
        Rect::new(Point::new(0, 10), Size::new(80, 1))
    );
    assert_eq!(layout.mode, Rect::new(Point::new(0, 11), Size::new(80, 1)));
}

// 낮은 Chat 화면도 두 줄의 대화 기록을 읽을 transcript 바닥을 먼저 보존한다.
#[test]
fn compact_layout_keeps_prompt_and_interrupt_before_footer_detail() {
    let layout = layout(
        Rect::new(Point::new(0, 0), Size::new(40, 4)),
        NonZeroU16::new(4).unwrap(),
        true,
    );

    assert_eq!(layout.transcript.size.height, 2);
    assert_eq!(layout.transient.size.height, 1);
    assert_eq!(layout.prompt.size.height, 1);
    assert_eq!(layout.metrics.size.height, 0);
    assert_eq!(layout.mode.size.height, 0);
}

// 같은 높이에서 idle과 active 전환은 예약된 작업 행의 내용만 바꾸고 prompt 위치는 움직이지 않는다.
#[test]
fn idle_and_active_layouts_keep_the_same_prompt_origin() {
    for height in 2..=12 {
        let area = Rect::new(Point::new(0, 0), Size::new(40, height));
        let prompt = NonZeroU16::new(3).unwrap();
        let idle = layout(area, prompt, false);
        let active = layout(area, prompt, true);

        assert_eq!(idle, active, "height {height} changed shell geometry");
    }
}

// 작업 행은 넓은 화면에서 의미와 두 중단 키를 모두 보여주고 좁아지면 한 줄 안에서 단계적으로
// 축약한다.
#[test]
fn activity_row_drops_description_without_wrapping_interrupt_keys() {
    let styles = ShellChromeStyles {
        activity: ActivityStyles::default(),
        metrics: Style::default(),
        mode: Style::default(),
        key_hint: Style::default(),
    };
    let mut wide = Surface::new(Size::new(48, 1)).unwrap();
    paint_transient(
        &mut wide
            .view(Rect::new(Point::new(0, 0), Size::new(48, 1)))
            .unwrap(),
        snapshot("codex", "~/projects/yo"),
        styles,
        ActivityMotionFrame::still("◐"),
        false,
    )
    .unwrap();
    let mut narrow = Surface::new(Size::new(14, 1)).unwrap();
    paint_transient(
        &mut narrow
            .view(Rect::new(Point::new(0, 0), Size::new(14, 1)))
            .unwrap(),
        snapshot("codex", "~/projects/yo"),
        styles,
        ActivityMotionFrame::still("◐"),
        false,
    )
    .unwrap();

    assert_eq!(row(&wide), "◐ Working");
    assert_eq!(row(&narrow), "◐ Working");

    let mut minimal = Surface::new(Size::new(6, 1)).unwrap();
    paint_transient(
        &mut minimal
            .view(Rect::new(Point::new(0, 0), Size::new(6, 1)))
            .unwrap(),
        snapshot("codex", "~/projects/yo"),
        styles,
        ActivityMotionFrame::still("◐"),
        true,
    )
    .unwrap();
    assert_eq!(row(&minimal), "Esc/^C");
}

// Working 문구와 고정 marker는 같은 elapsed 표본을 사용해 style만 바꾼다.
// 두 frame의 문자열과 폭은 같고 16ms repaint 주기만 renderer에 전달된다.
#[test]
fn working_row_moves_a_fixed_text_sheen_on_the_marker_phase() {
    let styles = ShellChromeStyles {
        activity: ActivityStyles {
            marker: Style::new(Color::Indexed(6), Color::Default, Attributes::empty()),
            muted: Style::new(Color::Default, Color::Default, Attributes::DIM),
            trail: Style::new(Color::Indexed(6), Color::Default, Attributes::DIM),
            peak: Style::new(Color::Indexed(6), Color::Default, Attributes::empty()),
        },
        metrics: Style::default(),
        mode: Style::default(),
        key_hint: Style::default(),
    };
    let appearance = AppearanceState::default();
    let pin = appearance.pin();
    let mut first = Surface::new(Size::new(32, 1)).unwrap();
    let first_period = paint_transient(
        &mut first
            .view(Rect::new(Point::new(0, 0), Size::new(32, 1)))
            .unwrap(),
        snapshot("codex", "~/projects/yo"),
        styles,
        pin.snapshot()
            .activity_motion_frame(Duration::from_millis(500)),
        false,
    )
    .unwrap();
    let mut second = Surface::new(Size::new(32, 1)).unwrap();
    let second_period = paint_transient(
        &mut second
            .view(Rect::new(Point::new(0, 0), Size::new(32, 1)))
            .unwrap(),
        snapshot("codex", "~/projects/yo"),
        styles,
        pin.snapshot()
            .activity_motion_frame(Duration::from_millis(1_000)),
        false,
    )
    .unwrap();

    assert_eq!(
        row(&first).split_once(' ').unwrap().1,
        row(&second).split_once(' ').unwrap().1
    );
    assert_ne!(first, second);
    assert_eq!(row(&first), "✦ Working");
    assert_eq!(row(&second), "✦ Working");
    assert!((0..9).any(|x| {
        first.cell(Point::new(x, 0)).unwrap().style()
            != second.cell(Point::new(x, 0)).unwrap().style()
    }));
    assert_eq!(first_period, Some(Duration::from_millis(16)));
    assert_eq!(second_period, first_period);
}

// 충분한 폭의 하단 도움말은 실제 newline binding과 종료·중단 키를 관례 표기로 보여주고,
// 현재 presentation mode는 같은 행 오른쪽에 남겨 입력창 아래 정보를 한눈에 읽게 한다.
#[test]
fn footer_uses_shared_key_notation_and_keeps_mode_at_the_right_edge() {
    let styles = ShellChromeStyles {
        activity: ActivityStyles::default(),
        metrics: Style::default(),
        mode: Style::default(),
        key_hint: Style::default(),
    };
    let mut surface = Surface::new(Size::new(72, 1)).unwrap();
    paint_mode(
        &mut surface
            .view(Rect::new(Point::new(0, 0), Size::new(72, 1)))
            .unwrap(),
        snapshot("codex", "~/projects/yo"),
        styles,
        NewlineBinding::default(),
        true,
    )
    .unwrap();

    assert_eq!(
        row(&surface),
        "Esc/^C interrupt  ·  S-Enter newline  ·  ^D exit                  inline"
    );
}

// 입력 초안이 비어 있지 않으면 Ctrl+D는 종료 명령이 아니므로 하단 도움말에서 exit를
// 광고하지 않고, 실제로 유효한 newline binding과 mode만 남긴다.
#[test]
fn footer_omits_ctrl_d_exit_while_the_prompt_has_a_draft() {
    let styles = ShellChromeStyles {
        activity: ActivityStyles::default(),
        metrics: Style::default(),
        mode: Style::default(),
        key_hint: Style::default(),
    };
    let mut surface = Surface::new(Size::new(48, 1)).unwrap();
    paint_mode(
        &mut surface
            .view(Rect::new(Point::new(0, 0), Size::new(48, 1)))
            .unwrap(),
        snapshot("codex", "~/projects/yo"),
        styles,
        NewlineBinding::default(),
        false,
    )
    .unwrap();

    assert_eq!(
        row(&surface),
        "Esc/^C interrupt  ·  S-Enter newline      inline"
    );
}

// metrics는 전체 작업 경로가 한 줄에 맞지 않으면 backend만 남겨 경로를 중간에서 잘라 오해시키지
// 않는다.
#[test]
fn metrics_drop_workspace_as_one_segment_when_width_is_insufficient() {
    let mut wide = Surface::new(Size::new(40, 1)).unwrap();
    paint_metrics(
        &mut wide
            .view(Rect::new(Point::new(0, 0), Size::new(40, 1)))
            .unwrap(),
        snapshot("codex", "~/projects/yo"),
        Style::default(),
    )
    .unwrap();
    let mut narrow = Surface::new(Size::new(12, 1)).unwrap();
    paint_metrics(
        &mut narrow
            .view(Rect::new(Point::new(0, 0), Size::new(12, 1)))
            .unwrap(),
        snapshot("codex", "~/projects/yo/with-a-long-tail"),
        Style::default(),
    )
    .unwrap();

    assert_eq!(row(&wide), "codex · ~/projects/yo");
    assert_eq!(row(&narrow), "codex");
}

// 좌우 status 그룹은 같은 행의 양끝을 소유해 이후 model/context 같은 우측 segment를 추가해도
// 하나의 고정 문자열 포맷으로 되돌아가지 않게 한다.
#[test]
fn status_groups_keep_independent_left_and_right_alignment() {
    let mut surface = Surface::new(Size::new(24, 1)).unwrap();
    let groups = StatusGroups {
        left: vec![StatusSegment::new("codex", 100)],
        right: vec![StatusSegment::new("42%", 80)],
    };

    paint_status_groups(
        &mut surface
            .view(Rect::new(Point::new(0, 0), Size::new(24, 1)))
            .unwrap(),
        groups,
        Style::default(),
    )
    .unwrap();

    assert_eq!(row(&surface), "codex                42%");
}

// 한 셀 화면에서 표현할 수 없는 고우선순위 segment는 그 항목만 생략하여, 표시 가능한
// 저우선순위 fallback을 함께 잃거나 frame 전체를 실패시키지 않는다.
#[test]
fn unrenderable_status_segments_are_omitted_instead_of_failing_the_frame() {
    for backend in ["가", "\u{301}"] {
        let mut surface = Surface::new(Size::new(1, 1)).unwrap();
        paint_metrics(
            &mut surface
                .view(Rect::new(Point::new(0, 0), Size::new(1, 1)))
                .unwrap(),
            snapshot(backend, "x"),
            Style::default(),
        )
        .unwrap();
        assert_eq!(row(&surface), "x");
    }
}
