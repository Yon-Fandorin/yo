use std::time::Duration;

use super::{
    AppearanceCandidate, AppearanceCandidateError, AppearanceCommitError, AppearanceGlyphRole,
    AppearanceState, ColorCapability, GlyphProfile, MotionPreference, validate_marker,
};
use crate::surface::{Attributes, Color, GraphemeError, Style};

// 기본 appearance는 Rich 글리프와 첫 revision을 같은 검증 경로로 확정한다.
#[test]
fn default_state_starts_with_a_valid_rich_snapshot() {
    let state = AppearanceState::default();
    let pin = state.pin();

    assert_eq!(pin.revision().get(), 1);
    assert_eq!(pin.snapshot().transcript_config().user_marker(), "❯");
    assert_eq!(pin.snapshot().transcript_config().assistant_marker(), "•");
}

// 내장 profile의 focus accent는 host 색상 능력에 맞춰 해석하되 선택 행에 굵기를 더하지
// 않는다. section label은 같은 publication 안에서도 neutral 기본 style을 유지한다.
#[test]
fn built_in_selection_focus_uses_resolved_theme_accent_without_bold() {
    let cases = [
        (
            ColorCapability::TrueColor,
            Color::Rgb {
                red: 94,
                green: 179,
                blue: 179,
            },
        ),
        (ColorCapability::Limited, Color::Indexed(73)),
        (ColorCapability::Unknown, Color::Default),
    ];

    for profile in [GlyphProfile::Rich, GlyphProfile::Ascii] {
        for (color_capability, expected_foreground) in cases {
            let state =
                AppearanceState::new(AppearanceCandidate::for_profile_with_host_preferences(
                    profile,
                    color_capability,
                    MotionPreference::Standard,
                ))
                .unwrap();
            let styles = state.pin().snapshot().styles().overlay.styles;

            assert_eq!(
                styles.selected,
                Style::new(expected_foreground, Color::Default, Attributes::empty())
            );
            assert_eq!(styles.label, Style::default());
        }
    }
}

// 유효한 후보만 revision을 증가시키며 Rich snapshot 전체를 ASCII snapshot으로 교체한다.
#[test]
fn valid_candidate_replaces_the_whole_snapshot_once() {
    let mut state = AppearanceState::default();
    let old = state.pin();

    let revision = state
        .commit(AppearanceCandidate::for_profile(GlyphProfile::Ascii))
        .unwrap();
    let current = state.pin();

    assert_eq!(revision.get(), 2);
    assert_eq!(old.revision().get(), 1);
    assert_eq!(old.snapshot().transcript_config().user_marker(), "❯");
    assert_eq!(current.snapshot().transcript_config().user_marker(), ">");
    assert_eq!(
        current.snapshot().transcript_config().assistant_marker(),
        "*"
    );
}

// 제어 문자가 든 후보는 명시적으로 거부되고 기존 snapshot과 revision을 보존한다.
#[test]
fn control_marker_is_rejected_without_changing_committed_state() {
    let mut state = AppearanceState::default();
    let before = state.pin();
    let candidate =
        AppearanceCandidate::for_profile(GlyphProfile::Ascii).with_markers_for_test("\u{1b}", "*");

    assert_eq!(
        state.commit(candidate),
        Err(AppearanceCommitError::InvalidCandidate(
            AppearanceCandidateError::MarkerContainsControl {
                role: AppearanceGlyphRole::UserMarker,
            },
        ))
    );
    assert_eq!(state.pin(), before);
}

// 빈 글리프, 복수 글리프, 폭이 없는 글리프도 역할별 오류로 구분해 거부한다.
#[test]
fn structurally_invalid_markers_have_specific_rejections() {
    let cases = [
        (
            "",
            AppearanceCandidateError::EmptyMarker {
                role: AppearanceGlyphRole::UserMarker,
            },
        ),
        (
            "ab",
            AppearanceCandidateError::MarkerMustBeOneGrapheme {
                role: AppearanceGlyphRole::UserMarker,
            },
        ),
        (
            "\u{0301}",
            AppearanceCandidateError::UnrenderableMarker {
                role: AppearanceGlyphRole::UserMarker,
                cause: GraphemeError::ZeroWidth,
            },
        ),
    ];

    for (marker, expected) in cases {
        let mut state = AppearanceState::default();
        let candidate = AppearanceCandidate::for_profile(GlyphProfile::Ascii)
            .with_markers_for_test(marker, "*");
        assert_eq!(
            state.commit(candidate),
            Err(AppearanceCommitError::InvalidCandidate(expected))
        );
    }
}

// 본문 들여쓰기보다 넓은 단일 grapheme은 인접 본문 칸을 침범하므로 거부한다.
#[test]
fn marker_wider_than_the_body_indent_is_rejected() {
    assert_eq!(
        validate_marker("한", AppearanceGlyphRole::UserMarker, 1),
        Err(AppearanceCandidateError::MarkerWiderThanIndent {
            role: AppearanceGlyphRole::UserMarker,
            marker_width: 2,
            body_indent: 1,
        })
    );
}

// Rich와 ASCII 내장 profile은 rib에서 선택한 frame 순서를 80ms마다 elapsed로 고르고,
// marker 형태가 바뀌어도 부드러운 shimmer를 위한 16ms repaint 주기는 그대로 유지한다.
#[test]
fn built_in_activity_profiles_select_marker_frames_at_eighty_milliseconds() {
    let rich = AppearanceState::default().pin();
    let first = rich.snapshot().activity_motion_frame(Duration::ZERO);
    let later = rich
        .snapshot()
        .activity_motion_frame(Duration::from_millis(777));
    assert_eq!(first.marker(), "⠋");
    assert_eq!(later.marker(), "⠏");
    assert_eq!(first.reserved_marker_width(), 1);
    assert_eq!(first.period(), Some(Duration::from_millis(16)));

    let ascii = AppearanceState::new(AppearanceCandidate::for_profile(GlyphProfile::Ascii))
        .unwrap()
        .pin();
    let ascii_frame = ascii.snapshot().activity_motion_frame(Duration::ZERO);
    assert_eq!(ascii_frame.marker(), "|");
    assert_eq!(ascii_frame.period(), Some(Duration::from_millis(16)));
}

// reduced-motion 후보는 같은 snapshot publication 경로를 쓰되 marker와 label sheen 모두
// 정적으로 만들어 runner의 시간 기반 redraw 요구를 제거한다.
#[test]
fn reduced_motion_profile_is_valid_but_does_not_demand_motion() {
    let candidate = AppearanceCandidate::for_profile_with_host_preferences(
        GlyphProfile::Ascii,
        ColorCapability::Unknown,
        MotionPreference::Reduced,
    );
    let state = AppearanceState::new(candidate).unwrap();
    let pin = state.pin();
    let frame = pin.snapshot().activity_motion_frame(Duration::from_secs(9));

    assert_eq!(frame.marker(), "|");
    assert_eq!(frame.period(), None);
    assert_eq!(frame.sheen(7), None);
}

// 한 grapheme label도 연속 intensity가 시간에 따라 달라지므로 marker와 같은 방식으로
// timer demand를 유지하고, 빈 label만 실제로 바꿀 cell이 없어 demand를 만들지 않는다.
#[test]
fn activity_sheen_supports_one_visible_grapheme_but_not_an_empty_label() {
    let state = AppearanceState::default();
    let pin = state.pin();
    let frame = pin.snapshot().activity_motion_frame(Duration::ZERO);

    assert_eq!(frame.sheen(0), None);
    assert!(frame.sheen(1).is_some());
}

// process host가 TrueColor를 명시하면 RGB ramp를 쓰고, 기본 Unknown 생성자는 같은
// appearance 경계에서 RGB를 내보내지 않는 안전한 fallback을 유지한다.
#[test]
fn host_color_capability_selects_rgb_or_safe_fallback() {
    let true_color = AppearanceState::new(AppearanceCandidate::for_profile_with_host_preferences(
        GlyphProfile::Rich,
        ColorCapability::TrueColor,
        MotionPreference::Standard,
    ))
    .unwrap();
    let unknown = AppearanceState::default();
    let true_pin = true_color.pin();
    let unknown_pin = unknown.pin();
    let true_frame = true_pin
        .snapshot()
        .activity_motion_frame(Duration::from_secs(1));
    let unknown_frame = unknown_pin
        .snapshot()
        .activity_motion_frame(Duration::from_secs(1));
    let styles = true_pin.snapshot().styles().chrome.activity;

    assert!(matches!(
        true_frame.marker_style(styles).foreground,
        Color::Rgb { .. }
    ));
    assert!(!matches!(
        unknown_frame.marker_style(styles).foreground,
        Color::Rgb { .. }
    ));
}

// 빈 frame 목록·문자열, 제어·폭 0 grapheme, 잘못된 두 timer와 0초 sweep를 publication
// 전에 구체적인 오류로 거부해 renderer가 불완전한 activity profile을 보지 못하게 한다.
#[test]
fn invalid_activity_profiles_are_rejected_before_publication() {
    let base = AppearanceCandidate::for_profile(GlyphProfile::Rich);
    assert_eq!(
        base.clone().with_activity_motion_for_test(
            Duration::from_millis(16),
            Duration::from_millis(80),
            &[],
        ),
        Err(AppearanceCandidateError::EmptyActivityMarkerFrames)
    );
    assert_eq!(
        base.clone().with_activity_motion_for_test(
            Duration::from_millis(16),
            Duration::from_millis(80),
            &[""],
        ),
        Err(AppearanceCandidateError::EmptyActivityMarkerFrame { frame_index: 0 })
    );
    assert_eq!(
        base.clone().with_activity_motion_for_test(
            Duration::from_millis(15),
            Duration::from_millis(80),
            &["*"],
        ),
        Err(AppearanceCandidateError::ActivityRepaintIntervalTooFast {
            minimum: Duration::from_millis(16),
            actual: Duration::from_millis(15),
        })
    );
    assert_eq!(
        base.clone().with_activity_motion_for_test(
            Duration::from_millis(16),
            Duration::ZERO,
            &["*"],
        ),
        Err(AppearanceCandidateError::ZeroActivityMarkerInterval)
    );
    assert_eq!(
        base.clone().with_activity_motion_for_test(
            Duration::from_millis(20),
            Duration::from_millis(18),
            &["*"],
        ),
        Err(AppearanceCandidateError::ActivityMarkerIntervalTooFast {
            minimum: Duration::from_millis(20),
            actual: Duration::from_millis(18),
        })
    );
    assert_eq!(
        base.clone().with_activity_motion_for_test(
            Duration::from_millis(16),
            Duration::from_millis(80),
            &["\u{1b}"],
        ),
        Err(AppearanceCandidateError::ActivityMarkerFrameContainsControl { frame_index: 0 })
    );
    assert_eq!(
        base.clone().with_activity_motion_for_test(
            Duration::from_millis(16),
            Duration::from_millis(80),
            &["\u{0301}"],
        ),
        Err(AppearanceCandidateError::InvalidActivityMarkerGrapheme {
            frame_index: 0,
            grapheme_index: 0,
            cause: GraphemeError::ZeroWidth,
        })
    );
    let too_wide = "a".repeat(usize::from(u16::MAX) + 1);
    assert_eq!(
        base.clone().with_activity_motion_for_test(
            Duration::from_millis(16),
            Duration::from_millis(80),
            &[&too_wide],
        ),
        Err(AppearanceCandidateError::ActivityMarkerWidthOverflow { frame_index: 0 })
    );
    assert_eq!(
        base.with_activity_sweep_period_for_test(Duration::ZERO),
        Err(AppearanceCandidateError::ZeroActivitySweepPeriod)
    );
}
