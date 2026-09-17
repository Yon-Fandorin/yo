use super::*;

// 접기 구간과 겹친 native raster는 제거하고 아래쪽에 완전히 남은 raster는 셀과 함께 이동한다.
#[test]
fn tool_renderer_folding_keeps_native_images_aligned() {
    use std::{io::Cursor, num::NonZeroU16};

    use ::image::{ImageFormat, RgbImage};
    use base64::{Engine, engine::general_purpose::STANDARD};

    use crate::{OutputPreferences, ToolRenderer};

    let mut encoded = Cursor::new(Vec::new());
    RgbImage::new(8, 4)
        .write_to(&mut encoded, ImageFormat::Png)
        .unwrap();
    let image = format!(
        "![Image](data:image/png;base64,{})",
        STANDARD.encode(encoded.into_inner())
    );
    for at_end in [true, false] {
        let rows = "row\n\n".repeat(20);
        let markdown = if at_end {
            format!("{rows}{image}")
        } else {
            format!("{image}\n\n{rows}")
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_tool_renderer(Some(ToolRenderer::new(move |_| Some(markdown.clone()))))
            .with_output_preferences(
                OutputPreferences::default()
                    .with_tool_head_rows(u16::MAX)
                    .with_image_max_width(NonZeroU16::new(8).unwrap()),
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
                update: ActivityUpdate::TextSnapshot("payload".to_owned()),
            })
            .unwrap();
        let pin = session.appearance_pin();
        let full = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 70), &pin)
            .unwrap();
        assert_eq!(full.surface.rasters.len(), 1);
        let original_y = full.surface.rasters[0].area.origin.y;
        session = session.with_output_preferences(
            OutputPreferences::default()
                .with_tool_head_rows(0)
                .with_image_max_width(NonZeroU16::new(8).unwrap()),
        );
        let pin = session.appearance_pin();
        let folded = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 70), &pin)
            .unwrap();
        assert!(visible_rows(&folded.surface).contains("rows hidden"));
        if at_end {
            assert_eq!(folded.surface.rasters.len(), 1);
            assert!(folded.surface.rasters[0].area.origin.y < original_y);
            assert_eq!(folded.surface.rasters[0].png, full.surface.rasters[0].png);
        } else {
            assert!(folded.surface.rasters.is_empty());
        }
    }
}
// 메시지 이미지도 공통 표시 설정·폭 변경·커스텀 렌더러를 따르며 원본과 실패 문구를 보존한다.
#[test]
fn message_content_images_reflow_customize_and_preserve_source() {
    use std::io::Cursor;

    use ::image::{ImageFormat, RgbImage};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde_json::json;
    use yo_core::{ActivityOutcome, Failure, MessageContent};

    use crate::{AssistantRenderer, OutputPreferences};

    let mut png = Cursor::new(Vec::new());
    RgbImage::new(4, 2)
        .write_to(&mut png, ImageFormat::Png)
        .unwrap();
    let block = json!({"type":"image","mimeType":"image/png","data":STANDARD.encode(png.get_ref()),"_meta":{"origin":"fixture"}});
    let source = MessageContent {
        block: block.clone(),
    }
    .to_snapshot()
    .unwrap();
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced);
    let state = session.parts_mut().state;
    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::AgentMessage,
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(source),
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityFinished {
            activity: activity(1),
            outcome: ActivityOutcome::Failed(Failure::new("delivery stopped")),
        })
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    assert!(original.contains("fixture"));
    assert!(original.contains(block["data"].as_str().unwrap()));
    assert!(original.contains("delivery stopped"));
    assert!(!original.contains(MessageContent::SCHEMA));
    for visible in [true, false, true] {
        session =
            session.with_output_preferences(OutputPreferences::default().with_images(visible));
        for width in [80, 12, 80] {
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 30), &pin)
                .unwrap();
            assert_eq!(!frame.surface.rasters.is_empty(), visible);
            if visible {
                assert_eq!(frame.surface.rasters[0].png.as_ref(), png.get_ref());
            }
            let rendered = visible_rows(&frame.surface)
                .split_whitespace()
                .collect::<String>();
            assert!(rendered.contains("deliverystopped"), "{rendered}");
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
    }
    let expected = block;
    session = session.with_assistant_renderer(Some(AssistantRenderer::new(move |input| {
        assert_eq!(
            MessageContent::from_snapshot(input.source).unwrap().block,
            expected
        );
        Some("Custom media answer".into())
    })));
    let pin = session.appearance_pin();
    let frame = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(80, 30), &pin)
        .unwrap();
    assert!(visible_rows(&frame.surface).contains("Custom media answer"));
    assert!(frame.surface.rasters.is_empty());
    assert_eq!(session.session_output().unwrap().unwrap(), original);
}
// 답변 리소스·오디오·알 수 없는 블록은 공통 표시와 원본 보존을 함께 유지한다.
#[test]
fn message_content_resources_and_unknown_blocks_remain_observable() {
    use serde_json::json;
    use yo_core::{ActivityOutcome, MessageContent};
    for (block, marker) in [
        (
            json!({"type":"resource","resource":{"uri":"resource://main.rs","mimeType":"text/x-rust","text":"fn main() {}"}}),
            "fnmain(){}",
        ),
        (
            json!({"type":"resource_link","uri":"resource://guide","name":"Guide"}),
            "Guide",
        ),
        (
            json!({"type":"audio","mimeType":"audio/wav","data":"AA=="}),
            "playbackunavailable",
        ),
        (json!({"type":"future","payload":"RETAINED"}), "RETAINED"),
    ] {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced);
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::AgentMessage,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(
                    MessageContent {
                        block: block.clone(),
                    }
                    .to_snapshot()
                    .unwrap(),
                ),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: ActivityOutcome::Completed,
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        for width in [80, 12, 80] {
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 40), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface)
                .split_whitespace()
                .collect::<String>();
            assert!(text.contains(marker), "{text}");
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        assert!(original.contains(block["type"].as_str().unwrap()));
    }
}
// 불투명한 검색 결과는 폭 변경 뒤에도 원문으로 표시하며 콜백·내보내기에 전체 값을 전달한다.
#[test]
fn opaque_web_results_remain_visible_and_customizable() {
    use serde_json::json;
    use yo_core::{ActivityOutcome, ToolOutput};

    use crate::{OutputPreferences, ToolRenderer};
    let results = json!([
        {"title":"![literal](data:image/png;base64,AAAA)","url":"https://example.com/source","future":"RETAINED"},
        {"type":"new-result","payload":"SECOND"},
    ]);
    let output = ToolOutput {
        tool: "webSearch".into(),
        server: None,
        arguments: Some(json!({"query":"sources"})),
        result: Some(json!({"results":results})),
        content_items: None,
        error: None,
        plain_text: format!("Web search\nQuery: sources\nResults:\n{results:#}"),
    };
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX));
    let state = session.parts_mut().state;
    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ToolCall,
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityFinished {
            activity: activity(1),
            outcome: ActivityOutcome::Completed,
        })
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    assert!(original.contains("RETAINED") && original.contains("SECOND"));
    for width in [80, 24, 80] {
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 70), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface)
            .split_whitespace()
            .collect::<String>();
        for token in [
            "RETAINED",
            "SECOND",
            "https://example.com/source",
            "![literal]",
        ] {
            assert!(text.contains(token), "{text}");
        }
        assert!(frame.surface.rasters.is_empty());
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
    session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
        assert_eq!(input.output, Some(&output));
        Some("Custom search results".into())
    })));
    let pin = session.appearance_pin();
    let frame = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(80, 30), &pin)
        .unwrap();
    assert!(visible_rows(&frame.surface).contains("Custom search results"));
    assert_eq!(session.session_output().unwrap().unwrap(), original);
}
