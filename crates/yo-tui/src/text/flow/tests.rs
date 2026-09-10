use std::num::NonZeroU16;

use super::{TextFlowError, flow_literal_prose, flow_tail_text, flow_text, flow_text_with_cursor};
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

// tail 보관은 기존 줄바꿈·탭·제어 문자·한글·grapheme/source offset 규칙을 그대로 따른다.
#[test]
fn retained_tail_matches_complete_flow_cells_and_source_offsets() {
    for source in [
        "",
        "A\r\n\r\nB\n",
        "abcd\tZ",
        "한글 e\u{301} 👩‍💻\n\u{1b}[31m\nlast\n",
        "1234567890",
    ] {
        for columns in [2, 5, 12] {
            let full = flow_text(source, width(columns)).unwrap();
            for rows in [1, 3, u16::MAX] {
                let tail = flow_tail_text(source, width(columns), width(rows)).unwrap();
                let skipped = full.height.saturating_sub(rows);
                let expected = full
                    .glyphs
                    .iter()
                    .filter(|glyph| glyph.point.y >= skipped)
                    .cloned()
                    .map(|mut glyph| {
                        glyph.point.y -= skipped;
                        glyph
                    })
                    .collect::<Vec<_>>();
                assert_eq!(tail.skipped_rows, usize::from(skipped));
                assert_eq!(tail.flow.height, full.height.min(rows));
                assert_eq!(
                    tail.flow.glyphs, expected,
                    "source {source:?}, columns {columns}, rows {rows}"
                );
            }
        }
    }
}

// 일반 배치의 최대 높이와 첫 초과를 넘는 입력도 tail은 필요한 행만 보관하고 정확히 센다.
#[test]
fn retained_tail_crosses_full_layout_height_without_overflow() {
    for breaks in [usize::from(u16::MAX) - 1, usize::from(u16::MAX), 100_000] {
        let source = format!("{}last", "x\n".repeat(breaks));
        let tail = flow_tail_text(&source, width(8), width(5)).unwrap();
        assert_eq!(tail.skipped_rows, breaks + 1 - 5);
        assert_eq!(tail.flow.height, 5);
        assert_eq!(tail.flow.glyphs.len(), 8);
        assert_eq!(tail.flow.glyphs.last().unwrap().point, Point::new(3, 4));
        assert_eq!(
            tail.flow.glyphs.last().unwrap().byte_index,
            source.len() - 1
        );
        assert_eq!(
            flow_text(&source, width(8)).is_err(),
            breaks >= usize::from(u16::MAX)
        );
    }
    let blanks = flow_tail_text(&"\n".repeat(100_000), width(8), width(3)).unwrap();
    assert_eq!(blanks.skipped_rows, 99_998);
    assert_eq!(blanks.flow.height, 3);
    assert!(blanks.flow.glyphs.is_empty());
}

// 매우 긴 한 줄도 마지막 한글을 자르지 않고 전체 배치보다 큰 표시 행 수를 처리한다.
#[test]
fn retained_tail_wraps_a_line_beyond_full_layout_capacity() {
    let source = format!("{}끝", "x".repeat(2 * usize::from(u16::MAX)));
    let tail = flow_tail_text(&source, width(2), width(5)).unwrap();
    assert_eq!(tail.skipped_rows, usize::from(u16::MAX) + 1 - 5);
    assert_eq!(tail.flow.glyphs.len(), 9);
    let last = tail.flow.glyphs.last().unwrap();
    assert_eq!(last.grapheme.as_str(), "끝");
    assert_eq!(last.point, Point::new(0, 4));
    assert_eq!(last.byte_index, 2 * usize::from(u16::MAX));
    assert_eq!(
        flow_text(&source, width(2)),
        Err(TextFlowError::HeightOverflow)
    );
}

// 페이지 경계에서도 탭·제어 문자·넓은 문자와 마지막 빈 행의 실제 셀 배치를 유지한다.
#[test]
fn pages_match_full_flow_at_every_window() {
    for source in [
        "",
        "A\r\n\r\nB\n",
        "abcd\tZ",
        "한글 e\u{301} 👩‍💻\n\u{1b}[31m\nlast\n",
        "1234567890",
    ] {
        for columns in [2, 5, 12] {
            let full = flow_text(source, width(columns)).unwrap();
            let pages = super::TextPages::new(source, width(columns)).unwrap();
            assert_eq!(pages.row_count(), usize::from(full.height));
            for start in 0..=usize::from(full.height) {
                for rows in [1, 3, 7] {
                    let page = flow_text(pages.window(start, width(rows)), width(columns)).unwrap();
                    let expected = full
                        .glyphs
                        .iter()
                        .filter(|glyph| {
                            let row = usize::from(glyph.point.y);
                            row >= start && row < start + usize::from(rows)
                        })
                        .map(|glyph| {
                            (
                                glyph.grapheme.as_str(),
                                Point::new(
                                    glyph.point.x,
                                    (usize::from(glyph.point.y) - start) as u16,
                                ),
                            )
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(
                        positions(&page),
                        expected,
                        "source {source:?}, width {columns}, start {start}, rows {rows}"
                    );
                    assert!(page.height <= rows);
                }
            }
        }
    }
}

// 희소 인덱스의 양쪽과 u16 높이 초과 지점도 임의 접근하며 끝의 빈 행을 보존한다.
#[test]
fn pages_seek_across_checkpoints_and_surface_height_limit() {
    let source = (0..70_000)
        .map(|index| format!("row {index}\n"))
        .collect::<String>();
    let pages = super::TextPages::new(&source, width(24)).unwrap();
    assert_eq!(pages.row_count(), 70_001);
    for start in [0, 127, 128, 129, 65_534, 65_535, 65_536, 69_999] {
        assert_eq!(pages.window(start, width(1)), format!("row {start}"));
    }
    assert_eq!(pages.window(69_999, width(2)), "row 69999\n");
    assert_eq!(pages.window(70_000, width(1)), "");
    assert_eq!(pages.window(usize::MAX, width(1)), "");
    let line = format!("{}끝", "x".repeat(2 * usize::from(u16::MAX)));
    let pages = super::TextPages::new(&line, width(2)).unwrap();
    assert_eq!(pages.row_count(), usize::from(u16::MAX) + 1);
    assert_eq!(pages.window(usize::from(u16::MAX), width(1)), "끝");
}

// 원문 위치는 개행 재배치·빈 CRLF 행·탭 확장에서도 유효한 행으로 복원하며 u16 높이에 제한받지
// 않는다.
#[test]
fn pages_resolve_original_offsets_across_reflow_and_empty_rows() {
    let source = "한글 e\u{301} abcdefghijklmnop\r\n\r\n\tZ\n";
    for columns in [2, 5, 24, 80] {
        let pages = super::TextPages::new(source, width(columns)).unwrap();
        let full = flow_text(source, width(columns)).unwrap();
        for glyph in &full.glyphs {
            let row = pages.row_for_source(glyph.byte_index);
            assert!(row <= usize::from(glyph.point.y));
            assert!(full.glyphs.iter().any(|first| {
                usize::from(first.point.y) == row && first.byte_index == glyph.byte_index
            }));
        }
        let empty = source.find("\r\n\r\n").unwrap() + 2;
        let row = pages.row_for_source(empty);
        assert_eq!(pages.source_offset(row), empty);
        assert_eq!(pages.window(row, width(1)), "");
        assert_eq!(pages.source_offset(pages.row_count() - 1), source.len());
    }
    let source = "line\n".repeat(70_000);
    let pages = super::TextPages::new(&source, width(24)).unwrap();
    assert_eq!(pages.row_for_source(65_536 * 5), 65_536);
    assert_eq!(pages.source_offset(65_536), 65_536 * 5);
    assert_eq!(pages.row_for_source(usize::MAX), 70_000);
}

// 읽기용 문장은 단어를 보존하되 명시적 들여쓰기·빈 줄과 원래 UTF-8 위치를 유지한다.
#[test]
fn literal_prose_preserves_words_indentation_blank_lines_and_source_offsets() {
    let text = "  alpha beta\n\n  γδ 🙂 é";
    let flow = flow_literal_prose(text, width(10)).unwrap();
    let rows: Vec<String> = (0..flow.height)
        .map(|y| {
            flow.glyphs
                .iter()
                .filter(|glyph| glyph.point.y == y)
                .map(|glyph| glyph.grapheme.as_str())
                .collect()
        })
        .collect();
    assert_eq!(rows, ["  alpha", "beta", "", "  γδ 🙂 é"]);
    for glyph in &flow.glyphs {
        assert!(text[glyph.byte_index..].starts_with(glyph.grapheme.as_str()));
        assert!(glyph.point.x + glyph.grapheme.width().get() <= 10);
    }
    let indented = flow_literal_prose("    word", width(6)).unwrap();
    assert_eq!(indented.glyphs[4].point, Point::new(4, 0));
    assert_eq!(indented.glyphs[6].point, Point::new(0, 1));
    assert_eq!(indented.height, 2);
    let cjk = flow_literal_prose("abc 한글", width(5)).unwrap();
    assert_eq!(
        cjk.glyphs
            .iter()
            .find(|glyph| glyph.grapheme.as_str() == "한")
            .unwrap()
            .point,
        Point::new(0, 1)
    );
    assert_eq!(
        flow_literal_prose("header\n🙂", width(1)).unwrap_err(),
        TextFlowError::GraphemeTooWide {
            byte_index: 7,
            width: width(2)
        }
    );
}

// 읽기용 줄바꿈을 추가해도 원문 flow와 편집기 커서의 기존 grapheme 배치는 바뀌지 않는다.
#[test]
fn prose_wrapping_does_not_change_literal_or_editor_coordinate_contracts() {
    let text = "alpha beta";
    let prose = flow_literal_prose(text, width(8)).unwrap();
    let literal = flow_text(text, width(8)).unwrap();
    let editor = flow_text_with_cursor(text, text.len(), width(8)).unwrap();
    assert_eq!(literal.glyphs, editor.glyphs);
    assert_eq!(editor.cursor, Point::new(2, 1));
    assert_eq!(
        literal
            .glyphs
            .iter()
            .find(|glyph| glyph.grapheme.as_str() == "b")
            .unwrap()
            .point,
        Point::new(6, 0)
    );
    assert_eq!(
        prose
            .glyphs
            .iter()
            .find(|glyph| glyph.grapheme.as_str() == "b")
            .unwrap()
            .point,
        Point::new(0, 1)
    );
}

// 이스케이프 페이지는 hard line과 빈 행을 유지하고 읽기 위치를 원문 byte에 대응한다.
#[test]
fn escaped_pages_preserve_hard_lines_and_original_offsets() {
    let source = "\u{301}\nnext\n\nlast";
    let pages = super::TextPages::with_escaped_fallback(source, width(80), "Escaped").unwrap();
    assert_eq!(pages.window(0, width(8)), "Escaped\n\\u{301}\nnext\n\nlast");
    assert_eq!(pages.source_offset(1), 0);
    assert_eq!(pages.source_offset(2), source.find("next").unwrap());
    assert_eq!(pages.source_offset(3), source.find("\n\n").unwrap() + 1);
    assert_eq!(pages.row_for_source(source.find("last").unwrap()), 4);
    let source = "first\n한글\nlast";
    let pages = super::TextPages::with_escaped_fallback(source, width(1), "Escaped").unwrap();
    let first = pages.row_for_source(source.find('한').unwrap());
    assert_eq!(pages.window(first, width(8)).replace('\n', ""), "\\u{d55c}");
    for row in first..first + 8 {
        assert_eq!(pages.source_offset(row), source.find('한').unwrap());
    }
    let literal = super::TextPages::with_escaped_fallback(source, width(24), "Escaped").unwrap();
    assert_eq!(literal.window(0, width(8)), source);
    assert_eq!(literal.row_for_source(pages.source_offset(first + 3)), 1);
}

// 코드 줄바꿈은 짧은 단어를 붙여 두고 공백·탭·유니코드와 원문 위치를 하나도 버리지 않는다.
#[test]
fn code_wrap_preserves_all_literal_glyphs_and_keeps_fitting_words_together() {
    let text = "  let theme = \"selected\";\n\n\t한글 é 🙂  \r\nend";
    for columns in [2, 8, 20, 80] {
        let code = super::flow_code(text, width(columns)).unwrap();
        let literal = flow_text(text, width(columns)).unwrap();
        let source_glyphs = |flow: &super::TextFlow| {
            flow.glyphs
                .iter()
                .map(|glyph| (glyph.byte_index, glyph.grapheme.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(source_glyphs(&code), source_glyphs(&literal));
        assert!(
            code.glyphs
                .iter()
                .all(|glyph| glyph.point.x + glyph.grapheme.width().get() <= columns)
        );
    }
    let code = super::flow_code("let theme = \"selected\";", width(20)).unwrap();
    let selected = code
        .glyphs
        .iter()
        .filter(|glyph| glyph.byte_index >= 12)
        .collect::<Vec<_>>();
    assert!(selected.iter().all(|glyph| glyph.point.y == 1));
    assert_eq!(selected[0].point.x, 0);
    assert_eq!(
        super::flow_code("abc\n🙂", width(1)).unwrap_err(),
        TextFlowError::GraphemeTooWide {
            byte_index: 4,
            width: width(2)
        }
    );
}

// 코드의 이어지는 줄은 제한된 원문 들여쓰기를 유지하고 모든 문자와 바이트 위치를 보존한다.
#[test]
fn code_continuations_retain_bounded_indentation_without_synthetic_source() {
    let source = "    return [value * 2 for value in values]\nnext";
    for columns in [2, 8, 16, 24, 80] {
        let code = super::flow_code(source, width(columns)).unwrap();
        let literal = flow_text(source, width(columns)).unwrap();
        assert_eq!(
            code.glyphs
                .iter()
                .map(|g| (g.byte_index, &g.grapheme))
                .collect::<Vec<_>>(),
            literal
                .glyphs
                .iter()
                .map(|g| (g.byte_index, &g.grapheme))
                .collect::<Vec<_>>()
        );
        let next = source.find("next").unwrap();
        let next_row = code.glyphs.iter().find(|g| g.byte_index == next).unwrap();
        assert_eq!(next_row.point.x, 0);
        for row in 1..next_row.point.y {
            let first = code.glyphs.iter().find(|g| g.point.y == row).unwrap();
            assert_eq!(first.point.x, 4.min(columns / 4));
        }
        assert!(
            code.glyphs
                .iter()
                .all(|g| g.point.x + g.grapheme.width().get() <= columns)
        );
    }
}
