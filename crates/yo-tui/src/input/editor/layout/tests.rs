use std::num::NonZeroU16;

use super::{LayoutError, layout_text};
use crate::{
    input::editor::PromptEditor,
    surface::{GraphemeError, Point},
};

fn width(value: u16) -> NonZeroU16 {
    NonZeroU16::new(value).unwrap()
}

fn positions(layout: &super::TextLayout) -> Vec<(&str, Point)> {
    layout
        .glyphs
        .iter()
        .map(|glyph| (glyph.grapheme.as_str(), glyph.point))
        .collect()
}

// ASCII와 넓은 한글은 고정 폭을 채우고 끝 커서는 다음 물리 행으로 이동한다.
#[test]
fn lays_out_mixed_width_text_and_visible_end_cursor() {
    let layout = layout_text("A가B", "A가B".len(), width(4)).unwrap();

    assert_eq!(
        positions(&layout),
        [
            ("A", Point::new(0, 0)),
            ("가", Point::new(1, 0)),
            ("B", Point::new(3, 0))
        ]
    );
    assert_eq!(layout.cursor, Point::new(0, 1));
    assert_eq!(layout.height.get(), 2);
}

// 남은 칸보다 넓은 grapheme은 쪼개지 않고 다음 행에서 시작한다.
#[test]
fn soft_wraps_before_a_complete_wide_grapheme() {
    let layout = layout_text("A가B", "A가B".len(), width(2)).unwrap();

    assert_eq!(
        positions(&layout),
        [
            ("A", Point::new(0, 0)),
            ("가", Point::new(0, 1)),
            ("B", Point::new(0, 2))
        ]
    );
    assert_eq!(layout.cursor, Point::new(1, 2));
    assert_eq!(layout.height.get(), 3);
}

// LF, CRLF와 단독 CR은 glyph 없이 각각 새 행을 시작한다.
#[test]
fn hard_breaks_reset_the_physical_column() {
    let text = "A\nB\r\nC\rD";
    let layout = layout_text(text, text.len(), width(4)).unwrap();

    assert_eq!(
        positions(&layout),
        [
            ("A", Point::new(0, 0)),
            ("B", Point::new(0, 1)),
            ("C", Point::new(0, 2)),
            ("D", Point::new(0, 3))
        ]
    );
    assert_eq!(layout.cursor, Point::new(1, 3));
    assert_eq!(layout.height.get(), 4);
}

// 커서 뒤의 넓은 grapheme이 줄바꿈되면 커서도 그 grapheme의 시작 위치에 놓인다.
#[test]
fn cursor_precedes_the_next_grapheme_after_soft_wrap() {
    let layout = layout_text("A가", "A".len(), width(2)).unwrap();

    assert_eq!(layout.cursor, Point::new(0, 1));
    assert_eq!(
        positions(&layout),
        [("A", Point::new(0, 0)), ("가", Point::new(0, 1))]
    );
}

// 커서가 앞에 있어도 뒤쪽 text가 차지하는 모든 행을 layout 높이에 포함한다.
#[test]
fn height_includes_content_after_the_cursor() {
    let layout = layout_text("A가B", 0, width(2)).unwrap();

    assert_eq!(layout.cursor, Point::new(0, 0));
    assert_eq!(layout.height.get(), 3);
}

// 빈 문자열도 첫 행의 첫 칸에 표시 가능한 커서를 가진다.
#[test]
fn empty_text_has_one_cursor_row() {
    let layout = PromptEditor::new().layout(width(8)).unwrap();

    assert!(layout.glyphs.is_empty());
    assert_eq!(layout.cursor, Point::new(0, 0));
    assert_eq!(layout.height.get(), 1);
}

// viewport보다 넓은 grapheme은 분할하거나 유실하지 않고 정확한 위치로 실패한다.
#[test]
fn rejects_a_grapheme_wider_than_the_viewport() {
    assert_eq!(
        layout_text("가", "가".len(), width(1)),
        Err(LayoutError::GraphemeTooWide {
            byte_index: 0,
            width: width(2)
        })
    );
}

// tab과 독립 결합 문자처럼 표시 정책이 없는 입력은 byte 위치와 원인을 보존한다.
#[test]
fn reports_unrenderable_input_without_partial_layout() {
    assert_eq!(
        layout_text("A\tB", "A\tB".len(), width(8)),
        Err(LayoutError::UnrenderableGrapheme {
            byte_index: 1,
            cause: GraphemeError::Control
        })
    );
    assert_eq!(
        layout_text("\u{301}", "\u{301}".len(), width(8)),
        Err(LayoutError::UnrenderableGrapheme {
            byte_index: 0,
            cause: GraphemeError::ZeroWidth
        })
    );
}

// 외부에서 잘못 전달한 byte 커서는 범위와 grapheme 내부 위치를 구분해 거절한다.
#[test]
fn validates_external_cursor_positions() {
    assert_eq!(
        layout_text("가", 4, width(8)),
        Err(LayoutError::CursorOutOfBounds)
    );
    assert_eq!(
        layout_text("가", 1, width(8)),
        Err(LayoutError::CursorNotOnGraphemeBoundary)
    );
}

// u16 최대 행의 다음 높이는 표현할 수 없으므로 최종 계산에서 구조화 오류가 된다.
#[test]
fn rejects_final_height_beyond_u16() {
    let text = "\n".repeat(usize::from(u16::MAX));

    assert_eq!(
        layout_text(&text, text.len(), width(1)),
        Err(LayoutError::HeightOverflow)
    );
}

// u16 범위를 넘는 hard break는 스캔 중에도 overflow 없이 구조화 오류가 된다.
#[test]
fn rejects_hard_break_increment_beyond_u16() {
    let text = "\n".repeat(usize::from(u16::MAX) + 1);

    assert_eq!(
        layout_text(&text, text.len(), width(1)),
        Err(LayoutError::HeightOverflow)
    );
}
