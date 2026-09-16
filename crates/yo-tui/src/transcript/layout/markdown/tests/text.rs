use super::support::*;

// 페이지 렌더도 기존 Markdown의 강조·링크·코드·diff 색상 및 빈 행 배경을
// 보존하며, 비연속 페이지에서 같은 셀을 생성하는지 비교합니다.
#[test]
fn paged_markdown_preserves_existing_cells_styles_and_links() {
    for source in [
        "# Result\n\n**bold and *nested*** [link](https://example.com)\n\n> quote\n\n- 한글 item",
        "```rust\n  let value = \"literal\";\n\n\tprintln!(\"é 🙂\");\n```",
        "```diff\n@@ -1 +1 @@\n-old\n+new\n unchanged\n```",
        "| A | B |\n|---|---|\n| one | **two** |",
    ] {
        for columns in [8, 20, 80] {
            let width = NonZeroU16::new(columns).unwrap();
            let old = prepare(source, width).unwrap();
            let paged = PagedMarkdown::new(
                source,
                width,
                true,
                NonZeroU16::new(64).unwrap(),
                true,
                None,
                1,
            )
            .unwrap();
            assert_eq!(paged.height, usize::from(old.height));
            for first in 0..paged.height {
                let page = paged.window(first, NonZeroU16::new(3).unwrap());
                for glyph in &old.glyphs {
                    if !(first..first + 3).contains(&usize::from(glyph.point.y)) {
                        continue;
                    }
                    let y = glyph.point.y - first as u16;
                    let actual = page
                        .glyphs
                        .iter()
                        .find(|g| g.point == Point::new(glyph.point.x, y))
                        .unwrap();
                    assert_eq!(actual.grapheme, glyph.grapheme, "{source:?} {columns}");
                    assert_eq!(actual.decoration, glyph.decoration, "{source:?} {columns}");
                    assert_eq!(actual.hyperlink, glyph.hyperlink);
                }
                let old_styles: Vec<_> = old
                    .row_styles
                    .iter()
                    .filter(|(row, _)| (first..first + 3).contains(&usize::from(*row)))
                    .map(|(row, style)| (*row - first as u16, *style))
                    .collect();
                assert_eq!(page.row_styles, old_styles);
            }
        }
    }
}

// 하나의 긴 코드 블록도 전체 셀 배열 없이 마지막 페이지와 diff 의미 색상을
// 유지하고 65,535행 다음 위치까지 탐색할 수 있어야 합니다.
#[test]
fn paged_markdown_reads_large_code_blocks_without_cell_retention() {
    let source = format!("```diff\n{}+END\n```", "+한글\n".repeat(70_000));
    let width = NonZeroU16::new(20).unwrap();
    let paged = PagedMarkdown::new(&source, width, false, width, false, None, 1).unwrap();
    assert_eq!(paged.height, 70_003);
    let page = paged.window(70_000, NonZeroU16::new(3).unwrap());
    assert!(rows(&page).iter().any(|row| row.contains("+END")));
    assert!(page.glyphs.len() < 40);
    let end = page
        .glyphs
        .iter()
        .find(|glyph| glyph.grapheme.as_str() == "E")
        .unwrap();
    assert_eq!(end.decoration.role, Role::DiffAdded);
}

// 한 표 값이 65,535행을 넘어도 정규화·열 배치에서 잘리지 않고 마지막 글자를 읽습니다.
#[test]
fn paged_markdown_preserves_large_table_cells() {
    let source = format!("| A |\n| --- |\n| {}END |", "x".repeat(300_000));
    let pages = PagedMarkdown::new(
        &source,
        NonZeroU16::new(4).unwrap(),
        false,
        NonZeroU16::new(64).unwrap(),
        false,
        None,
        1,
    )
    .unwrap();
    assert!(pages.height > usize::from(u16::MAX));
    let tail = pages.window(pages.height - 4, NonZeroU16::new(4).unwrap());
    let text = tail
        .glyphs
        .iter()
        .map(|g| g.grapheme.as_str())
        .collect::<String>();
    assert!(text.contains("END"), "{text}");
    assert!(tail.glyphs.len() <= 16);
}

// 좁은 코드 패널은 좌우 여백 안에서 줄바꿈하고 장식 선 없이 원문 부호를 보존한다.
#[test]
fn narrow_code_wraps_with_padding_without_decorative_rails() {
    let rendered = prepare(
        "```diff\n+abcdefghij\n next\n```",
        NonZeroU16::new(10).unwrap(),
    )
    .unwrap();
    assert_eq!(
        rows(&rendered),
        [" diff", " +abcdefg", " hij", "  next", " "]
    );
    let expanded = prepare(
        "```diff\n+abcdefghij\n next\n```",
        NonZeroU16::new(30).unwrap(),
    )
    .unwrap();
    assert_eq!(rows(&expanded), [" diff", " +abcdefghij", "  next", " "]);
}

// 제목과 중첩 강조는 속성을 합치고 fenced code 내부 기호와 공백은 그대로 남긴다.
#[test]
fn headings_nested_emphasis_and_literal_code_have_distinct_styles() {
    let rendered = prepare(
        "# Result\n\n**bold and *nested***\n\n```rust\n  **literal**\n```",
        NonZeroU16::new(50).unwrap(),
    )
    .unwrap();
    assert_eq!(
        rows(&rendered),
        [
            "Result",
            "",
            "bold and nested",
            "",
            " rust",
            "   **literal**",
            " "
        ]
    );
    let nested = rendered
        .glyphs
        .iter()
        .find(|g| g.point == Point::new(9, 2))
        .unwrap();
    assert!(nested.decoration.attributes.contains(Attributes::BOLD));
    assert!(nested.decoration.attributes.contains(Attributes::ITALIC));
    assert_eq!(rendered.glyphs[0].decoration.role, Role::Heading);
    assert_eq!(
        rendered
            .row_styles
            .iter()
            .map(|(row, _)| *row)
            .collect::<Vec<_>>(),
        [4, 5, 6]
    );
    assert!(
        rendered
            .glyphs
            .iter()
            .filter(|g| g.point.y == 5 && g.point.x >= 2)
            .all(|g| g.decoration.role == Role::Code
                && g.decoration.attributes == Attributes::empty())
    );
}

// 목록은 단어 단위로 줄바꿈하고 이어지는 줄에는 bullet 대신 들여쓰기를 남긴다.
#[test]
fn lists_wrap_words_with_hanging_indents_and_preserve_numbering() {
    let rendered = prepare(
        "3. alpha beta gamma\n4. next\n   - child\n\n> quoted text",
        NonZeroU16::new(13).unwrap(),
    )
    .unwrap();
    assert_eq!(
        rows(&rendered),
        [
            "3. alpha beta",
            "   gamma",
            "4. next",
            "   - child",
            "",
            "> quoted text"
        ]
    );
}

// streaming 중 닫히지 않은 fence와 강조도 모든 UTF-8 경계에서 폭 안에 렌더링한다.
#[test]
fn incomplete_streaming_markdown_remains_bounded_with_cjk_and_emoji() {
    let source = "# 한글\n\n- **hello 한글 👩‍💻**\n\n```rust\nlet value = `literal`;\n";
    for width in [4, 8, 20, 80] {
        for end in source.char_indices().map(|(i, _)| i).chain([source.len()]) {
            // ZWJ and combining clusters can be temporarily incomplete in the source.
            match prepare(&source[..end], NonZeroU16::new(width).unwrap()) {
                Ok(rendered) => {
                    for glyph in rendered.glyphs {
                        assert!(glyph.point.x + glyph.grapheme.width().get() <= width);
                        assert!(glyph.point.y < rendered.height);
                    }
                },
                Err(TextFlowError::UnrenderableGrapheme { .. }) => {},
                Err(error) => {
                    panic!("unexpected layout failure at {end}, width {width}: {error:?}")
                },
            }
        }
    }
    let rendered = prepare(source, NonZeroU16::new(30).unwrap()).unwrap();
    assert!(
        rows(&rendered)
            .iter()
            .any(|row| row.contains("let value = `literal`;"))
    );
}

// 긴 단어만 grapheme 단위로 나누며, 일반 단어와 한글은 앞 단어에 밀려 잘리지 않는다.
#[test]
fn prose_wrap_retains_styles_across_whitespace_and_hard_breaks() {
    let rendered = prepare(
        "alpha **beta** gamma  \n한글 한글\n\nabcdefghijk",
        NonZeroU16::new(9).unwrap(),
    )
    .unwrap();
    assert_eq!(
        rows(&rendered),
        ["alpha", "beta", "gamma", "한글 한글", "", "abcdefghi", "jk"]
    );
    assert!(
        rendered
            .glyphs
            .iter()
            .filter(|g| g.point.y == 1)
            .all(|g| g.decoration.attributes.contains(Attributes::BOLD))
    );
}

// 표는 한글 셀 폭과 우측 정렬을 계산하고 inline 강조를 숫자 끝까지 보존한다.
#[test]
fn tables_align_wide_cells_and_keep_inline_styles() {
    let source = "| Name | Count |\n| :--- | ---: |\n| 한글 | **12** |\n| a | 3 |";
    let rendered = prepare(source, NonZeroU16::new(30).unwrap()).unwrap();
    assert_eq!(
        rows(&rendered),
        [
            "Name │ Count",
            "─────┼──────",
            "한글 │    12",
            "a    │     3"
        ]
    );
    for x in [10, 11] {
        let glyph = rendered
            .glyphs
            .iter()
            .find(|g| g.point == Point::new(x, 2))
            .unwrap();
        assert!(glyph.decoration.attributes.contains(Attributes::BOLD));
    }
    assert!(rendered.row_styles.is_empty());
}

// 표가 한 칸만 넘쳐도 값을 자르지 않고 header:value로 전환하며 행 사이를 구분한다.
#[test]
fn table_first_excess_column_switches_to_lossless_stacked_rows() {
    let source = "| Name | Count |\n| --- | ---: |\n| 한글 | 12 |\n| beta | 3 |";
    let fits = prepare(source, NonZeroU16::new(12).unwrap()).unwrap();
    assert_eq!(rows(&fits)[0], "Name │ Count");
    let narrow = prepare(source, NonZeroU16::new(11).unwrap()).unwrap();
    assert_eq!(
        rows(&narrow),
        ["Name: 한글", "Count: 12", "", "Name: beta", "Count: 3"]
    );
    assert!(
        narrow
            .glyphs
            .iter()
            .all(|g| g.point.x + g.grapheme.width().get() <= 11)
    );
}

// quote 안의 표에도 prefix를 유지하고 cell의 탭·제어 문자를 정렬 전에 가시 표기로 바꾼다.
#[test]
fn tables_keep_quote_context_and_safely_expand_cell_controls() {
    let rendered = prepare(
        "> | Key | Value |\n> | --- | --- |\n> | x | a\t\u{1b}b |\n\nAfter",
        NonZeroU16::new(50).unwrap(),
    )
    .unwrap();
    let text = rows(&rendered);
    assert!(text[0].starts_with("> Key"));
    assert!(text[1].starts_with("> "));
    assert!(text[2].starts_with("> x"));
    assert!(text[2].contains("^[b"));
    assert!(!text.join("\n").contains('\u{1b}'));
    assert_eq!(text.last().unwrap(), "After");
}

// diff fence에서만 추가·삭제·메타 역할을 구분하고 줄바꿈되어도 같은 변경 역할을 유지한다.
#[test]
fn diff_fences_keep_signs_and_distinguish_file_headers_from_changes() {
    let source = "```diff\n--- a/file\n+++ b/file\n@@ -1 +1 @@\n-old value\n+new value that wraps\n context\n```\n\n```text\n+ordinary log\n```";
    let rendered = prepare(source, NonZeroU16::new(16).unwrap()).unwrap();
    let text = rows(&rendered).join("\n");
    assert!(text.contains("-old value"));
    assert!(text.contains("+new value"));
    let removed = rendered
        .glyphs
        .iter()
        .find(|g| g.grapheme.as_str() == "o" && g.decoration.role == Role::DiffRemoved)
        .unwrap();
    assert_eq!(removed.point.y, 4);
    assert!(
        rendered
            .glyphs
            .iter()
            .filter(|g| g.point.y == 6 && g.point.x >= 2)
            .all(|g| g.decoration.role == Role::DiffAdded)
    );
    assert!(
        rendered
            .glyphs
            .iter()
            .filter(|g| (1..=3).contains(&g.point.y) && g.point.x >= 2)
            .all(|g| g.decoration.role == Role::DiffMeta)
    );
    assert!(
        rendered
            .glyphs
            .iter()
            .any(|g| g.grapheme.as_str() == "+" && g.decoration.role == Role::Code)
    );
}

// 변경된 본문이 ++/--로 시작해도 파일 header의 공백 구분자가 없으면 추가·삭제로 남는다.
#[test]
fn diff_metadata_requires_a_file_header_separator() {
    let rendered = prepare(
        "```diff\n---flag\n+++count\n```",
        NonZeroU16::new(30).unwrap(),
    )
    .unwrap();
    assert!(
        rendered
            .glyphs
            .iter()
            .filter(|g| g.point.y == 1 && g.point.x >= 2)
            .all(|g| g.decoration.role == Role::DiffRemoved)
    );
    assert!(
        rendered
            .glyphs
            .iter()
            .filter(|g| g.point.y == 2 && g.point.x >= 2)
            .all(|g| g.decoration.role == Role::DiffAdded)
    );
}

// centered cell의 좌우 공백을 prose 정규화로 지우지 않고 terminal 열 위치를 보존한다.
#[test]
fn centered_table_cells_preserve_padding() {
    let rendered = prepare("| label |\n| :---: |\n| x |", NonZeroU16::new(10).unwrap()).unwrap();
    assert_eq!(rows(&rendered), ["label", "─────", "  x  "]);
    assert_eq!(
        rendered
            .glyphs
            .iter()
            .find(|g| g.grapheme.as_str() == "x")
            .unwrap()
            .point,
        Point::new(2, 2)
    );
}

// 중간 폭에서는 표의 열을 유지하며 긴 셀과 한글을 줄바꿈하고 강조를 보존한다.
#[test]
fn tables_wrap_inside_columns_without_losing_values() {
    let rendered = prepare("| Key | Description |\n| --- | --- |\n| alpha | **one two three four five** |\n| 한글 | 여섯 일곱 여덟 아홉 |", NonZeroU16::new(22).unwrap()).unwrap();
    let text = rows(&rendered);
    assert!(text[0].contains(" │ "));
    let values = text
        .iter()
        .skip(2)
        .map(|row| row.split(" │ ").nth(1).unwrap().trim())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(values.contains("one two three four five"));
    assert!(values.contains("여섯 일곱 여덟 아홉"));
    assert!(
        rendered
            .glyphs
            .iter()
            .all(|g| g.point.x + g.grapheme.width().get() <= 22)
    );
    let five = rendered
        .glyphs
        .iter()
        .find(|g| g.grapheme.as_str() == "v")
        .unwrap();
    assert!(five.decoration.attributes.contains(Attributes::BOLD));
}

// 표의 좁은 열은 유지하고 나머지 폭은 앞 열부터 배분하며 큰 셀도 재배치 중 보존한다.
#[test]
fn table_width_allocation_preserves_ties_and_large_cell_contents() {
    let source = "| K | V | W |\n| --- | --- | --- |\n| abc | abcdefghij | klmnopqrst |";
    for (width, expected) in [
        (25, [3, 8, 8]),
        (26, [3, 9, 8]),
        (27, [3, 9, 9]),
        (28, [3, 10, 9]),
    ] {
        let rendered = prepare(source, NonZeroU16::new(width).unwrap()).unwrap();
        let text = rows(&rendered);
        assert_eq!(
            text[0].split(" │ ").map(str::len).collect::<Vec<_>>(),
            expected,
        );
        for (column, value) in ["abc", "abcdefghij", "klmnopqrst"].iter().enumerate() {
            let retained = text
                .iter()
                .skip(2)
                .map(|row| row.split(" │ ").nth(column).unwrap().trim())
                .collect::<String>();
            assert_eq!(&retained, value);
        }
    }
    let header = vec!["K"; 32].join(" | ");
    let rule = vec!["---"; 32].join(" | ");
    let value = "x".repeat(2048);
    let data = vec![value.as_str(); 32].join(" | ");
    let source = format!("| {header} |\n| {rule} |\n| {data} |");
    for width in [480, 481, 480] {
        let rendered = prepare(&source, NonZeroU16::new(width).unwrap()).unwrap();
        let text = rows(&rendered);
        assert_eq!(text[0].len(), usize::from(width) + 32 * 2 - 2);
        for column in 0..32 {
            let retained = text
                .iter()
                .skip(2)
                .map(|row| row.split(" │ ").nth(column).unwrap().trim())
                .collect::<String>();
            assert_eq!(retained, value);
        }
        assert!(
            rendered
                .glyphs
                .iter()
                .all(|glyph| glyph.point.x + glyph.grapheme.width().get() <= width)
        );
    }
}

// 언어 parser는 문자열 속 키워드와 여러 줄 주석을 구분하며, 좁은 폭에서도 원문 문자를
// 바꾸지 않는다. 미지원 언어는 읽을 수 있는 일반 코드로 남긴다.
#[test]
fn syntax_highlighting_tracks_multiline_context_and_preserves_source() {
    let source =
        "```rust\n/* fn hidden\n   still a comment */\nlet message = \"fn not_a_keyword\";\n```";
    for width in [12, 60] {
        let rendered = prepare(source, NonZeroU16::new(width).unwrap()).unwrap();
        assert!(
            rendered
                .glyphs
                .iter()
                .any(|g| g.decoration.role == Role::Syntax(0))
        );
        assert!(
            rendered
                .glyphs
                .iter()
                .any(|g| g.decoration.role == Role::Syntax(1))
        );
        assert!(
            rendered
                .glyphs
                .iter()
                .any(|g| g.decoration.role == Role::Syntax(2))
        );
        assert!(
            rendered
                .glyphs
                .iter()
                .all(|g| g.point.x + g.grapheme.width().get() <= width)
        );
        assert!(
            rendered
                .row_styles
                .iter()
                .all(|(_, d)| !matches!(d.role, Role::Syntax(_)))
        );
    }
    let literal = prepare(
        "```rust\nlet x = \"data:image/png;base64,VGhpcy\";\n```",
        NonZeroU16::new(60).unwrap(),
    )
    .unwrap();
    assert!(
        rows(&literal)
            .join("\n")
            .contains("data:image/png;base64,VGhpcy")
    );
    let unknown = prepare(
        "```unrecognized\nfn keep_me() {}\n```",
        NonZeroU16::new(40).unwrap(),
    )
    .unwrap();
    assert!(
        rows(&unknown)
            .iter()
            .any(|row| row.contains("fn keep_me() {}"))
    );
    assert!(
        !unknown
            .glyphs
            .iter()
            .any(|g| matches!(g.decoration.role, Role::Syntax(_)))
    );
}

// 좁은 diff에서도 문자열 토큰을 보존하고 줄바꿈한 모든 조각에 추가 행 스타일을 유지한다.
#[test]
fn code_word_wrap_preserves_diff_role_and_reflows_on_width_change() {
    let source = "```diff\n+let theme = \"selected\";\n```";
    for columns in [80, 24, 80] {
        let rendered = prepare(source, NonZeroU16::new(columns).unwrap()).unwrap();
        let lines = rows(&rendered);
        assert!(lines.iter().any(|line| line.contains("\"selected\";")));
        let source_chars: String = rendered
            .glyphs
            .iter()
            .filter(|glyph| glyph.decoration.role == Role::DiffAdded)
            .map(|glyph| glyph.grapheme.as_str())
            .collect();
        assert_eq!(
            source_chars.split_whitespace().collect::<String>(),
            "+lettheme=\"selected\";"
        );
    }
}
