use super::*;

// 명시적 도구 출력 profile의 PNG·인수는 기본 렌더러에 도달하고 사용자 콜백은 원본 JSON을 받는다.
#[test]
fn structured_tool_output_reaches_native_images_and_custom_renderer() {
    use std::{io::Cursor, num::NonZeroU16};

    use ::image::{ImageFormat, RgbImage};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ToolRenderer};

    let mut encoded = Cursor::new(Vec::new());
    RgbImage::new(16, 8)
        .write_to(&mut encoded, ImageFormat::Png)
        .unwrap();
    let png = encoded.into_inner();
    let data = STANDARD.encode(&png);
    let mut output = ToolOutput {
        tool: "screenshot".to_owned(),
        server: Some("browser".to_owned()),
        arguments: Some("original args".into()),
        result: None,
        content_items: None,
        error: None,
        plain_text: "browser.screenshot\nReadable result".to_owned(),
    };
    output.result = Some(
        format!(r#"{{"content":[{{"type":"image","mimeType":"image/png","data":"{data}"}}]}}"#)
            .parse()
            .unwrap(),
    );
    for (show, dynamic, resource) in [
        (true, false, false),
        (false, false, false),
        (true, true, false),
        (true, false, true),
        (false, false, true),
    ] {
        let mut output = output.clone();
        if dynamic {
            output.result = None;
            output.content_items = Some(
                format!(r#"[{{"type":"inputImage","imageUrl":"data:image/png;base64,{data}"}}]"#)
                    .parse()
                    .unwrap(),
            );
        }
        if resource {
            output.result = Some(
                json!({"content":[{"type":"resource","resource":{"uri":"resource://image","mimeType":"image/png","blob":data}}]}),
            );
        }
        let wire = output.to_snapshot().unwrap();
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(
                OutputPreferences::default()
                    .with_tool_head_rows(u16::MAX)
                    .with_images(show)
                    .with_image_max_width(NonZeroU16::new(4).unwrap()),
            );
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolCall,
            })
            .unwrap();
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(wire.clone()),
            })
            .unwrap();
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(70, 50), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(text.contains("browser.screenshot"), "{text}");
        assert!(text.contains("original args"), "{text}");
        assert!(!text.contains(&data));
        assert_eq!(text.contains("Image display disabled"), !show);
        if show {
            assert_eq!(frame.surface.rasters.len(), 1);
            assert_eq!(&*frame.surface.rasters[0].png, png.as_slice());
            assert_eq!(frame.surface.rasters[0].area.size.width, 4);
        } else {
            assert!(frame.surface.rasters.is_empty());
        }
        let plain = session.session_output().unwrap().unwrap();
        assert!(plain.contains("Readable result"), "{plain}");
        assert!(!plain.contains(ToolOutput::SCHEMA), "{plain}");
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            assert_eq!(input.source, expected.plain_text);
            Some("**Custom typed result**".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(70, 50), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom typed result"));
    }
    // A literal text block containing fences and image syntax must not become media.
    let literal = format!("```\n![literal](data:image/png;base64,{data})\n```");
    let result = output.result.as_mut().unwrap();
    result["content"][0]["type"] = "text".into();
    result["content"][0]["text"] = literal.into();
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX));
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ToolCall,
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
        })
        .unwrap();
    let pin = session.appearance_pin();
    let frame = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(70, 50), &pin)
        .unwrap();
    assert!(frame.surface.rasters.is_empty());
    output.result = Some(r#"{"content":[],"structuredContent":{"ok":true}}"#.parse().unwrap());
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
        })
        .unwrap();
    let frame = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(70, 50), &pin)
        .unwrap();
    let text = visible_rows(&frame.surface);
    assert!(text.contains("(empty content)"), "{text}");
    assert!(text.contains("Structured result"), "{text}");
}
// MCP 리소스 링크의 확인된 필드만 구분하고 미지 필드·원문·잘못된 값은 보존한다.
// 좁은 폭에서도 설명을 Markdown이나 이미지로 실행하지 않고 URI를 자동으로 열지 않는다.
#[test]
fn resource_links_preserve_metadata_literals_and_renderer_input() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ToolRenderer};

    for (size, rich) in [
        (json!(0), true),
        (json!(u64::MAX), true),
        (json!(-1), false),
        (json!(1.5), false),
        (json!("20"), false),
    ] {
        let block = json!({"type":"resource_link","name":"report.txt","title":"Build report",
            "uri":"file:///unopened/report.txt","description":"**literal**\n![image](file.png)",
            "mimeType":"text/plain","size":size,"annotations":{"audience":["user"]},"future":"retained"});
        let output = ToolOutput {
            tool: "resources".to_owned(),
            server: Some("any-provider".to_owned()),
            arguments: None,
            result: Some(json!({"content":[block]})),
            content_items: None,
            error: None,
            plain_text: "Original resource link receipt".to_owned(),
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
            assert_eq!(joined.contains("Resourcelink"), rich, "{text}");
            assert!(joined.contains("file:///unopened/report.txt"), "{text}");
            assert!(joined.contains("**literal**"), "{text}");
            assert!(joined.contains("![image](file.png)"), "{text}");
            assert!(
                joined.contains("future")
                    && joined.contains("retained")
                    && joined.contains("audience"),
                "{text}"
            );
            assert!(frame.surface.rasters.is_empty());
            assert!(!frame.surface.has_hyperlinks());
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Custom resource card".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 30), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom resource card"));
    }
}
// 리소스 카드의 JSON 256 KiB 경계와 첫 초과는 각각 카드와 전체 리터럴 fallback을 선택한다.
#[test]
fn resource_link_card_limit_preserves_the_first_excess_byte() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::OutputPreferences;

    for (bytes, rich) in [(256 * 1024, true), (256 * 1024 + 1, false)] {
        let mut block =
            json!({"type":"resource_link","name":"a","uri":"resource://a","description":""});
        let padding = bytes - block.to_string().len();
        block["description"] = "x".repeat(padding).into();
        assert_eq!(block.to_string().len(), bytes);
        let output = ToolOutput {
            tool: "resources".to_owned(),
            server: None,
            arguments: None,
            result: Some(json!({"content":[block]})),
            content_items: None,
            error: None,
            plain_text: format!("Original {bytes}-byte resource receipt"),
        };
        let mut session = TuiSession::new(ColorCapability::Unknown, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(12));
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
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 40), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert_eq!(text.contains("Resource link"), rich, "{text}");
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
}
// 내장 리소스는 URI·MIME·코드와 내외부 메타데이터를 보존하며 모호한 본문은 JSON으로 유지한다.
#[test]
fn embedded_resources_keep_outer_and_inner_metadata_across_resize() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ToolRenderer};

    for (resource, rich) in [
        (
            json!({"uri":"resource://main.rs","mimeType":"text/x-rust","text":"fn main() {}\n![literal](file.png)","_meta":{"revision":7}}),
            true,
        ),
        (
            json!({"uri":"resource://data","mimeType":"application/octet-stream","blob":"AA==","_meta":{"revision":7}}),
            true,
        ),
        (
            json!({"uri":"resource://data","mimeType":12,"text":"literal","_meta":{"revision":7}}),
            false,
        ),
        (
            json!({"uri":"resource://data","text":"literal","blob":"AA==","_meta":{"revision":7}}),
            false,
        ),
        (
            json!({"uri":"","text":"literal","_meta":{"revision":7}}),
            false,
        ),
    ] {
        let output = ToolOutput {
            tool: "lookup".to_owned(),
            server: Some("generic".to_owned()),
            arguments: None,
            result: Some(
                json!({"content":[{"type":"resource","resource":resource,"annotations":{"audience":["user"]},"future":"retained"}]}),
            ),
            content_items: None,
            error: None,
            plain_text: "Original embedded resource".to_owned(),
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
            assert_eq!(joined.contains("Resourcemetadata"), rich, "{text}");
            assert!(
                joined.contains("revision")
                    && joined.contains("audience")
                    && joined.contains("future")
                    && joined.contains("retained"),
                "{text}"
            );
            if rich && resource.get("text").is_some() {
                assert!(joined.contains("fnmain(){}"), "{text}");
                assert!(joined.contains("![literal](file.png)"), "{text}");
                assert!(joined.contains("Type:text/x-rust"), "{text}");
            }
            if rich && resource.get("blob").is_some() {
                assert!(joined.contains("Binaryresource"), "{text}");
            }
            assert!(frame.surface.rasters.is_empty());
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Custom embedded resource".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 30), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom embedded resource"));
    }
}
