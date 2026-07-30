use super::{
    AppearanceCandidate, AppearanceCandidateError, AppearanceCommitError, AppearanceGlyphRole,
    AppearanceState, GlyphProfile, validate_marker,
};
use crate::surface::GraphemeError;

// 기본 appearance는 Rich 글리프와 첫 revision을 같은 검증 경로로 확정한다.
#[test]
fn default_state_starts_with_a_valid_rich_snapshot() {
    let state = AppearanceState::default();
    let pin = state.pin();

    assert_eq!(pin.revision().get(), 1);
    assert_eq!(pin.snapshot().transcript_config().user_marker(), "❯");
    assert_eq!(pin.snapshot().transcript_config().assistant_marker(), "⏺");
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
