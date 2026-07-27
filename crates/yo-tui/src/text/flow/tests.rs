use std::num::NonZeroU16;

use super::{TextFlowError, flow_text, flow_text_with_cursor};
use crate::surface::Point;

fn width(value: u16) -> NonZeroU16 {
    NonZeroU16::new(value).unwrap()
}

fn positions(flow: &super::TextFlow) -> Vec<(&str, Point)> {
    flow.glyphs
        .iter()
        .map(|glyph| (glyph.grapheme.as_str(), glyph.point))
        .collect()
}

// cursor 없는 빈 text는 transcript에서 불필요한 빈 행을 소유하지 않는다.
#[test]
fn empty_text_has_zero_content_height_without_cursor() {
    let flow = flow_text("", width(8)).unwrap();

    assert!(flow.glyphs.is_empty());
    assert_eq!(flow.height, 0);
}

// 폭을 정확히 채운 text는 prompt 끝 커서용 다음 행 없이 실제 glyph 한 행만 차지한다.
#[test]
fn full_row_has_no_cursor_only_trailing_row() {
    let flow = flow_text("ABCD", width(4)).unwrap();

    assert_eq!(flow.height, 1);
    assert_eq!(
        positions(&flow),
        [
            ("A", Point::new(0, 0)),
            ("B", Point::new(1, 0)),
            ("C", Point::new(2, 0)),
            ("D", Point::new(3, 0)),
        ]
    );
}

// cursor 없는 exact full row는 u16 최대 높이까지 허용하고 다음 물리 행부터 overflow다.
#[test]
fn cursor_free_full_row_uses_the_complete_u16_height_range() {
    let mut exact = "\n".repeat(usize::from(u16::MAX) - 1);
    exact.push('A');

    let flow = flow_text(&exact, width(1)).unwrap();

    assert_eq!(flow.height, u16::MAX);
    assert_eq!(
        flow.glyphs.last().unwrap().point,
        Point::new(0, u16::MAX - 1)
    );

    let mut wrapped = exact.clone();
    wrapped.push('B');
    assert_eq!(
        flow_text(&wrapped, width(1)),
        Err(TextFlowError::HeightOverflow)
    );

    let mut hard_break = exact;
    hard_break.push('\n');
    assert_eq!(
        flow_text(&hard_break, width(1)),
        Err(TextFlowError::HeightOverflow)
    );
}

// 같은 text에 끝 커서를 요청하면 cursor가 보일 다음 행까지 높이에 포함한다.
#[test]
fn cursor_adapter_adds_a_visible_trailing_row() {
    let flow = flow_text_with_cursor("ABCD", "ABCD".len(), width(4)).unwrap();

    assert_eq!(flow.cursor, Point::new(0, 1));
    assert_eq!(flow.height.get(), 2);
}

// trailing hard break는 glyph가 없어도 의미 있는 마지막 빈 행을 높이에 포함한다.
#[test]
fn trailing_hard_break_preserves_the_empty_line() {
    let flow = flow_text("A\n", width(4)).unwrap();

    assert_eq!(flow.height, 2);
    assert_eq!(flow.glyphs.len(), 1);
}

// cursor 없는 경로도 prompt와 같은 grapheme 폭·control 표시 정책을 사용한다.
#[test]
fn shared_flow_wraps_wide_and_control_text_safely() {
    let flow = flow_text("A가\u{3}", width(2)).unwrap();

    assert_eq!(
        positions(&flow),
        [
            ("A", Point::new(0, 0)),
            ("가", Point::new(0, 1)),
            ("^", Point::new(0, 2)),
            ("C", Point::new(1, 2)),
        ]
    );
    assert_eq!(flow.height, 3);
}

// cursor 없는 경로도 표시할 수 없는 grapheme의 byte 위치와 원인을 보존한다.
#[test]
fn shared_flow_preserves_unrenderable_grapheme_error() {
    assert_eq!(
        flow_text("\u{301}", width(4)),
        Err(TextFlowError::UnrenderableGrapheme {
            byte_index: 0,
            cause: crate::surface::GraphemeError::ZeroWidth,
        })
    );
}
