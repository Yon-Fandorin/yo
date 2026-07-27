use super::{TranscriptLayoutConfig, TranscriptState, TranscriptViewState, id, render_into};
use crate::{
    surface::{GraphemeError, Size},
    text::flow::TextFlowError,
    transcript::{TranscriptMeasure, TranscriptMeasureError, measure},
};

// 측정 높이는 같은 폭과 설정으로 실제 렌더가 보고하는 전체 content 높이와 일치한다.
#[test]
fn measurement_matches_rendered_content_height() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "abcdef".into())
        .expect("unique user item");
    let config = TranscriptLayoutConfig::default();
    let mut state = TranscriptViewState::default();

    let measurement = measure(&transcript, 5, &config).unwrap();
    let (_, frame) = render_into(&transcript, Size::new(5, 2), &config, &mut state, None);

    assert_eq!(
        measurement,
        TranscriptMeasure {
            content_height: frame.content_height,
        }
    );
}

// 측정은 Surface 없이도 zero-width와 본문 layout 실패를 동일한 구조적 오류로 보고한다.
#[test]
fn measurement_reports_preflight_failures_without_a_surface() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "\u{301}".into())
        .expect("unique user item");

    assert_eq!(
        measure(&transcript, 0, &TranscriptLayoutConfig::default()),
        Err(TranscriptMeasureError::ZeroWidth)
    );
    assert_eq!(
        measure(&transcript, 8, &TranscriptLayoutConfig::default()),
        Err(TranscriptMeasureError::Text(
            TextFlowError::UnrenderableGrapheme {
                byte_index: 0,
                cause: GraphemeError::ZeroWidth,
            }
        ))
    );
}
