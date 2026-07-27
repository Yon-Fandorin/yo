use super::{
    TranscriptLayoutConfig, TranscriptRenderError, TranscriptState, TranscriptViewState,
    assert_failure_preserves_surface_and_state, id, render, styles,
};
use crate::{
    surface::{Grapheme, Point, Rect, Size, Surface, WriteOutcome},
    transcript::{MessageRole, TranscriptLayoutConfigError},
};

// 여러 줄 marker는 설정 검증에서 거절하고 기존 화면과 스크롤 위치를 보존한다.
#[test]
fn control_in_marker_preserves_surface_and_view_state() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "hello".into())
        .expect("unique user item");
    let config = TranscriptLayoutConfig::default()
        .with_user_marker("U\n")
        .with_assistant_marker("A");

    assert_failure_preserves_surface_and_state(
        &transcript,
        Size::new(8, 2),
        &config,
        TranscriptRenderError::InvalidConfig(TranscriptLayoutConfigError::MarkerContainsControl {
            role: MessageRole::User,
        }),
    );
}

// marker가 본문 시작 열을 침범하면 암묵적으로 wrap하지 않고 구조적 설정 오류를 반환한다.
#[test]
fn marker_wider_than_indent_preserves_surface_and_view_state() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "hello".into())
        .expect("unique user item");
    let config = TranscriptLayoutConfig::default()
        .with_body_indent(1)
        .with_user_marker("USER")
        .with_assistant_marker("A");

    assert_failure_preserves_surface_and_state(
        &transcript,
        Size::new(8, 2),
        &config,
        TranscriptRenderError::InvalidConfig(TranscriptLayoutConfigError::MarkerWiderThanIndent {
            role: MessageRole::User,
            marker_width: 4,
            body_indent: 1,
        }),
    );
}

// marker 폭 검증은 글자 수가 아니라 CJK가 차지하는 실제 2개 terminal cell을 사용한다.
#[test]
fn wide_marker_uses_terminal_cell_width() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "hello".into())
        .expect("unique user item");
    let config = TranscriptLayoutConfig::default()
        .with_body_indent(1)
        .with_user_marker("가");

    assert_failure_preserves_surface_and_state(
        &transcript,
        Size::new(8, 2),
        &config,
        TranscriptRenderError::InvalidConfig(TranscriptLayoutConfigError::MarkerWiderThanIndent {
            role: MessageRole::User,
            marker_width: 2,
            body_indent: 1,
        }),
    );
}

// 빈 Final 항목의 marker도 view 폭을 넘으면 clear 전에 거절해 기존 Surface를 보존한다.
#[test]
fn empty_final_marker_wider_than_view_is_atomic() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), String::new())
        .expect("unique user item");
    let config = TranscriptLayoutConfig::default()
        .with_body_indent(2)
        .with_user_marker("가");
    let size = Size::new(1, 1);
    let mut surface = Surface::new(size).unwrap();
    {
        let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
        assert_eq!(
            view.write(
                Point::new(0, 0),
                Grapheme::try_from("X").unwrap(),
                styles().user_body
            ),
            WriteOutcome::Written
        );
    }
    let before_surface = surface.clone();
    let mut state = TranscriptViewState::default();
    let before_state = state;

    let error = {
        let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
        render(&transcript, &mut view, &config, styles(), &mut state, None).unwrap_err()
    };

    assert_eq!(
        error,
        TranscriptRenderError::InvalidConfig(TranscriptLayoutConfigError::MarkerWiderThanView {
            role: MessageRole::User,
            marker_width: 2,
            view_width: 1,
        })
    );
    assert_eq!(surface, before_surface);
    assert_eq!(state, before_state);
}
