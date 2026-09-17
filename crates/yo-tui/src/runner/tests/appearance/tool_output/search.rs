use super::*;

// 파일 검색은 패턴·결과·보고된 제한을 구분하되 잘못된 메타데이터와 원본은 유지한다.
// 좁은 폭에서도 파일 이름을 Markdown으로 실행하지 않고 사용자 renderer가 원본을 받는다.
#[test]
fn file_search_preserves_literals_limits_and_custom_renderer_across_resize() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ToolRenderer};

    let source = "src/main.rs\n![literal](file.png)";
    for (details, partial) in [
        (json!({"resultLimitReached": 1}), true),
        (
            json!({"resultLimitReached": 2, "truncation": {
                "content": source, "truncated": true, "truncatedBy": "lines",
                "totalLines": 5, "totalBytes": 500, "outputLines": 2, "outputBytes": source.len(),
                "lastLinePartial": false, "firstLineExceedsLimit": false, "maxLines": 2, "maxBytes": 1024
            }}),
            true,
        ),
        (json!({"resultLimitReached": u64::MAX}), true),
        (json!({"resultLimitReached": 0}), false),
        (json!({"resultLimitReached": -1}), false),
        (json!({"resultLimitReached": 1.5}), false),
        (json!({"resultLimitReached": "2"}), false),
        (json!({"resultLimitReached": 2, "future": true}), false),
        (json!({"resultLimitReached": 2, "truncation": {}}), false),
    ] {
        let output = ToolOutput {
            tool: "find".to_owned(),
            server: None,
            arguments: Some(json!({"pattern": "**/*.rs", "path": ".", "limit": 2})),
            result: Some(
                json!({"content": [{"type": "text", "text": source}], "details": details}),
            ),
            content_items: None,
            error: None,
            plain_text: source.to_owned(),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX));
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolResult,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        for width in [80, 24, 80] {
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 90), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text.split_whitespace().collect::<String>();
            assert!(joined.contains("Filesearchpattern"), "{text}");
            assert!(joined.contains("**/*.rs"), "{text}");
            assert!(joined.contains("Matchingpaths"), "{text}");
            assert!(joined.contains("![literal](file.png)"), "{text}");
            assert_eq!(joined.contains("Partialfilesearch"), partial, "{text}");
            assert_eq!(joined.contains("resultLimitReached"), !partial, "{text}");
            assert!(frame.surface.rasters.is_empty());
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Custom file search".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 30), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom file search"));
    }
}
// 내용 검색은 실제 match 제한과 행 잘림만 안내하고 오류·미지 메타데이터는 원문으로 남긴다.
// colon·Markdown 문자가 있는 결과를 파일 위치로 추측하지 않고 좁은 폭과 사용자 렌더러를 보존한다.
#[test]
fn content_search_renders_reported_limits_without_interpreting_result_lines() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ToolRenderer};

    for (details, failed, limit, lines, fallback) in [
        (
            json!({"matchLimitReached": 2, "linesTruncated": true}),
            false,
            true,
            true,
            false,
        ),
        (json!({"linesTruncated": false}), false, false, false, false),
        (json!({"matchLimitReached": 1}), false, true, false, false),
        (
            json!({"matchLimitReached": u64::MAX}),
            false,
            true,
            false,
            false,
        ),
        (json!({"matchLimitReached": 0}), false, false, false, true),
        (json!({"matchLimitReached": -1}), false, false, false, true),
        (json!({"matchLimitReached": 1.5}), false, false, false, true),
        (json!({"linesTruncated": "yes"}), false, false, false, true),
        (
            json!({"matchLimitReached": 2, "future": 1}),
            false,
            false,
            false,
            true,
        ),
        (
            json!({"matchLimitReached": 2, "linesTruncated": true}),
            true,
            false,
            false,
            true,
        ),
    ] {
        let source = "src/a:b.rs:12: **literal** fn main()\nsrc/a:b.rs-13- ![literal](file.png)";
        let output = ToolOutput {
            tool: "grep".to_owned(),
            server: None,
            arguments: Some(
                json!({"pattern": "fn main", "path": "src", "glob": "*.rs", "ignoreCase": true}),
            ),
            result: Some(
                json!({"content": [{"type":"text","text": source}], "details": details, "isError": failed}),
            ),
            content_items: None,
            error: None,
            plain_text: source.to_owned(),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX));
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolResult,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        for width in [80, 24, 80] {
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 100), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text.split_whitespace().collect::<String>();
            assert!(joined.contains("Contentsearchpattern"), "{text}");
            assert!(joined.contains("ignoreCase"), "{text}");
            assert!(
                joined.contains("src/a:b.rs:12:**literal**fnmain()"),
                "{text}"
            );
            assert!(joined.contains("![literal](file.png)"), "{text}");
            assert_eq!(joined.contains("Searchresults"), !failed, "{text}");
            assert_eq!(joined.contains("Matchlimitreached:"), limit, "{text}");
            assert_eq!(joined.contains("Partialsearchlines"), lines, "{text}");
            assert_eq!(
                joined.contains("matchLimitReached") || joined.contains("linesTruncated"),
                fallback,
                "{text}"
            );
            assert!(frame.surface.rasters.is_empty());
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Custom content search".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 30), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom content search"));
    }
}
