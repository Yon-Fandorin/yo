use std::time::Duration;

use super::{
    ActivityMotionProfile, ActivityRgb, ActivityStyles, ColorCapability, MotionPreference,
    blend_channel, resolve_style, sweep_position,
};
use crate::surface::{Attributes, Color, Style};

// 2초 주기의 시작과 끝은 sweep가 label 바깥 열 칸에 있어 모든 글자 밝기가 0이고,
// modulo로 처음으로 돌아가도 경계 frame이 눈에 띄게 튀지 않는다.
#[test]
fn padded_sweep_is_dark_at_both_period_edges() {
    let profile = ActivityMotionProfile::built_in(
        &["✦"],
        ColorCapability::TrueColor,
        MotionPreference::Standard,
    );
    let first = profile.frame_at(Duration::ZERO).sheen(7).unwrap();
    let last = profile
        .frame_at(Duration::from_nanos(1_999_999_999))
        .sheen(7)
        .unwrap();

    for index in 0..7 {
        assert_eq!(first.intensity_at(index), 0.0);
        assert!(last.intensity_at(index) < 0.000_001);
    }
}

// 폭 1인 점, 폭 2인 한글, 두 개의 ASCII grapheme가 섞여도 profile은 각 frame의
// 셀 폭 합 중 최댓값 2를 계속 예약하고, 80ms 경계에서 elapsed 기반 frame만 선택한다.
#[test]
fn marker_frames_use_elapsed_phase_and_reserve_the_widest_frame() {
    let profile = ActivityMotionProfile::built_in(
        &[".", "가", "oo"],
        ColorCapability::Unknown,
        MotionPreference::Standard,
    );

    let first = profile.frame_at(Duration::ZERO);
    let wide = profile.frame_at(Duration::from_millis(80));
    let multi = profile.frame_at(Duration::from_millis(160));
    let skipped = profile.frame_at(Duration::from_millis(245));

    assert_eq!((first.marker(), first.marker_width()), (".", 1));
    assert_eq!((wide.marker(), wide.marker_width()), ("가", 2));
    assert_eq!((multi.marker(), multi.marker_width()), ("oo", 2));
    assert_eq!(skipped.marker(), ".");
    assert_eq!(first.reserved_marker_width(), 2);
    assert_eq!(wide.reserved_marker_width(), 2);
}

// reduced motion은 elapsed가 여러 frame 주기를 지나도 첫 frame을 고정하고 timer를
// 요청하지 않되, layout이 사용하는 최대 예약 폭 정보는 표준 모드와 동일하게 유지한다.
#[test]
fn reduced_motion_freezes_the_first_frame_without_losing_reserved_width() {
    let profile = ActivityMotionProfile::built_in(
        &[".", "가"],
        ColorCapability::Unknown,
        MotionPreference::Reduced,
    );
    let frame = profile.frame_at(Duration::from_secs(9));

    assert_eq!(frame.marker(), ".");
    assert_eq!(frame.reserved_marker_width(), 2);
    assert_eq!(frame.period(), None);
}

// 연속 위치를 정수 peak로 먼저 줄이지 않아 같은 글자도 16ms가 흐르면 서로 다른 RGB로
// 해석되고, TrueColor 해석은 appearance가 정한 배경과 속성을 그대로 보존한다.
#[test]
fn fractional_position_changes_true_color_between_repaints() {
    let profile = ActivityMotionProfile::built_in(
        &["✦"],
        ColorCapability::TrueColor,
        MotionPreference::Standard,
    );
    let styles = ActivityStyles {
        trail: Style::new(Color::Default, Color::Indexed(4), Attributes::DIM),
        ..ActivityStyles::built_in()
    };
    let first = profile
        .frame_at(Duration::from_millis(500))
        .sheen(7)
        .unwrap()
        .style_at(0, styles);
    let second = profile
        .frame_at(Duration::from_millis(516))
        .sheen(7)
        .unwrap()
        .style_at(0, styles);

    assert_ne!(first.foreground, second.foreground);
    assert!(matches!(first.foreground, Color::Rgb { .. }));
    assert_eq!(first.background, Color::Indexed(4));
    assert_eq!(first.attributes, Attributes::DIM);
}

// Limited와 Unknown은 RGB를 내보내지 않고 같은 intensity를 dim/default/bold의
// 세 구간으로만 해석해 낮은 색상 깊이에서도 계약을 정직하게 유지한다.
#[test]
fn lower_depth_capabilities_use_bounded_attribute_roles() {
    let styles = ActivityStyles::built_in();
    for capability in [ColorCapability::Limited, ColorCapability::Unknown] {
        let profile =
            ActivityMotionProfile::built_in(&["*"], capability, MotionPreference::Standard);
        let frame = profile.frame_at(Duration::from_millis(1_000));
        let sheen = frame.sheen(1).unwrap();
        let style = sheen.style_at(0, styles);

        assert!(!matches!(style.foreground, Color::Rgb { .. }));
        assert!(
            matches!(style.attributes, Attributes::DIM | Attributes::BOLD)
                || style.attributes == Attributes::empty()
        );
    }
}

// fallback의 0.2와 0.6 경계는 각각 default와 bold 구간에 포함되어, 부동소수점
// 비교식이 문서의 반열린 구간과 다르게 구현되는 회귀를 막는다.
#[test]
fn fallback_thresholds_use_the_documented_half_open_ranges() {
    let styles = ActivityStyles {
        marker: Style::default(),
        muted: Style::new(Color::Indexed(1), Color::Default, Attributes::DIM),
        trail: Style::new(Color::Indexed(2), Color::Default, Attributes::empty()),
        peak: Style::new(Color::Indexed(3), Color::Default, Attributes::BOLD),
    };
    let base = ActivityRgb::new(0, 0, 0);
    let highlight = ActivityRgb::new(255, 255, 255);

    assert_eq!(
        resolve_style(
            styles,
            styles.trail,
            ColorCapability::Limited,
            base,
            highlight,
            0.19,
        ),
        styles.muted
    );
    assert_eq!(
        resolve_style(
            styles,
            styles.trail,
            ColorCapability::Limited,
            base,
            highlight,
            0.2,
        ),
        styles.trail
    );
    assert_eq!(
        resolve_style(
            styles,
            styles.trail,
            ColorCapability::Unknown,
            base,
            highlight,
            0.59,
        ),
        styles.trail
    );
    assert_eq!(
        resolve_style(
            styles,
            styles.trail,
            ColorCapability::Unknown,
            base,
            highlight,
            0.6,
        ),
        styles.peak
    );
}

// reduced-motion은 고정 marker와 정적 style을 유지하고 16ms timer를 요청하지 않아,
// 접근성 선택이 숨은 repaint 작업으로 남지 않는다.
#[test]
fn reduced_motion_disarms_timed_repaint() {
    let profile = ActivityMotionProfile::built_in(
        &["✦"],
        ColorCapability::TrueColor,
        MotionPreference::Reduced,
    );
    let frame = profile.frame_at(Duration::from_secs(9));

    assert_eq!(frame.marker(), "✦");
    assert_eq!(frame.period(), None);
    assert_eq!(frame.sheen(7), None);
}

// RGB 채널 보간은 계약의 0.9 배율을 적용한 뒤 가장 가까운 정수로 반올림하므로
// 플랫폼이나 renderer마다 다른 색을 만들지 않는다.
#[test]
fn rgb_channel_interpolation_rounds_deterministically() {
    assert_eq!(blend_channel(128, 255, 0.9), 242);
    assert_eq!(
        ActivityRgb::new(128, 128, 128).blend(ActivityRgb::new(255, 255, 255), 1.0),
        Color::Rgb {
            red: 242,
            green: 242,
            blue: 242,
        }
    );
}

// label 길이가 달라도 시작 위치는 -10으로 같고 끝에는 label 오른쪽 열 칸까지 지나가,
// 고정 padding 식이 실제 visible grapheme 수를 반영한다.
#[test]
fn sweep_position_scales_with_visible_grapheme_count() {
    assert_eq!(
        sweep_position(Duration::ZERO, Duration::from_secs(2), 7),
        -10.0
    );
    assert_eq!(
        sweep_position(Duration::from_secs(1), Duration::from_secs(2), 7),
        3.5
    );
}
