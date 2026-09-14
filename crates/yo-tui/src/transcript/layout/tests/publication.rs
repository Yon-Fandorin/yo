use super::id;
use crate::transcript::{TranscriptLayoutConfig, TranscriptState};

// 같은 너비로 준비한 persistent prefix와 unpublished suffix는 경계 separator를 한쪽에서
// 잃거나 중복하지 않고 전체 transcript의 plain projection을 정확히 재구성한다.
#[test]
fn adjacent_slices_reconstruct_the_complete_transcript_projection() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "question".to_owned())
        .expect("the first item ID is unique");
    transcript
        .start_assistant(id(2))
        .expect("the second item ID is unique");
    transcript
        .append_text(id(2), "answer")
        .expect("the assistant item is streaming");
    let config = TranscriptLayoutConfig::default();

    let complete = transcript.plain_output(&config).unwrap().unwrap();
    let persistent = transcript
        .plain_output_slice(transcript.slice(0..1), &config)
        .unwrap()
        .unwrap();
    let live = transcript
        .plain_output_slice(transcript.suffix(1), &config)
        .unwrap()
        .unwrap();

    assert_eq!(format!("{persistent}{live}"), complete);
    assert_eq!(persistent, "❯ question\n");
    assert_eq!(live, "\n• answer\n");
}

// 두 사용자 항목 사이의 두 행 separator도 suffix가 이전 visible 항목의 존재를 기억해
// publication cursor가 항목 사이를 가를 때 전체 레이아웃과 동일하게 유지된다.
#[test]
fn user_suffix_preserves_the_two_row_turn_separator() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "first".to_owned())
        .expect("the first item ID is unique");
    transcript
        .push_user(id(2), "second".to_owned())
        .expect("the second item ID is unique");
    let config = TranscriptLayoutConfig::default();

    assert_eq!(
        transcript
            .plain_output_slice(transcript.suffix(1), &config)
            .unwrap()
            .unwrap(),
        "\n\n❯ second\n"
    );
}

// 작은 항목이 많이 쌓여도 마지막 화면의 행 인덱스 탐색은 이력 전체를 순회하지
// 않으며, 실제 paint에는 보이는 항목과 글자만 들어오는지 확인합니다.
#[test]
fn many_short_items_seek_only_the_visible_page() {
    use std::{cell::Cell, num::NonZeroU16};

    use super::{
        super::{VisibleRows, paint_indexed_commands, prepare, visible_row_entries},
        rendered_row, styles,
    };
    use crate::{
        surface::{Point, Rect, Size, Surface},
        transcript::TranscriptViewState,
    };

    let mut transcript = TranscriptState::new();
    for value in 1..=10_000 {
        transcript
            .push_user(id(value), format!("ITEM_{value}"))
            .unwrap();
    }
    let prepared = prepare(&transcript, 20, &TranscriptLayoutConfig::default()).unwrap();
    let mut state = TranscriptViewState::default();
    let visible = VisibleRows::resolve_commands(
        prepared.content_height(),
        NonZeroU16::new(12).unwrap(),
        state,
        &[],
        &[],
    );
    let visits = Cell::new(0);
    let glyphs = visible_row_entries(&prepared.layout.glyphs, visible, |entry| {
        visits.set(visits.get() + 1);
        entry.0
    });
    assert!(
        visits.get() < 64,
        "row lookup must be logarithmic in item count"
    );
    assert!(glyphs.len() <= 12 * 2);
    assert!(prepared.layout.visible_items(visible).len() <= 12);
    let size = Size::new(20, 12);
    let mut surface = Surface::new(size).unwrap();
    let mut view = surface.view(Rect::new(Point::new(0, 0), size)).unwrap();
    paint_indexed_commands(&prepared, &mut view, styles(), &mut state, &[]).unwrap();
    assert!((0..12).any(|row| rendered_row(&surface, row).contains("ITEM_10000")));
}
