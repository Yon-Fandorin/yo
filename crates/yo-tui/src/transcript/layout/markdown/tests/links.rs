use super::support::*;

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
