use std::num::NonZeroU16;

use super::{
    TranscriptLayoutConfig, TranscriptRenderError, TranscriptScrollCommand, TranscriptState,
    TranscriptViewState, assert_failure_preserves_surface_and_state, id, render, styles,
};
use crate::{
    surface::{Grapheme, GraphemeError, Point, Rect, Size, Surface, WriteOutcome},
    text::flow::TextFlowError,
    transcript::{TranscriptPaintError, paint_prepared, prepare},
};

// view 폭 전체가 들여쓰기에 소비되면 본문을 잃지 않고 렌더를 원자적으로 거절한다.
#[test]
fn unavailable_body_width_preserves_surface_and_view_state() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "hello".into())
        .expect("unique user item");
    let config = TranscriptLayoutConfig::default()
        .with_body_indent(8)
        .with_user_marker("U")
        .with_assistant_marker("A");

    assert_failure_preserves_surface_and_state(
        &transcript,
        Size::new(8, 2),
        &config,
        TranscriptRenderError::BodyWidthUnavailable,
    );
}

// 본문의 표시 불가능한 grapheme 오류도 기존 화면과 nonzero scroll offset을 보존한다.
#[test]
fn text_failure_preserves_surface_and_view_state() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "\u{301}".into())
        .expect("unique user item");

    assert_failure_preserves_surface_and_state(
        &transcript,
        Size::new(8, 2),
        &TranscriptLayoutConfig::default(),
        TranscriptRenderError::Text(TextFlowError::UnrenderableGrapheme {
            byte_index: 0,
            cause: GraphemeError::ZeroWidth,
        }),
    );
}

// 여러 item의 separator와 본문 높이 합이 u16을 넘으면 wrapping하지 않고 원자적으로 실패한다.
#[test]
fn combined_height_overflow_preserves_surface_and_view_state() {
    let tall = "a\n".repeat(32_767);
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), tall.clone())
        .expect("unique user item");
    transcript.push_user(id(2), tall).expect("unique user item");

    assert_failure_preserves_surface_and_state(
        &transcript,
        Size::new(8, 2),
        &TranscriptLayoutConfig::default(),
        TranscriptRenderError::HeightOverflow,
    );
}

// 폭이나 높이가 0인 view는 layout과 Surface 변경을 시작하지 않는다.
#[test]
fn empty_view_dimensions_are_rejected_before_mutation() {
    let transcript = TranscriptState::new();
    for (size, expected) in [
        (Size::new(0, 1), TranscriptRenderError::ZeroWidth),
        (Size::new(1, 0), TranscriptRenderError::ZeroHeight),
    ] {
        let mut surface = Surface::new(size).unwrap();
        let before = surface.clone();
        let mut state = TranscriptViewState::default();
        let error = {
            let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
            render(
                &transcript,
                &mut view,
                &TranscriptLayoutConfig::default(),
                styles(),
                &mut state,
                None,
            )
            .unwrap_err()
        };

        assert_eq!(error, expected);
        assert_eq!(surface, before);
        assert_eq!(state, TranscriptViewState::default());
    }
}

// view 경계를 가로지르는 기존 wide footprint는 clear 단계에서 원자적으로 거절한다.
#[test]
fn crossing_surface_footprint_preserves_surface_and_view_state() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "hello".into())
        .expect("unique user item");
    let mut surface = Surface::new(Size::new(4, 2)).unwrap();
    {
        let mut full = surface
            .view(Rect::new(Point::new(0, 0), Size::new(4, 2)))
            .unwrap();
        assert_eq!(
            full.write(
                Point::new(0, 0),
                Grapheme::try_from("가").unwrap(),
                styles().user_body
            ),
            WriteOutcome::Written
        );
    }
    let before_surface = surface.clone();
    let mut state = TranscriptViewState::default();
    let mut seed = TranscriptState::new();
    seed.push_user(id(99), "0\n1\n2\n3".into())
        .expect("unique seed item");
    {
        let mut scratch = Surface::new(Size::new(4, 2)).unwrap();
        let mut view = scratch
            .view(Rect::new(Point::new(0, 0), Size::new(4, 2)))
            .unwrap();
        let frame = render(
            &seed,
            &mut view,
            &TranscriptLayoutConfig::default(),
            styles(),
            &mut state,
            Some(TranscriptScrollCommand::LineUp),
        )
        .unwrap();
        assert!(frame.first_visible_row > 0);
    }
    let before_state = state;

    let error = {
        let mut view = surface
            .view(Rect::new(Point::new(1, 0), Size::new(2, 2)))
            .unwrap();
        render(
            &transcript,
            &mut view,
            &TranscriptLayoutConfig::default()
                .with_max_body_width(NonZeroU16::new(1))
                .with_body_indent(1)
                .with_user_marker("U")
                .with_assistant_marker("A"),
            styles(),
            &mut state,
            None,
        )
        .unwrap_err()
    };

    assert_eq!(error, TranscriptRenderError::SurfaceConflict);
    assert_eq!(surface, before_surface);
    assert_eq!(state, before_state);
}

// 준비 폭과 다른 view는 glyph 쓰기 전에 거절되어 Surface와 viewport 상태를 보존한다.
#[test]
fn prepared_width_mismatch_is_rejected_before_painting() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "x".into())
        .expect("unique user item");
    let prepared = prepare(&transcript, 3, &TranscriptLayoutConfig::default()).unwrap();
    let mut state = TranscriptViewState::default();
    let mut surface = Surface::new(Size::new(4, 1)).unwrap();
    let before_surface = surface.clone();
    let before_state = state;

    let error = {
        let mut view = surface
            .view(Rect::new(Point::new(0, 0), Size::new(4, 1)))
            .unwrap();
        paint_prepared(prepared, &mut view, styles(), &mut state, None).unwrap_err()
    };

    assert_eq!(
        error,
        TranscriptPaintError::WidthMismatch {
            prepared: 3,
            actual: 4,
        }
    );
    assert_eq!(surface, before_surface);
    assert_eq!(state, before_state);
}
