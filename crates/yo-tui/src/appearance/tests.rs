use super::{
    ActivityStyles, AppearanceCandidate, AppearanceCandidateError, AppearanceCommitError,
    AppearanceGlyphRole, AppearanceState, GlyphProfile, validate_marker,
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

// Rich와 ASCII의 내장 cycle은 120ms 논리 tick마다 승인된 순서의 한 grapheme을 선택한다.
#[test]
fn built_in_activity_cycles_follow_the_approved_order_and_period() {
    let rich = AppearanceState::default().pin();
    let expected_rich = ["·", "✢", "✳", "✶", "✻", "✽", "✽", "✻", "✶", "✳", "✢", "·"];
    for (tick, expected) in expected_rich.into_iter().enumerate() {
        let frame = rich
            .snapshot()
            .activity_motion_frame(Duration::from_millis(u64::try_from(tick).unwrap() * 120));
        assert_eq!(frame.marker(), expected);
        assert_eq!(frame.period(), Some(Duration::from_millis(120)));
        let sheen = frame.sheen(7).unwrap();
        let roles = ActivityStyles {
            marker: Style::default(),
            muted: Style::new(Color::Default, Color::Default, Attributes::DIM),
            trail: Style::new(Color::Indexed(6), Color::Default, Attributes::DIM),
            peak: Style::new(Color::Indexed(6), Color::Default, Attributes::empty()),
        };
        assert_eq!(sheen.style_at(tick % 7, roles), roles.peak);
    }

    let ascii = AppearanceState::new(AppearanceCandidate::for_profile(GlyphProfile::Ascii))
        .unwrap()
        .pin();
    assert_eq!(
        ascii
            .snapshot()
            .activity_motion_frame(Duration::ZERO)
            .marker(),
        "."
    );
    assert_eq!(
        ascii
            .snapshot()
            .activity_motion_frame(Duration::from_millis(120))
            .marker(),
        "*"
    );
}

// 한 frame 후보는 같은 검증·선택 경로를 쓰지만 period를 요구하지 않아 runner timer를 끈다.
#[test]
fn one_frame_activity_profile_is_valid_but_does_not_demand_motion() {
    let candidate = AppearanceCandidate::for_profile(GlyphProfile::Ascii)
        .with_activity_motion_for_test(Duration::from_millis(120), &["."])
        .unwrap();
    let state = AppearanceState::new(candidate).unwrap();
    let pin = state.pin();
    let frame = pin.snapshot().activity_motion_frame(Duration::from_secs(9));

    assert_eq!(frame.marker(), ".");
    assert_eq!(frame.period(), None);
    assert_eq!(frame.sheen(7), None);
}

// 움직이는 profile이어도 보이는 글자가 하나뿐이면 강조 위치가 달라질 수 없으므로
// style sheen은 timer demand를 만들지 않는다.
#[test]
fn activity_sheen_requires_at_least_two_visible_graphemes() {
    let state = AppearanceState::default();
    let pin = state.pin();
    let frame = pin.snapshot().activity_motion_frame(Duration::ZERO);

    assert_eq!(frame.sheen(0), None);
    assert_eq!(frame.sheen(1), None);
    assert!(frame.sheen(2).is_some());
}

// 3단계 sheen은 가운데 peak와 화면 안쪽의 이웃 한 칸만 trail로 칠하며,
// 첫·마지막 글자에서는 반대편으로 trail을 순환시키지 않는다.
#[test]
fn activity_sheen_clips_trails_at_visible_label_edges() {
    let pin = AppearanceState::default().pin();
    let styles = ActivityStyles {
        marker: Style::default(),
        muted: Style::new(Color::Default, Color::Default, Attributes::DIM),
        trail: Style::new(Color::Indexed(6), Color::Default, Attributes::DIM),
        peak: Style::new(Color::Indexed(6), Color::Default, Attributes::empty()),
    };

    let first = pin
        .snapshot()
        .activity_motion_frame(Duration::ZERO)
        .sheen(4)
        .unwrap();
    assert_eq!(first.style_at(0, styles), styles.peak);
    assert_eq!(first.style_at(1, styles), styles.trail);
    assert_eq!(first.style_at(3, styles), styles.muted);

    let last = pin
        .snapshot()
        .activity_motion_frame(Duration::from_millis(360))
        .sheen(4)
        .unwrap();
    assert_eq!(last.style_at(2, styles), styles.trail);
    assert_eq!(last.style_at(3, styles), styles.peak);
    assert_eq!(last.style_at(0, styles), styles.muted);
}

// 내장 activity 역할은 터미널 기본색 또는 팔레트 색만 사용하고, marker에는
// 글리프 실루엣을 뭉개는 bold·dim 속성을 붙이지 않는다.
#[test]
fn built_in_activity_styles_are_palette_based_and_marker_weight_is_stable() {
    let styles = AppearanceState::default()
        .pin()
        .snapshot()
        .styles()
        .chrome
        .activity;

    for style in [styles.marker, styles.muted, styles.trail, styles.peak] {
        assert!(matches!(
            style.foreground,
            Color::Default | Color::Indexed(_)
        ));
        assert!(matches!(
            style.background,
            Color::Default | Color::Indexed(_)
        ));
    }
    assert_eq!(styles.marker.attributes, Attributes::empty());
}

// 빈 cycle·0ms·복수 grapheme·서로 다른 cell 폭은 publication 전에 구체적 오류로 거부한다.
#[test]
fn invalid_activity_profiles_are_rejected_before_publication() {
    let base = AppearanceCandidate::for_profile(GlyphProfile::Rich);
    assert_eq!(
        base.clone()
            .with_activity_motion_for_test(Duration::from_millis(120), &[]),
        Err(AppearanceCandidateError::EmptyActivityFrames)
    );
    assert_eq!(
        base.clone()
            .with_activity_motion_for_test(Duration::ZERO, &["."]),
        Err(AppearanceCandidateError::ZeroActivityFramePeriod)
    );
    assert_eq!(
        base.clone()
            .with_activity_motion_for_test(Duration::from_millis(120), &["ab"]),
        Err(AppearanceCandidateError::ActivityFrameMustBeOneGrapheme { index: 0 })
    );
    assert_eq!(
        base.with_activity_motion_for_test(Duration::from_millis(120), &[".", "한"]),
        Err(AppearanceCandidateError::UnequalActivityFrameWidth {
            index: 1,
            expected: 1,
            actual: 2,
        })
    );
}
use std::time::Duration;
