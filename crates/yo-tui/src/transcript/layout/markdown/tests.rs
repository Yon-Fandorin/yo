use std::io::Cursor;

use base64::{Engine, engine::general_purpose::STANDARD};

use super::{image::decode_image, *};
use crate::surface::Size;

fn rows(prepared: &PreparedMarkdown) -> Vec<String> {
    let mut rows = vec![String::new(); usize::from(prepared.height)];
    for glyph in &prepared.glyphs {
        rows[usize::from(glyph.point.y)].push_str(glyph.grapheme.as_str());
    }
    rows
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

// 링크는 label과 목적지를 모두 표시하고 escape·HTML·제어문자는 실행하지 않는 셀로 남긴다.
#[test]
fn links_escapes_and_control_notation_preserve_readable_content() {
    let rendered = prepare(
        "[guide](https://example.test) and \\*literal\\*\n\n<span>raw</span>\n\n`\u{1b}[31m`",
        NonZeroU16::new(80).unwrap(),
    )
    .unwrap();
    let text = rows(&rendered).join("\n");
    assert!(text.contains("guide (https://example.test) and *literal*"));
    assert!(text.contains("<span>raw</span>"));
    assert!(!text.contains('\u{1b}'));
    assert!(
        rendered
            .glyphs
            .iter()
            .any(|g| g.decoration.role == Role::Link)
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

// 음수 막대는 0선 왼쪽, 양수는 오른쪽에 표시하고 숫자를 그대로 남긴다.
// NaN·불완전한 데이터는 잘못된 차트를 만들지 않고 원문으로 되돌린다.
#[test]
fn charts_preserve_values_zero_direction_and_invalid_source() {
    let rendered = prepare(
        "```chart\nDown: -8\nZero: 0\nUp: 4\n```",
        NonZeroU16::new(20).unwrap(),
    )
    .unwrap();
    let lines = rows(&rendered);
    assert!(lines.iter().any(|line| line.contains("Down  -8")));
    assert!(lines.iter().any(|line| line.contains("Zero  0")));
    assert!(lines.iter().any(|line| line.contains("█│")));
    assert!(lines.iter().any(|line| line.contains("│█")));
    let axes: Vec<_> = rendered
        .glyphs
        .iter()
        .filter(|g| g.grapheme.as_str() == "│")
        .map(|g| g.point.x)
        .collect();
    assert_eq!(axes.len(), 3);
    assert!(axes.iter().all(|x| *x == axes[0]));
    for source in [
        "```chart\nValue: NaN\n```",
        "```chart\nValue: pending\n```",
        "```sparkline\n\n```",
    ] {
        let rendered = prepare(source, NonZeroU16::new(20).unwrap()).unwrap();
        assert!(
            !rendered
                .glyphs
                .iter()
                .any(|g| g.decoration.role == Role::Chart)
        );
    }
    let trend = prepare("```sparkline\n5 5 5\n```", NonZeroU16::new(20).unwrap()).unwrap();
    assert!(rows(&trend).iter().any(|line| line.contains("▅▅▅")));
}

// 실제 PNG의 서로 다른 픽셀 색과 크기가 cell preview까지 유지되고, 실패한 이미지의
// base64 문자열은 본문에 쏟아지지 않는다. decode 크기 제한은 원본에 적용된다.
#[test]
fn embedded_images_decode_bound_dimensions_and_hide_invalid_payloads() {
    use ::image::{ImageFormat, Rgb, RgbImage};
    let pixels = RgbImage::from_fn(4, 2, |x, _| {
        if x < 2 {
            Rgb([255, 0, 0])
        } else {
            Rgb([0, 0, 255])
        }
    });
    let mut bytes = Cursor::new(Vec::new());
    pixels.write_to(&mut bytes, ImageFormat::Png).unwrap();
    let source = format!(
        "![Color check](data:image/png;base64,{})",
        STANDARD.encode(bytes.into_inner())
    );
    let rendered = prepare(&source, NonZeroU16::new(40).unwrap()).unwrap();
    assert!(
        rendered
            .glyphs
            .iter()
            .any(|g| g.decoration.role == Role::Pixel([255, 0, 0], [255, 0, 0]))
    );
    assert!(
        rendered
            .glyphs
            .iter()
            .any(|g| g.decoration.role == Role::Pixel([0, 0, 255], [0, 0, 255]))
    );
    assert!(rows(&rendered).join("\n").contains("4 × 2"));
    assert_eq!(rendered.rasters[0].area.size, Size::new(4, 1));
    let partial = prepare(
        "![Photo](data:image/png;base64,VGhpcy1pcy1ub3QtcG5n",
        NonZeroU16::new(40).unwrap(),
    )
    .unwrap();
    assert!(!rows(&partial).join("\n").contains("VGhpcy"));
    let large = RgbImage::new(2049, 1);
    let mut bytes = Cursor::new(Vec::new());
    large.write_to(&mut bytes, ImageFormat::Png).unwrap();
    assert!(
        decode_image(&format!(
            "data:image/png;base64,{}",
            STANDARD.encode(bytes.into_inner())
        ))
        .is_err()
    );
}

// 색상 미지원과 ASCII 설정에서도 이미지가 빈 사각형이 되지 않고 명암으로 남는다.
#[test]
fn media_fallback_uses_ascii_density_and_indexed_colors() {
    let plain = MarkdownStyles::plain(Style::default());
    let dark = Decoration::role(Role::Pixel([0, 0, 0], [0, 0, 0]));
    let light = Decoration::role(Role::Pixel([255, 255, 255], [255, 255, 255]));
    assert_eq!(
        plain
            .display_glyph(dark, Grapheme::try_from("▀").unwrap())
            .as_str(),
        " "
    );
    assert_eq!(
        plain
            .display_glyph(light, Grapheme::try_from("▀").unwrap())
            .as_str(),
        "@"
    );
    assert_eq!(
        plain.resolve(light, Style::default()).background,
        Color::Default
    );
    assert!(matches!(
        pixel_color([123, 45, 67], Color::Indexed(1)),
        Color::Indexed(_)
    ));
    let ascii = MarkdownStyles {
        rich_media: false,
        ..plain
    };
    assert_eq!(
        ascii
            .display_glyph(
                Decoration::role(Role::Chart),
                Grapheme::try_from("━").unwrap()
            )
            .as_str(),
        "="
    );
}

// 원본 PNG는 썸네일과 별도로 유지되어 terminal 전송 시 해상도를 잃지 않는다.
#[test]
fn native_image_keeps_original_png_and_bounded_cell_placement() {
    use ::image::{ImageFormat, RgbImage};
    let image = RgbImage::new(120, 60);
    let mut encoded = Cursor::new(Vec::new());
    image.write_to(&mut encoded, ImageFormat::Png).unwrap();
    let bytes = encoded.into_inner();
    let markdown = format!(
        "![Original](data:image/png;base64,{})",
        STANDARD.encode(&bytes)
    );
    let prepared = prepare(&markdown, NonZeroU16::new(40).unwrap()).unwrap();
    assert_eq!(prepared.rasters.len(), 1);
    let raster = &prepared.rasters[0];
    assert_eq!(&*raster.png, bytes.as_slice());
    assert!(raster.area.size.width <= 40);
    assert!(raster.area.size.height <= 20);
    assert!(raster.area.origin.y + raster.area.size.height <= prepared.height);
}

// 선 차트는 실제 데이터 점들을 연결하고 축·범위를 제공한다. 좁은 폭은 sparkline으로
// 내려가도 값은 보존되고 색상/문자 fallback과 무관하게 범위를 벗어나지 않는다.
#[test]
fn line_chart_is_connected_and_responsive() {
    for width in [12, 40, 88] {
        let prepared = prepare(
            "```linechart\n12 8 16 10 24\n```",
            NonZeroU16::new(width).unwrap(),
        )
        .unwrap();
        assert!(
            prepared
                .glyphs
                .iter()
                .all(|g| g.point.x + g.grapheme.width().get() <= width)
        );
        if width >= 40 {
            assert!(prepared.glyphs.iter().any(|g| {
                g.grapheme
                    .as_str()
                    .chars()
                    .any(|c| ('\u{2801}'..='\u{28ff}').contains(&c))
            }));
            assert!(rows(&prepared).join("\n").contains("1 → 5 samples"));
        }
    }
}

// 표시를 끄면 잘못된 데이터도 디코딩 오류나 원문 URL 대신 대체 설명만 남긴다.
#[test]
fn disabled_images_skip_decode_and_preserve_alt_text() {
    let prepared = prepare_with_images(
        "![설명](data:image/png;base64,INVALID)",
        NonZeroU16::new(40).unwrap(),
        false,
        NonZeroU16::new(8).unwrap(),
    )
    .unwrap();
    let text = rows(&prepared).join("\n");
    assert!(text.contains("설명"));
    assert!(text.contains("Image display disabled"));
    assert!(!text.contains("INVALID"));
    assert!(!text.contains("base64"));
    assert!(prepared.rasters.is_empty());
}

// Mermaid 흐름도·시퀀스는 실제 연결선을 만들고 좁은 폭·비활성·잘못된 문법은 원문을 보존한다.
#[test]
fn mermaid_diagrams_render_and_fall_back_without_clipping_source() {
    for source in [
        "graph TD; A[Build] --> B[Test]",
        "sequenceDiagram\nAlice->>Bob: Hello",
    ] {
        let fenced = format!("```mermaid\n{source}\n```");
        let view = prepare(&fenced, NonZeroU16::new(100).unwrap()).unwrap();
        let text = rows(&view).join("\n");
        assert!(!text.contains("shown as source"), "{text}");
        assert!(text.contains('│') || text.contains('─'), "{text}");
        assert!(!text.contains("graph TD"), "{text}");
        let disabled = prepare_with_media(
            &fenced,
            NonZeroU16::new(100).unwrap(),
            true,
            NonZeroU16::new(64).unwrap(),
            false,
        )
        .unwrap();
        let text = rows(&disabled).join("\n");
        assert!(
            text.split_whitespace()
                .collect::<String>()
                .contains(&source.split_whitespace().collect::<String>()),
            "{text}"
        );
        let narrow = prepare(&fenced, NonZeroU16::new(12).unwrap()).unwrap();
        let text = rows(&narrow).join("\n");
        let joined = text
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        assert!(joined.contains("Diagramshownassource"), "{text}");
        assert!(
            joined.contains(
                &source
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
            ),
            "{text}"
        );
    }
    let invalid = prepare(
        "```mermaid\nnot a diagram\n```",
        NonZeroU16::new(80).unwrap(),
    )
    .unwrap();
    assert!(rows(&invalid).join("\n").contains("not a diagram"));
}

// 렌더링 전에 크기 상한을 적용하고 첫 초과 문장은 잘리지 않은 코드 원문으로 남긴다.
#[test]
fn mermaid_diagram_limits_preserve_the_first_excess_statement() {
    let source = format!(
        "graph TD;{}",
        (0..64)
            .map(|n| format!("N{n}-->N{};", n + 1))
            .collect::<String>()
    );
    let view = prepare(
        &format!("```mermaid\n{source}\n```"),
        NonZeroU16::new(80).unwrap(),
    )
    .unwrap();
    let text = rows(&view).join("\n");
    assert!(text.contains("diagram exceeds preview limits"), "{text}");
    assert!(
        text.chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .contains("N63-->N64;"),
        "{text}"
    );
}

// 일반 URL의 문장 부호는 제외하지만 주소 내부 괄호·IPv6·entity로 나뉜 query는 보존한다.
// 이미 지정된 목적지와 코드의 원문을 자동 인식으로 덮어쓰지 않는다.
#[test]
fn bare_web_links_preserve_complete_decoded_destinations_and_explicit_ownership() {
    let source = "See (https://example.com/a). https://en.wikipedia.org/wiki/Function_(mathematics),\n\nhttps://example.com/?a=1&amp;b=2 and http://[::1]:8080/path!\n\n[https://visible.example/](https://actual.example/) [https://blocked.example/](javascript:alert(1))\n\n`https://inline.example/`\n\n```text\nhttps://code.example/\n```\n\n| Source | Link |\n| --- | --- |\n| table | https://table.example/very/long/path |";
    let expected = [
        "https://example.com/a",
        "https://en.wikipedia.org/wiki/Function_(mathematics)",
        "https://example.com/?a=1&b=2",
        "http://[::1]:8080/path",
        "https://actual.example/",
        "https://table.example/very/long/path",
    ];
    for width in [80, 24, 80] {
        let prepared = prepare(source, NonZeroU16::new(width).unwrap()).unwrap();
        let mut targets = prepared
            .glyphs
            .iter()
            .filter_map(|glyph| glyph.hyperlink.as_ref().map(|link| link.destination()))
            .collect::<Vec<_>>();
        targets.sort_unstable();
        targets.dedup();
        let mut expected = expected.to_vec();
        expected.sort_unstable();
        assert_eq!(targets, expected);
        let linked = prepared
            .glyphs
            .iter()
            .filter(|glyph| {
                glyph
                    .hyperlink
                    .as_ref()
                    .is_some_and(|link| link.destination() == "https://example.com/a")
            })
            .map(|glyph| glyph.grapheme.as_str())
            .collect::<String>();
        assert_eq!(linked, "https://example.com/a");
        let explicit = prepared
            .glyphs
            .iter()
            .filter(|glyph| {
                glyph
                    .hyperlink
                    .as_ref()
                    .is_some_and(|link| link.destination() == "https://actual.example/")
            })
            .map(|glyph| glyph.grapheme.as_str())
            .collect::<String>();
        assert!(
            explicit.starts_with("https://visible.example/"),
            "{explicit}"
        );
    }
}

// 스트리밍 중 주소가 교체되면 새 전체 주소를 사용하고 여러 URL의 범위가 서로 섞이지 않는다.
#[test]
fn bare_web_links_track_streamed_text_and_adjacent_punctuation() {
    for suffix in ["", "/first", "/changed?q=value"] {
        let target = format!("https://example.com{suffix}");
        let source = format!("({target}), 'https://second.example/next'.");
        let prepared = prepare(&source, NonZeroU16::new(14).unwrap()).unwrap();
        let first = prepared
            .glyphs
            .iter()
            .filter(|glyph| {
                glyph
                    .hyperlink
                    .as_ref()
                    .is_some_and(|link| link.destination() == target)
            })
            .map(|glyph| glyph.grapheme.as_str())
            .collect::<String>();
        assert_eq!(first, target);
        let second = prepared
            .glyphs
            .iter()
            .filter(|glyph| {
                glyph
                    .hyperlink
                    .as_ref()
                    .is_some_and(|link| link.destination() == "https://second.example/next")
            })
            .map(|glyph| glyph.grapheme.as_str())
            .collect::<String>();
        assert_eq!(second, "https://second.example/next");
    }
}

// 표현 가능한 최소 양수도 0과 다른 높이로 표시하고 큰 지수 값에서도 축을 같은 열에 맞춘다.
#[test]
fn charts_preserve_extreme_numeric_ranges_and_axis_alignment() {
    for source in ["0 5e-324", "-5e-324 0", "-1e308 1e308"] {
        let chart = prepare(
            &format!("```sparkline\n{source}\n```"),
            NonZeroU16::new(24).unwrap(),
        )
        .unwrap();
        assert!(
            rows(&chart).iter().any(|row| row.contains("▁█")),
            "{:?}",
            rows(&chart)
        );
        let text = rows(&chart)
            .join(" ")
            .split_whitespace()
            .collect::<String>();
        assert!(
            text.contains(&format!(
                "last{}",
                source.split_whitespace().last().unwrap()
            )),
            "{text}"
        );
    }
    for width in [18, 24, 40] {
        let chart = prepare(
            "```linechart\n0 1e308 -1e308\n```",
            NonZeroU16::new(width).unwrap(),
        )
        .unwrap();
        let axes: Vec<_> = chart
            .glyphs
            .iter()
            .filter(|glyph| glyph.grapheme.as_str() == "│")
            .map(|glyph| glyph.point)
            .collect();
        assert_eq!(axes.len(), 6, "{:?}", rows(&chart));
        assert!(axes.iter().all(|point| point.x == axes[0].x), "{axes:?}");
        assert!(
            axes.windows(2).all(|pair| pair[1].y == pair[0].y + 1),
            "{axes:?}"
        );
    }
}

// 막대 길이는 수치로 계산하되 지수·부호·소수 자릿수는 입력 표기를 그대로 보여준다.
#[test]
fn bar_charts_keep_original_numeric_notation() {
    for width in [18, 80] {
        let chart = prepare(
            "```chart\nTiny: 5e-324\nDecimal: 1.2300\nSigned: +2.00\nZero: -0.0\n```",
            NonZeroU16::new(width).unwrap(),
        )
        .unwrap();
        let text = rows(&chart).join(" ");
        for literal in ["5e-324", "1.2300", "+2.00", "-0.0"] {
            assert!(text.contains(literal), "{text}");
        }
        assert!(!text.contains("0000000000"), "{text}");
    }
}

// 각주는 정의와 참조의 이름을 보존하고 본문의 링크·코드를 렌더링한다. 미정의·코드 내부 참조는
// 원문이다.
#[test]
fn footnotes_keep_labels_rich_definitions_and_literal_fallbacks() {
    let source = "Claim[^출처] and missing[^absent].\n\n[^출처]: Read [docs](https://example.com/source) and `code`.\n\n    - Nested **detail**\n\nAfter the note.\n\n```text\n[^출처]\n```";
    for width in [12, 24, 80] {
        let prepared = prepare(source, NonZeroU16::new(width).unwrap()).unwrap();
        let text = rows(&prepared)
            .join(" ")
            .split_whitespace()
            .collect::<String>();
        for expected in [
            "Claim[출처]",
            "missing[^absent]",
            "[출처]",
            "docs",
            "code",
            "Nesteddetail",
            "Afterthenote.",
            "[^출처]",
        ] {
            assert!(text.contains(expected), "{width}: {text}");
        }
        assert!(
            prepared
                .glyphs
                .iter()
                .filter_map(|glyph| glyph.hyperlink.as_ref())
                .any(|link| link.destination() == "https://example.com/source")
        );
        assert!(
            prepared
                .glyphs
                .iter()
                .all(|glyph| glyph.point.x + glyph.grapheme.width().get() <= width)
        );
        let labels: String = prepared
            .glyphs
            .iter()
            .filter(|glyph| {
                glyph.decoration.role == Role::Quote
                    && glyph.decoration.attributes.contains(Attributes::BOLD)
            })
            .map(|glyph| glyph.grapheme.as_str())
            .collect();
        assert!(labels.contains("[출처]"), "{labels}");
    }
}

// 계단 차트는 다음 관측까지 이전 값을 수평 유지하고 마지막 위치에서만 수직으로 바뀐다.
#[test]
fn step_chart_holds_previous_value_and_preserves_narrow_source() {
    for width in [12, 40, 88] {
        let prepared = prepare("```stepchart\n0 10\n```", NonZeroU16::new(width).unwrap()).unwrap();
        assert!(
            prepared
                .glyphs
                .iter()
                .all(|g| g.point.x + g.grapheme.width().get() <= width)
        );
        let text = rows(&prepared).join("\n");
        assert!(
            text.split_whitespace()
                .collect::<String>()
                .contains("Values:010"),
            "{text}"
        );
        if width >= 40 {
            let cells: Vec<_> = prepared
                .glyphs
                .iter()
                .filter_map(|g| {
                    let c = g.grapheme.as_str().chars().next()?;
                    ('\u{2800}'..='\u{28ff}')
                        .contains(&c)
                        .then_some((g.point.x, g.point.y, c))
                })
                .collect();
            assert!(!cells.is_empty());
            let right = cells.iter().map(|c| c.0).max().unwrap();
            let bottom = cells.iter().map(|c| c.1).max().unwrap();
            assert!(
                cells
                    .iter()
                    .any(|&(x, y, c)| x < right && y == bottom && c != '\u{2800}')
            );
            assert!(
                cells
                    .iter()
                    .any(|&(x, y, c)| x == right && y < bottom && c != '\u{2800}')
            );
            assert!(
                cells
                    .iter()
                    .all(|&(x, y, c)| c == '\u{2800}' || x == right || y == bottom),
                "{text}"
            );
        }
    }
    for source in ["NaN", "", "1 infinity"] {
        let prepared = prepare(
            &format!("```stepchart\n{source}\n```"),
            NonZeroU16::new(40).unwrap(),
        )
        .unwrap();
        assert!(rows(&prepared).join("\n").contains("Chart data incomplete"));
    }
}

// 구간 경계 값은 오른쪽 구간에 들어가고 최댓값은 마지막 구간에 포함되며 좁은 폭에도 개수를
// 보존한다.
#[test]
fn histogram_counts_boundaries_and_preserves_numeric_source() {
    for width in [80, 24, 12, 80] {
        let chart = prepare(
            "```histogram\nbins: 4\n0 1 2 2 3 4.00\n```",
            NonZeroU16::new(width).unwrap(),
        )
        .unwrap();
        let rendered = rows(&chart);
        let text = rendered.join(" ").split_whitespace().collect::<String>();
        assert!(text.contains("6samples·4bins·count"), "{text}");
        assert!(text.contains("Values:012234.00"), "{text}");
        if width >= 24 {
            for (interval, count) in [
                ("[0, 1)", "1"),
                ("[1, 2)", "1"),
                ("[2, 3)", "2"),
                ("[3, 4]", "2"),
            ] {
                let row = rendered.iter().find(|row| row.contains(interval)).unwrap();
                assert!(row.trim_end().ends_with(count), "{row}");
            }
        }
        assert!(
            chart
                .glyphs
                .iter()
                .any(|glyph| glyph.decoration.role == Role::Bar)
        );
    }
}

// bin·표본 한도의 첫 초과는 원문으로 돌아가고 상수·극단 범위도 유한한 구간으로 표시한다.
#[test]
fn histogram_limits_constant_and_extreme_values() {
    for (values, expected) in [
        ("bins: 32\n0 1", "32bins"),
        ("5 5 5", "1bin"),
        ("0 5e-324", "1bin"),
        ("bins: 2\n-1e308 0 1e308", "2bins"),
    ] {
        let chart = prepare(
            &format!("```histogram\n{values}\n```"),
            NonZeroU16::new(80).unwrap(),
        )
        .unwrap();
        let text = rows(&chart)
            .join(" ")
            .split_whitespace()
            .collect::<String>();
        assert!(text.contains(expected), "{text}");
        assert!(!text.contains("NaN") && !text.contains("inf"));
        assert!(!text.contains("0000000000"), "{text}");
    }
    let accepted = std::iter::repeat_n("1", 64).collect::<Vec<_>>().join(" ");
    for (source, valid) in [
        (accepted.clone(), true),
        (format!("{accepted} 1"), false),
        ("bins: 33\n0 1".into(), false),
        ("bins: 0\n0 1".into(), false),
        ("NaN".into(), false),
        ("bins: 2".into(), false),
    ] {
        let chart = prepare(
            &format!("```histogram\n{source}\n```"),
            NonZeroU16::new(80).unwrap(),
        )
        .unwrap();
        let text = rows(&chart).join(" ");
        assert_eq!(text.contains("Chart data incomplete"), !valid, "{text}");
        if !valid {
            assert!(
                text.split_whitespace()
                    .collect::<String>()
                    .contains(&source.split_whitespace().collect::<String>()),
                "{text}"
            );
        }
    }
}

// 산점도는 입력 순서가 아닌 X 값 간격으로 점을 놓고 연결선을 만들지 않으며 좁은 폭에는 좌표를
// 남긴다.
#[test]
fn scatter_uses_numeric_x_spacing_and_unconnected_points() {
    for width in [80, 24, 12, 80] {
        let chart = prepare(
            "```scatterchart\n0,0 1,5 10,10.00\n```",
            NonZeroU16::new(width).unwrap(),
        )
        .unwrap();
        let text = rows(&chart)
            .join(" ")
            .split_whitespace()
            .collect::<String>();
        assert!(text.contains("X0→10"), "{text}");
        assert!(text.contains("Y0→10·3points"), "{text}");
        assert!(text.contains("Points:0,01,510,10.00"), "{text}");
        let dots: Vec<_> = chart
            .glyphs
            .iter()
            .filter_map(|glyph| {
                let character = glyph.grapheme.as_str().chars().next()? as u32;
                (0x2801..=0x28ff)
                    .contains(&character)
                    .then(|| (glyph.point, character - 0x2800))
            })
            .collect();
        if width >= 24 {
            assert_eq!(
                dots.iter().map(|(_, mask)| mask.count_ones()).sum::<u32>(),
                3
            );
            assert_eq!(dots.len(), 3);
            let top = dots.iter().min_by_key(|(point, _)| point.y).unwrap().0;
            let bottom = dots.iter().max_by_key(|(point, _)| point.y).unwrap().0;
            let middle = dots
                .iter()
                .find(|(point, _)| point.y != top.y && point.y != bottom.y)
                .unwrap()
                .0;
            assert!(top.x > bottom.x);
            assert!(middle.x - bottom.x < (top.x - bottom.x) / 2);
        } else {
            assert!(dots.is_empty());
            assert!(text.contains("showingcoordinates"));
        }
    }
}

// 단일 점·상수 축·극단 좌표를 지원하고 좌표 한쪽 오류나 표본 한도의 첫 초과는 원문으로 돌린다.
#[test]
fn scatter_handles_constant_axes_and_rejects_invalid_coordinates() {
    for source in [
        "5,10",
        "5,10 5,20 5,30",
        "0,0 5e-324,5e-324",
        "-1e308,-1e308 1e308,1e308",
    ] {
        let chart = prepare(
            &format!("```scatterchart\n{source}\n```"),
            NonZeroU16::new(80).unwrap(),
        )
        .unwrap();
        let text = rows(&chart).join(" ");
        assert!(
            !text.contains("incomplete") && !text.contains("NaN") && !text.contains("inf"),
            "{text}"
        );
        assert!(chart.glyphs.iter().any(|glyph| {
            glyph
                .grapheme
                .as_str()
                .chars()
                .any(|ch| ('\u{2801}'..='\u{28ff}').contains(&ch))
        }));
    }
    let accepted = std::iter::repeat_n("0,1", 64).collect::<Vec<_>>().join(" ");
    for (source, valid) in [
        (accepted.clone(), true),
        (format!("{accepted} 0,1"), false),
        ("NaN,1".into(), false),
        ("1,inf".into(), false),
        ("1,2,3".into(), false),
        ("1,".into(), false),
        ("1 2".into(), false),
    ] {
        let chart = prepare(
            &format!("```scatterchart\n{source}\n```"),
            NonZeroU16::new(80).unwrap(),
        )
        .unwrap();
        let text = rows(&chart).join(" ");
        assert_eq!(text.contains("Chart data incomplete"), !valid, "{text}");
        assert!(
            text.split_whitespace()
                .collect::<String>()
                .contains(&source.split_whitespace().collect::<String>()),
            "{text}"
        );
    }
}

// 차트별 높이는 실제 축 행 수에 반영되고 최소·최대 경계와 좁은 폭 원문 수치를 보존한다.
#[test]
fn chart_height_controls_plot_rows_and_rejects_first_excess() {
    for (kind, values) in [
        ("linechart", "1 5 2"),
        ("stepchart", "1 5 2"),
        ("scatterchart", "0,1 1,5 2,2"),
    ] {
        for height in [2, 3, 6, 16] {
            for width in [80, 24, 12] {
                let chart = prepare(
                    &format!("```{kind}\nheight: {height}\n{values}\n```"),
                    NonZeroU16::new(width).unwrap(),
                )
                .unwrap();
                let axes = chart
                    .glyphs
                    .iter()
                    .filter(|glyph| glyph.grapheme.as_str() == "│")
                    .count();
                assert_eq!(axes, if width >= 24 { height } else { 0 });
                let text = rows(&chart)
                    .join(" ")
                    .split_whitespace()
                    .collect::<String>();
                assert!(!text.contains("height:"), "{text}");
                assert!(
                    text.contains(&values.split_whitespace().collect::<String>()),
                    "{text}"
                );
                assert!(!text.contains("incomplete"));
            }
        }
        for height in ["1", "17", "-1", "2.5", "pending"] {
            let source = format!("height: {height}\n{values}");
            let chart = prepare(
                &format!("```{kind}\n{source}\n```"),
                NonZeroU16::new(80).unwrap(),
            )
            .unwrap();
            let text = rows(&chart).join(" ");
            assert!(text.contains("Chart data incomplete"), "{text}");
            assert!(text.contains(&format!("height: {height}")), "{text}");
        }
    }
}

// 여러 계열은 공통 Y 축에서 비교하고 번호·범례·원래 수치를 좁은 폭에서도 유지한다.
#[test]
fn named_charts_share_axes_and_keep_numbered_legends_and_values() {
    for kind in ["linechart", "stepchart", "scatterchart"] {
        let data = if kind == "scatterchart" {
            "Low: 0,0 1,0\nHigh: 100,10"
        } else {
            "Low: 0 0\nHigh: 10 10"
        };
        for width in [80, 24, 12] {
            let rendered = prepare(
                &format!("```{kind}\nheight: 4\n{data}\n```"),
                NonZeroU16::new(width).unwrap(),
            )
            .unwrap();
            let text = rows(&rendered).join("\n");
            let flat = text.split_whitespace().collect::<String>();
            assert!(
                flat.contains("[1]Low") && flat.contains("[2]High"),
                "{text}"
            );
            assert!(flat.contains("Y0→10"), "{text}");
            assert!(
                flat.contains(if kind == "scatterchart" {
                    "[1]Low:0,01,0"
                } else {
                    "[1]Low:00"
                }),
                "{text}"
            );
            if width >= 24 {
                let plot = rows(&rendered)
                    .into_iter()
                    .filter(|row| row.contains('│'))
                    .collect::<Vec<_>>();
                assert_eq!(plot.len(), 4);
                assert!(plot[0].split_once('│').unwrap().1.contains('2'), "{text}");
                assert!(plot[3].split_once('│').unwrap().1.contains('1'), "{text}");
                if kind == "scatterchart" {
                    assert!(flat.contains("X0→100"), "{text}");
                    assert!(plot[3].split_once('│').unwrap().1.ends_with(' '), "{text}");
                }
                let first_plot_row = rows(&rendered)
                    .iter()
                    .position(|row| row.contains('│'))
                    .unwrap() as u16;
                assert!(rendered.glyphs.iter().any(|g| g.point.y == first_plot_row
                    && g.grapheme.as_str() == "2"
                    && g.decoration.role == Role::ChartSeries(1)));
                assert!(
                    rendered
                        .glyphs
                        .iter()
                        .any(|g| g.decoration.role == Role::ChartSeries(1))
                );
            } else {
                assert!(flat.contains("Chartnarrowed"), "{text}");
            }
        }
    }
    let shared = prepare(
        "```linechart\nA: 1 2\nB: 1 2\n```",
        NonZeroU16::new(40).unwrap(),
    )
    .unwrap();
    assert!(rows(&shared).join("\n").contains("× plotted overlap"));
    let mixed = prepare(
        "```linechart\nheight: 4\nA: 0 0\nB: 1 1\nC: 10 10\n```",
        NonZeroU16::new(40).unwrap(),
    )
    .unwrap();
    let mixed_rows = rows(&mixed).join("\n");
    assert!(mixed_rows.contains("Mixed cells shown muted"));
    assert!(!mixed_rows.contains('×'));
    let ascii = MarkdownStyles {
        rich_media: false,
        ..MarkdownStyles::plain(Style::default())
    };
    assert_eq!(
        ascii
            .display_glyph(
                Decoration::role(Role::ChartSeries(1)),
                Grapheme::try_from("⠉").unwrap()
            )
            .as_str(),
        "*"
    );
    assert_eq!(
        ascii
            .display_glyph(
                Decoration::role(Role::Chart),
                Grapheme::try_from("×").unwrap()
            )
            .as_str(),
        "x"
    );
}

// 계열·이름·표본 상한의 첫 초과, 중복 이름, 불균일한 선 표본과 비유한 값은 전체 원문으로 돌아간다.
#[test]
fn named_chart_boundaries_reject_ambiguous_or_excess_data() {
    let samples = "1 ".repeat(64);
    let cases = [
        (
            (1..=4)
                .map(|i| format!("S{i}: {samples}"))
                .collect::<Vec<_>>()
                .join("\n"),
            true,
        ),
        (
            (1..=5)
                .map(|i| format!("S{i}: 1 2"))
                .collect::<Vec<_>>()
                .join("\n"),
            false,
        ),
        (format!("{}: 1 2", "n".repeat(64)), true),
        (format!("{}: 1 2", "n".repeat(65)), false),
        (format!("A: {}", "1 ".repeat(65)), false),
        ("A: 1 2\nA: 3 4".into(), false),
        ("A: 1 2\nB: 3".into(), false),
        ("A: NaN 2".into(), false),
        (": 1 2".into(), false),
        ("A: 1e308 -1e308\nB: 0 0".into(), true),
        ("A: 4\nB: 4".into(), true),
    ];
    for (data, valid) in cases {
        let rendered = prepare(
            &format!("```linechart\n{data}\n```"),
            NonZeroU16::new(40).unwrap(),
        )
        .unwrap();
        let text = rows(&rendered)
            .join("\n")
            .split_whitespace()
            .collect::<String>();
        assert_eq!(!text.contains("Chartdataincomplete"), valid, "{data}");
        if !valid {
            assert!(
                text.contains(&data.split_whitespace().collect::<String>()),
                "{data}"
            );
        }
    }
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

// PNG/JPEG EXIF의 회전·반전과 잘못된 값을 원본 픽셀로 검증하고 셀·PNG 방향을 일치시킨다.
#[test]
fn image_orientation_is_applied_to_pixels_dimensions_and_native_png() {
    use ::image::{
        ExtendedColorType, ImageDecoder, ImageEncoder, ImageFormat, ImageReader, Rgb, RgbImage,
        codecs::{jpeg::JpegEncoder, png::PngEncoder},
        load_from_memory,
        metadata::Orientation,
    };
    let pixels = RgbImage::from_fn(3, 2, |x, y| Rgb([(x * 80) as u8, (y * 180) as u8, 70]));
    for format in [ImageFormat::Jpeg, ImageFormat::Png] {
        let mut bytes = Cursor::new(Vec::new());
        pixels.write_to(&mut bytes, format).unwrap();
        let bytes = bytes.into_inner();
        let original = load_from_memory(&bytes).unwrap().to_rgb8();
        for orientation in 0_u16..=9 {
            let mut exif = b"II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
            exif.extend_from_slice(&orientation.to_le_bytes());
            exif.extend_from_slice(&[0; 6]);
            let mut encoded = Vec::new();
            let mime = if format == ImageFormat::Png {
                let mut encoder = PngEncoder::new(&mut encoded);
                encoder.set_exif_metadata(exif).unwrap();
                encoder
                    .write_image(pixels.as_raw(), 3, 2, ExtendedColorType::Rgb8)
                    .unwrap();
                "png"
            } else {
                let mut encoder = JpegEncoder::new(&mut encoded);
                encoder.set_exif_metadata(exif).unwrap();
                encoder.encode_image(&pixels).unwrap();
                "jpeg"
            };
            let source = format!(
                "![Oriented](data:image/{mime};base64,{})",
                STANDARD.encode(&encoded)
            );
            for width in [40, 2, 40] {
                let rendered = prepare(&source, NonZeroU16::new(width).unwrap()).unwrap();
                let raster = rendered.rasters.first().unwrap();
                let corrected = load_from_memory(&raster.png).unwrap().to_rgb8();
                if (2..=8).contains(&orientation) {
                    let mut decoder = ImageReader::with_format(
                        Cursor::new(raster.png.as_ref()),
                        ImageFormat::Png,
                    )
                    .into_decoder()
                    .unwrap();
                    assert_eq!(decoder.orientation().unwrap(), Orientation::NoTransforms);
                } else if format == ImageFormat::Png {
                    assert_eq!(raster.png.as_ref(), encoded.as_slice());
                }
                let swapped = (5..=8).contains(&orientation);
                assert_eq!(
                    corrected.dimensions(),
                    if swapped { (2, 3) } else { (3, 2) }
                );
                for (x, y, pixel) in corrected.enumerate_pixels() {
                    let (sx, sy) = match orientation {
                        2 => (2 - x, y),
                        3 => (2 - x, 1 - y),
                        4 => (x, 1 - y),
                        5 => (y, x),
                        6 => (y, 1 - x),
                        7 => (2 - y, 1 - x),
                        8 => (2 - y, x),
                        _ => (x, y),
                    };
                    assert_eq!(
                        pixel,
                        original.get_pixel(sx, sy),
                        "{format:?} orientation {orientation} at {x},{y}"
                    );
                }
                if width == 40 {
                    let pairs = rendered
                        .glyphs
                        .iter()
                        .filter_map(|glyph| match glyph.decoration.role {
                            Role::Pixel(top, bottom) => Some((top, bottom)),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    let expected = (0..corrected.height())
                        .step_by(2)
                        .flat_map(|y| (0..corrected.width()).map(move |x| (x, y)))
                        .map(|(x, y)| {
                            (
                                corrected.get_pixel(x, y).0,
                                corrected
                                    .get_pixel(x, (y + 1).min(corrected.height() - 1))
                                    .0,
                            )
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(pairs, expected);
                    assert!(rows(&rendered).join(" ").contains(if swapped {
                        "2 × 3"
                    } else {
                        "3 × 2"
                    }));
                }
            }
        }
    }
}
