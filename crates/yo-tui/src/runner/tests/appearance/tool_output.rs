use super::support::*;

// 호스트 도구 본문은 실제 코드·표·이미지 프레임으로 표시되며 실패 상태·원문·표시 설정을 보존한다.
#[test]
fn tool_renderer_uses_rich_layout_without_replacing_status_or_source() {
    use std::{io::Cursor, num::NonZeroU16};

    use ::image::{ImageFormat, RgbImage};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use yo_core::{ActivityOutcome, Failure};

    use crate::{OutputPreferences, Theme, ToolRenderer};

    let mut encoded = Cursor::new(Vec::new());
    RgbImage::new(32, 16)
        .write_to(&mut encoded, ImageFormat::Png)
        .unwrap();
    let png = encoded.into_inner();
    let markdown = format!(
        "```rust\nlet value = 42;\n```\n\n| Key | Value |\n| --- | --- |\n| 상태 | ready |\n\n![Tool image](data:image/png;base64,{})",
        STANDARD.encode(&png)
    );
    for show_images in [true, false] {
        let renderer = ToolRenderer::new({
            let markdown = markdown.clone();
            move |input| {
                assert_eq!(input.kind, ActivityKind::ToolCall);
                assert_eq!(input.source, "docs.search\noriginal payload");
                assert_eq!(input.columns.get(), 30);
                Some(markdown.clone())
            }
        });
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_tool_renderer(Some(renderer))
            .with_output_preferences(
                OutputPreferences::default()
                    .with_max_body_width(NonZeroU16::new(30))
                    .with_tool_head_rows(u16::MAX)
                    .with_images(show_images)
                    .with_image_max_width(NonZeroU16::new(4).unwrap()),
            )
            .with_theme(Theme::Light);
        let pin = session.appearance_pin();
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
                update: ActivityUpdate::TextSnapshot("docs.search\noriginal payload".to_owned()),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: ActivityOutcome::Failed(Failure::new("VISIBLE FAILURE")),
            })
            .unwrap();
        let frame = state.prepare_frame(Size::new(60, 50), &pin).unwrap();
        let text = visible_rows(&frame.surface);
        for expected in [
            "Tool failed",
            "VISIBLE FAILURE",
            "let value = 42;",
            "ready",
            "Tool image",
        ] {
            assert!(text.contains(expected), "{expected}: {text}");
        }
        assert!(!text.contains("original payload"));
        assert_eq!(text.contains("Image display disabled"), !show_images);
        if show_images {
            assert_eq!(frame.surface.rasters.len(), 1);
            assert_eq!(frame.surface.rasters[0].area.size.width, 4);
            assert_eq!(&*frame.surface.rasters[0].png, png.as_slice());
        } else {
            assert!(frame.surface.rasters.is_empty());
        }
        let plain = session.session_output().unwrap().unwrap();
        assert!(plain.contains("original payload"));
        assert!(plain.contains("VISIBLE FAILURE"));
        assert!(!plain.contains("let value = 42;"));
        session = session.with_tool_renderer(None);
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(60, 50), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("original payload"));
        assert!(frame.surface.rasters.is_empty());
    }
}

// 적용 거절·높이 초과는 원문으로 돌아가며 새 렌더러 설치는 이미 캐시된 레이아웃을 갱신한다.
#[test]
fn tool_renderer_fallback_and_replacement_invalidate_layout() {
    use crate::ToolRenderer;

    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced);
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ToolResult,
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot("original payload".to_owned()),
        })
        .unwrap();
    for (renderer, expected) in [
        (
            ToolRenderer::new(|_| Some("replacement one".to_owned())),
            "replacement one",
        ),
        (
            ToolRenderer::new(|_| Some("replacement two".to_owned())),
            "replacement two",
        ),
        (ToolRenderer::new(|_| None), "original payload"),
        (
            ToolRenderer::new(|_| Some("\u{301}".into())),
            "original payload",
        ),
    ] {
        session = session.with_tool_renderer(Some(renderer));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(60, 20), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(text.contains(expected), "{text}");
        assert!(text.contains("Tool result"), "{text}");
    }
}

// 도구 확장 콜백은 일반 답변·추론·파일 diff를 가로채지 않는다.
#[test]
fn tool_renderer_does_not_intercept_other_activity_kinds() {
    use crate::ToolRenderer;

    for kind in [
        ActivityKind::AgentMessage,
        ActivityKind::ModelWork,
        ActivityKind::FileChange,
    ] {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_tool_renderer(Some(ToolRenderer::new(|_| {
                panic!("non-tool must not reach renderer")
            })));
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind,
            })
            .unwrap();
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot("literal payload".to_owned()),
            })
            .unwrap();
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(60, 20), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("literal payload"));
    }
}

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

// 남는다.
#[test]
fn file_tool_output_highlights_only_known_successful_source() {
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole, ToolRenderer};

    for (tool, resource, failed, expected_code) in [
        ("read", false, false, true),
        ("read_file", false, false, true),
        ("lookup", true, false, true),
        ("lookup", false, false, false),
        ("read", false, true, false),
    ] {
        let source = "fn main() { println!(\"hello\"); }\n```\n![literal](ignored.png)";
        let mut output = ToolOutput {
            tool: tool.to_owned(), server: Some("files".to_owned()),
            arguments: Some(r#"{"path":"src/main.rs","offset":7,"limit":3}"#.parse().unwrap()),
            result: Some(if resource {
                r#"{"content":[{"type":"resource","resource":{"uri":"file:///src/main.rs","text":""}}],"isError":false}"#
            } else {
                r#"{"content":[{"type":"text","text":""}],"isError":false}"#
            }.parse().unwrap()),
            content_items: None, error: None,
            plain_text: format!("files.{tool}\nsrc/main.rs\n{source}"),
        };
        let result = output.result.as_mut().unwrap();
        result["isError"] = failed.into();
        if resource {
            result["content"][0]["resource"]["text"] = source.into();
        } else {
            result["content"][0]["text"] = source.into();
        }
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX))
            .with_theme_overrides(
                ThemeOverrides::default()
                    .with_color(ThemeRole::SyntaxKeyword, ThemeColor::Rgb(17, 211, 119)),
            );
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
        let original = session.session_output().unwrap().unwrap();
        for line in source.lines() {
            assert!(original.contains(line), "{original}");
        }
        let pin = session.appearance_pin();
        for width in [70, 24, 70] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 65), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            assert!(
                text.chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
                    .contains("src/main.rs"),
                "{text}"
            );
            assert!(text.contains("![literal]"), "{text}");
            assert!(text.contains("```"), "{text}");
            let (y, row) = text
                .lines()
                .enumerate()
                .find(|(_, line)| line.contains("fn main()"))
                .unwrap();
            let x = row.find("fn main()").unwrap();
            let color = frame
                .surface
                .cell(Point::new(x as u16, y as u16))
                .unwrap()
                .style()
                .foreground;
            assert_eq!(
                color
                    == Color::Rgb {
                        red: 17,
                        green: 211,
                        blue: 119
                    },
                expected_code,
                "{text}"
            );
            assert!(frame.surface.rasters.is_empty());
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        session = session.with_tool_renderer(Some(ToolRenderer::new(|_| {
            Some("Custom file view".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(70, 30), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom file view"));
    }
}

// 쓰기 제안은 인수 JSON과 분리해 코드 색상을 적용하며 빈 파일·오류·잘못된 인수도 원문과 구분을
// 유지한다.
#[test]
fn proposed_file_content_is_highlighted_without_claiming_execution() {
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole, ToolRenderer};

    for (content, failed) in [
        (Some("fn main() {}\n```\n![literal](file.png)"), false),
        (Some("fn main() {}"), true),
        (Some(""), false),
        (None, false),
    ] {
        let mut output = ToolOutput {
            tool: "write_file".to_owned(),
            server: None,
            arguments: Some(
                r#"{"path":"src/main.rs","content":"","create":true}"#
                    .parse()
                    .unwrap(),
            ),
            result: None,
            content_items: None,
            error: None,
            plain_text: format!("write_file\nsrc/main.rs\n{}", content.unwrap_or("7")),
        };
        output.arguments.as_mut().unwrap()["content"] =
            content.map_or_else(|| 7.into(), Into::into);
        if failed {
            output.error = Some("permission denied".into());
        }
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX))
            .with_theme_overrides(
                ThemeOverrides::default()
                    .with_color(ThemeRole::SyntaxKeyword, ThemeColor::Rgb(19, 212, 117)),
            );
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
        let original = session.session_output().unwrap().unwrap();
        let pin = session.appearance_pin();
        for width in [70, 24, 70] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 65), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            assert_eq!(
                joined.contains("Proposedfilecontent"),
                content.is_some(),
                "{text}"
            );
            assert!(!text.contains("Tool completed"), "{text}");
            assert!(frame.surface.rasters.is_empty());
            if let Some(content) = content {
                assert!(!text.contains("\"content\""), "{text}");
                if content.is_empty() {
                    assert!(text.contains("(empty file)"), "{text}");
                } else {
                    assert_eq!(text.matches("fn main()").count(), 1, "{text}");
                    let (y, row) = text
                        .lines()
                        .enumerate()
                        .find(|(_, line)| line.contains("fn main()"))
                        .unwrap();
                    let x = row.find("fn main()").unwrap();
                    assert_eq!(
                        frame
                            .surface
                            .cell(Point::new(x as u16, y as u16))
                            .unwrap()
                            .style()
                            .foreground,
                        Color::Rgb {
                            red: 19,
                            green: 212,
                            blue: 117
                        }
                    );
                }
            } else {
                assert!(text.contains("\"content\": 7"), "{text}");
            }
            if failed {
                assert!(text.contains("permission denied"), "{text}");
            }
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        if !failed {
            use yo_core::ActivityOutcome;
            session
                .parts_mut()
                .state
                .observe(AgentEvent::ActivityFinished {
                    activity: activity(1),
                    outcome: ActivityOutcome::Completed,
                })
                .unwrap();
            assert!(
                session
                    .session_output()
                    .unwrap()
                    .unwrap()
                    .contains("Tool call prepared")
            );
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(70, 60), &pin)
                .unwrap();
            assert!(visible_rows(&frame.surface).contains("Tool call prepared"));
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Custom write view".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(70, 30), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom write view"));
    }
}

// 여러 파일 읽기는 원래 순서·실제 줄 범위·개별 오류를 보존하며 손상·확장된 결과는 통째로 원문
// 표시한다.
#[test]
fn batch_read_panels_preserve_ranges_errors_and_source() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole, ToolRenderer};

    let good = json!({"results":[
        {"path":"src/main.rs","status":"ok","start":2,"end":3,"total":8,"next_offset":4,"content":"fn main() {\n}\n"},
        {"path":"missing.txt","status":"error","error":"unavailable"},
        {"path":"empty.txt","status":"ok","start":0,"end":0,"total":0,"content":""}
    ]});
    let mut unknown = good.clone();
    unknown["results"][0]["unknown"] = "retained".into();
    let mut invalid_range = good.clone();
    invalid_range["results"][0]["next_offset"] = 5.into();
    let mut invalid_content = good.clone();
    invalid_content["results"][0]["content"] = "only one line".into();
    let encoded = good.to_string();
    let at_limit = format!("{}{}", " ".repeat(256 * 1024 - encoded.len()), encoded);
    let over_limit = format!(" {at_limit}");
    let mut eight = good.clone();
    for _ in 0..5 {
        eight["results"]
            .as_array_mut()
            .unwrap()
            .push(good["results"][1].clone());
    }
    let mut nine = eight.clone();
    nine["results"]
        .as_array_mut()
        .unwrap()
        .push(good["results"][1].clone());
    for (source, rich) in [
        (good.to_string(), true),
        (at_limit, true),
        (over_limit, false),
        (eight.to_string(), true),
        (nine.to_string(), false),
        (unknown.to_string(), false),
        (invalid_range.to_string(), false),
        (invalid_content.to_string(), false),
        (
            "{\"results\":[\n[yo: tool output truncated]".to_owned(),
            false,
        ),
    ] {
        let output = ToolOutput {
            tool: "read_files".to_owned(),
            server: None,
            arguments: Some(
                json!({"files":[{"path":"src/main.rs"},{"path":"missing.txt"},{"path":"empty.txt"}]}),
            ),
            result: Some(json!({"content":[{"type":"text","text":source}]})),
            content_items: None,
            error: None,
            plain_text: format!("read_files\n{source}"),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX))
            .with_theme_overrides(
                ThemeOverrides::default()
                    .with_color(ThemeRole::SyntaxKeyword, ThemeColor::Rgb(18, 213, 118)),
            );
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
        let exported = original
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        let content = source
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        assert!(exported.contains(&content));
        let pin = session.appearance_pin();
        for width in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 110), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            assert_eq!(joined.contains("Lines2–3of8"), rich, "{text}");
            if rich {
                assert!(joined.contains("Continuereadingatline4"), "{text}");
                assert!(joined.contains("Readfailed·missing.txt"), "{text}");
                assert!(text.contains("unavailable"), "{text}");
                assert!(text.contains("(empty file)"), "{text}");
                let (y, row) = text
                    .lines()
                    .enumerate()
                    .find(|(_, line)| line.contains("fn main()"))
                    .unwrap();
                let x = row.find("fn main()").unwrap();
                assert_eq!(
                    frame
                        .surface
                        .cell(Point::new(x as u16, y as u16))
                        .unwrap()
                        .style()
                        .foreground,
                    Color::Rgb {
                        red: 18,
                        green: 213,
                        blue: 118
                    }
                );
                assert!(text.find("fn main()").unwrap() < text.find("unavailable").unwrap());
                assert!(text.find("unavailable").unwrap() < text.find("(empty file)").unwrap());
            } else {
                assert!(joined.contains("\"results\":"), "{text}");
            }
            assert!(frame.surface.rasters.is_empty());
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Custom batch view".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 35), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom batch view"));
    }
}

// 교체 제안의 삭제·추가 색상과 개행 차이를 유지하고 실제 적용 개수·실패·원본 인수를 별도로
// 보존한다.
#[test]
fn edit_proposals_keep_diff_styles_and_reported_results_separate() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole, ToolRenderer};

    for (valid, failed) in [(true, false), (true, true), (false, false)] {
        let mut args = json!({"path":"src/main.rs","edits":[
            {"oldText":"let old = 1;","newText":"let new = 2;\n"},
            {"oldText":"remove me\n","newText":""}
        ]});
        if !valid {
            args["edits"][0]["extra"] = "retained".into();
        }
        let source = r#"{"path":"src/main.rs","status":"ok","replacements":2}"#;
        let output = ToolOutput {
            tool: "edit_file".to_owned(),
            server: None,
            arguments: Some(args),
            result: Some(json!({"content":[{"type":"text","text":source}],"isError":failed})),
            content_items: None,
            error: failed.then(|| "permission denied".into()),
            plain_text: format!("edit_file\nlet old = 1;\nlet new = 2;\n{source}"),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX))
            .with_theme_overrides(
                ThemeOverrides::default()
                    .with_color(ThemeRole::DiffAdded, ThemeColor::Rgb(20, 214, 120))
                    .with_color(ThemeRole::DiffRemoved, ThemeColor::Rgb(215, 21, 120)),
            );
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
        let original = session.session_output().unwrap().unwrap();
        assert!(original.contains(source));
        let pin = session.appearance_pin();
        for width in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 100), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            assert_eq!(joined.contains("Proposedreplacements"), valid, "{text}");
            assert_eq!(joined.contains("Appliedreplacements:2"), !failed, "{text}");
            if valid {
                assert!(!text.contains("\"oldText\""), "{text}");
                assert!(joined.contains("Oldtexthasnotrailingnewline"), "{text}");
                assert!(!joined.contains("Newtexthasnotrailingnewline"), "{text}");
                assert!(text.contains("-remove me"), "{text}");
                for (needle, color) in [
                    (
                        "-let old",
                        Color::Rgb {
                            red: 215,
                            green: 21,
                            blue: 120,
                        },
                    ),
                    (
                        "+let new",
                        Color::Rgb {
                            red: 20,
                            green: 214,
                            blue: 120,
                        },
                    ),
                ] {
                    let (y, row) = text
                        .lines()
                        .enumerate()
                        .find(|(_, line)| line.contains(needle))
                        .unwrap();
                    let x = row.find(needle).unwrap();
                    assert_eq!(
                        frame
                            .surface
                            .cell(Point::new(x as u16, y as u16))
                            .unwrap()
                            .style()
                            .foreground,
                        color
                    );
                }
            } else {
                assert!(text.contains("\"extra\""), "{text}");
                assert!(joined.contains("retained"), "{text}");
            }
            assert!(!text.contains("@@ -"), "{text}");
            assert!(frame.surface.rasters.is_empty());
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Custom edit view".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 30), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom edit view"));
    }
}

// 교체 수·본문 바이트 상한과 첫 초과 값에서 제안 표시 여부를 구분하고 단일 교체 형식도 지원한다.
#[test]
fn edit_proposal_limits_preserve_literal_fallbacks() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::OutputPreferences;

    let replacement = json!({"oldText":"old\n","newText":"new\n"});
    for (args, marker, rich) in [
        (
            json!({"path":"file.rs","edits":vec![replacement.clone();256]}),
            "Replacement 256",
            true,
        ),
        (
            json!({"path":"file.rs","edits":vec![replacement;257]}),
            "Replacement 257",
            false,
        ),
        (
            json!({"path":"file.rs","oldText":"x".repeat(256*1024),"newText":""}),
            "Old text has no trailing newline",
            true,
        ),
        (
            json!({"path":"file.rs","oldText":"x".repeat(256*1024+1),"newText":""}),
            "Old text has no trailing newline",
            false,
        ),
    ] {
        let output = ToolOutput {
            tool: "edit_file".to_owned(),
            server: None,
            arguments: Some(args.clone()),
            result: None,
            content_items: None,
            error: None,
            plain_text: args.to_string(),
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
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 40), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert_eq!(text.contains(marker), rich, "expected {marker}: {text}");
        assert!(frame.surface.rasters.is_empty());
    }
}

// 성공한 내장 수정·쓰기 결과만 개수로 요약하고 잘못되거나 확장된 상태는 원문으로 남긴다.
#[test]
fn mutation_result_summaries_require_exact_success_payloads() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::OutputPreferences;

    let mut at_limit = json!({"path":"","status":"ok","bytes":0});
    at_limit["path"] = "p".repeat(16 * 1024 - at_limit.to_string().len()).into();
    let mut over_limit = at_limit.clone();
    over_limit["path"] = format!("{}p", at_limit["path"].as_str().unwrap()).into();
    for (tool, result, expected) in [
        ("write_file", at_limit, Some("Written bytes: 0")),
        ("write_file", over_limit, None),
        (
            "edit_file",
            json!({"path":"file.rs","status":"ok","replacements":2}),
            Some("Applied replacements: 2"),
        ),
        (
            "write_file",
            json!({"path":"file.rs","status":"ok","bytes":0}),
            Some("Written bytes: 0"),
        ),
        (
            "edit_file",
            json!({"path":"file.rs","status":"error","replacements":2}),
            None,
        ),
        (
            "edit_file",
            json!({"path":"file.rs","status":"ok","replacements":-1}),
            None,
        ),
        (
            "edit_file",
            json!({"path":"file.rs","status":"ok","replacements":2,"extra":"keep"}),
            None,
        ),
    ] {
        let source = result.to_string();
        let output = ToolOutput {
            tool: tool.to_owned(),
            server: None,
            arguments: None,
            result: Some(json!({"content":[{"type":"text","text":source}]})),
            content_items: None,
            error: None,
            plain_text: source.clone(),
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
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(90, 35), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        if let Some(expected) = expected {
            assert!(text.contains(expected), "{text}");
        } else {
            assert!(!text.contains("Applied replacements"), "{text}");
            assert!(text.contains("\"status\""), "{text}");
        }
        assert!(session.session_output().unwrap().unwrap().contains(&source));
    }
}

// 명령·실제 스트림·종료 상태를 분리하되 모호하거나 잘린 출력은 원문으로 보존하고 콜백 우선권을
// 유지한다.
#[test]
fn shell_command_and_streams_preserve_literal_boundaries() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole, ToolRenderer};

    for (source, parsed) in [
        (
            "status: 0\nstdout:\nhello\n```\n![literal](file.png)\n\u{1b}[31m\nstderr:\nwarning",
            true,
        ),
        ("status: signal\nstdout:\n\nstderr:\n", true),
        ("status: 1\nstdout:\nfailure\nstderr:\nerror", true),
        (
            "status: 0\nstdout:\nfake\nstderr:\ninside stdout\nstderr:\nactual stderr",
            false,
        ),
        (
            "status: 0\nstdout:\npartial\nstderr:\n[yo: tool output truncated]",
            false,
        ),
        ("status: unknown\nstdout:\nx\nstderr:\ny", false),
    ] {
        let command = "if true; then\n  printf '%s\\n' hello\nfi";
        let output = ToolOutput {
            tool: "run_command".to_owned(),
            server: None,
            arguments: Some(json!({"command":command,"timeout":30})),
            result: Some(json!({"content":[{"type":"text","text":source}],"isError":true})),
            content_items: None,
            error: None,
            plain_text: format!("{command}\n{source}"),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(
                OutputPreferences::default()
                    .with_tool_head_rows(u16::MAX)
                    .with_shell_tail_rows(u16::MAX),
            )
            .with_theme_overrides(
                ThemeOverrides::default()
                    .with_color(ThemeRole::SyntaxKeyword, ThemeColor::Rgb(22, 216, 122)),
            );
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
        for width in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 100), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            assert!(text.contains("Command"), "{text}");
            let (y, row) = text
                .lines()
                .enumerate()
                .find(|(_, line)| line.contains("if true; then"))
                .unwrap();
            let x = row.find("if true; then").unwrap();
            assert_eq!(
                frame
                    .surface
                    .cell(Point::new(x as u16, y as u16))
                    .unwrap()
                    .style()
                    .foreground,
                Color::Rgb {
                    red: 22,
                    green: 216,
                    blue: 122
                }
            );
            assert!(!text.contains("\"command\""), "{text}");
            assert!(text.contains("\"timeout\": 30"), "{text}");
            assert_eq!(joined.contains("Exitstatus:"), parsed, "{text}");
            if source.contains("![literal]") {
                assert!(text.contains("![literal]"), "{text}");
                assert!(text.contains("^[[31m"), "{text}");
            }
            if source.contains("signal") {
                assert!(text.contains("(no stdout)"), "{text}");
                assert!(text.contains("(no stderr)"), "{text}");
            }
            if !parsed {
                assert!(text.contains("status:"), "{text}");
            }
            assert!(frame.surface.rasters.is_empty());
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Custom shell view".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 35), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom shell view"));
    }
}

// 여러 명령 도구의 합산 출력은 native stdout/stderr로 오해하지 않으며 같은 메타데이터·색상·원문을
// 보존한다.
#[test]
fn command_panels_preserve_aggregate_output_and_customization_across_tools() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole, ToolRenderer};

    for tool in ["commandExecution", "bash", "powershell"] {
        let command = if tool == "powershell" {
            "if ($true) {\n  Write-Output test\n}"
        } else {
            "if true; then\n  echo test\nfi"
        };
        let command_start = command.lines().next().unwrap();
        let source = "status: 0\nstdout:\n![literal](file.png)\nstderr:\ncombined";
        let output = ToolOutput {
            tool: tool.to_owned(),
            server: None,
            arguments: Some(json!({"command":command, "cwd":"/workspace"})),
            result: Some(
                json!({"content":[{"type":"text","text":source}], "exitCode":101,"durationMs":1240,"status":"failed"}),
            ),
            content_items: None,
            error: None,
            plain_text: source.to_owned(),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(
                OutputPreferences::default()
                    .with_tool_head_rows(u16::MAX)
                    .with_shell_tail_rows(u16::MAX),
            )
            .with_theme_overrides(
                ThemeOverrides::default()
                    .with_color(ThemeRole::SyntaxKeyword, ThemeColor::Rgb(22, 216, 122)),
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
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        let pin = session.appearance_pin();
        for width in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 100), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            for expected in [
                "Command",
                "Exitstatus:101",
                "Duration:1240ms",
                "Status:failed",
                "![literal](file.png)",
                "status:0",
            ] {
                assert!(joined.contains(expected), "{text}");
            }
            assert!(!joined.contains("Exitstatus:0"), "{text}");
            assert!(!text.contains("\"command\""), "{text}");
            let (y, row) = text
                .lines()
                .enumerate()
                .find(|(_, row)| row.contains(command_start))
                .unwrap();
            let x = row.find(command_start).unwrap();
            // 내장 문법이 없는 PowerShell은 원문을 보존하고 지원되는 Bash만 키워드 색상을 검증한다.
            if tool != "powershell" {
                assert_eq!(
                    frame
                        .surface
                        .cell(Point::new(x as u16, y as u16))
                        .unwrap()
                        .style()
                        .foreground,
                    Color::Rgb {
                        red: 22,
                        green: 216,
                        blue: 122
                    }
                );
            }
            assert!(frame.surface.rasters.is_empty());
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Custom command".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 35), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom command"));
    }
}

// 디렉터리 표시는 실제 잘림 메타데이터를 사용하고 같은 이름의 파일·빈 결과·원문·콜백을 보존한다.
#[test]
fn directory_listing_uses_observed_truncation_and_literal_names() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ToolRenderer};

    for (source, truncated, rich, partial) in [
        (
            "src/\nREADME.md\n![literal](file.png)\n",
            Some(false),
            true,
            false,
        ),
        ("", Some(false), true, false),
        (
            "src/\n\n[yo: tool output truncated]",
            Some(true),
            true,
            true,
        ),
        ("\n[yo: tool output truncated]", Some(true), true, true),
        ("src/\n", Some(true), true, true),
        ("[yo: tool output truncated]\n", Some(false), true, false),
        ("unterminated", Some(false), false, false),
        ("unterminated", Some(true), false, false),
        ("src/\n", None, false, false),
        ("bad\u{1b}name\n", Some(false), false, false),
    ] {
        let mut result = json!({"content":[{"type":"text","text":source}]});
        if let Some(truncated) = truncated {
            result["truncated"] = truncated.into();
        }
        let output = ToolOutput {
            tool: "list_files".to_owned(),
            server: None,
            arguments: Some(json!({"path":"."})),
            result: Some(result),
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
        let pin = session.appearance_pin();
        for width in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 65), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            assert_eq!(
                joined.contains("entriesshown") || joined.contains("entryshown"),
                rich,
                "{text}"
            );
            assert_eq!(joined.contains("Partiallisting"), partial, "{text}");
            assert!(!text.contains("\"truncated\""), "{text}");
            assert_eq!(
                text.contains("Output truncated"),
                truncated == Some(true) && !rich,
                "{text}"
            );
            if source.starts_with("[yo:") {
                assert!(joined.contains("file[yo:tooloutputtruncated]"), "{text}");
            }
            if source.starts_with("src/") && rich {
                assert!(joined.contains("dirsrc/"), "{text}");
                assert!(joined.contains("1directory"), "{text}");
            }
            if source.contains("![literal]") {
                assert!(text.contains("![literal]"), "{text}");
            }
            if source.is_empty() {
                assert!(joined.contains("Noentrieswerereturned."), "{text}");
            }
            assert!(frame.surface.rasters.is_empty());
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Custom directory view".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 35), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom directory view"));
    }
}

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

// 차트 색상은 제목 accent와 독립적으로 적용되고 override 제거 시 다시 accent를 따른다.
#[test]
fn chart_color_is_independent_from_headings_and_preserves_reflowed_source() {
    use crate::{OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole};
    let source = "# Chart heading\n\n```linechart\n1 5 2 8\n```\n\n```chart\nA: 10\nB: 20\n```";
    let accent = Color::Rgb {
        red: 12,
        green: 180,
        blue: 160,
    };
    let chart = Color::Rgb {
        red: 230,
        green: 160,
        blue: 40,
    };
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX));
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
            update: ActivityUpdate::TextSnapshot(source.to_owned()),
        })
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    for custom in [true, false, true] {
        let mut overrides =
            ThemeOverrides::default().with_color(ThemeRole::Accent, ThemeColor::Rgb(12, 180, 160));
        if custom {
            overrides = overrides.with_color(ThemeRole::Chart, ThemeColor::Rgb(230, 160, 40));
        }
        session = session.with_theme_overrides(overrides);
        for width in [80, 24, 80] {
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 70), &pin)
                .unwrap();
            let mut bars = 0;
            let mut traces = 0;
            let mut heading = false;
            for y in 0..frame.surface.size().height {
                let mut row = String::new();
                for x in 0..frame.surface.size().width {
                    let cell = frame.surface.cell(Point::new(x, y)).unwrap();
                    if let CellContent::Grapheme { text, .. } = cell.content() {
                        row.push_str(text);
                        if text
                            .chars()
                            .any(|c| ('\u{2800}'..='\u{28ff}').contains(&c) || c == '█')
                        {
                            assert_eq!(
                                cell.style().foreground,
                                if custom { chart } else { accent }
                            );
                            if text.contains('█') {
                                bars += 1;
                            } else {
                                traces += 1;
                            }
                        }
                    } else {
                        row.push(' ');
                    }
                }
                if let Some(x) = row.chars().position(|c| c == 'C')
                    && row.contains("Chart heading")
                {
                    assert_eq!(
                        frame
                            .surface
                            .cell(Point::new(x as u16, y))
                            .unwrap()
                            .style()
                            .foreground,
                        accent
                    );
                    heading = true;
                }
            }
            assert!(heading && bars > 0 && traces > 0);
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
    }
}

// 네 계열의 범례·점은 각각 지정 색상을 받고 흑백에서도 번호와 원래 수치가 유지된다.
#[test]
fn named_chart_colors_and_mono_legends_preserve_original_values() {
    use crate::{OutputPreferences, Theme, ThemeColor, ThemeOverrides, ThemeRole};
    let source =
        "```linechart\nheight: 6\nFirst: 30 30\nSecond: 20 20\nThird: 10 10\nFourth: 0 0\n```";
    let roles = [
        ThemeRole::Chart,
        ThemeRole::Chart2,
        ThemeRole::Chart3,
        ThemeRole::Chart4,
    ];
    let mut overrides = ThemeOverrides::default();
    for (i, role) in roles.into_iter().enumerate() {
        overrides = overrides.with_color(role, ThemeColor::Rgb(40 + i as u8 * 40, 100, 180));
    }
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX))
        .with_theme_overrides(overrides);
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
            update: ActivityUpdate::TextSnapshot(source.to_owned()),
        })
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    for theme in [Theme::Default, Theme::Mono, Theme::Light] {
        session = session.with_theme(theme);
        for width in [80, 24, 12, 80] {
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 100), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface)
                .split_whitespace()
                .collect::<String>();
            for name in ["[1]First", "[2]Second", "[3]Third", "[4]Fourth"] {
                assert!(text.contains(name), "{text}");
            }
            assert!(text.contains("Fourth:00"), "{text}");
            if theme != Theme::Mono {
                let mut colors = [false; 4];
                for y in 0..frame.surface.size().height {
                    for x in 0..frame.surface.size().width {
                        let color = frame
                            .surface
                            .cell(Point::new(x, y))
                            .unwrap()
                            .style()
                            .foreground;
                        for (i, found) in colors.iter_mut().enumerate() {
                            *found |= color
                                == Color::Rgb {
                                    red: 40 + i as u8 * 40,
                                    green: 100,
                                    blue: 180,
                                };
                        }
                    }
                }
                assert!(colors.into_iter().all(|found| found));
            }
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
    }
}

// 항목 수·경로 길이·입력 바이트 상한의 첫 초과 값은 일부만 목록화하지 않고 전체 원문으로 돌아간다.
#[test]
fn directory_listing_limits_fall_back_without_partial_parsing() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::OutputPreferences;

    let at_bytes = format!("{}\n", "p".repeat(1023)).repeat(256);
    let mut over_bytes = at_bytes.clone();
    over_bytes.insert(over_bytes.len() - 1, 'p');
    for (source, rich) in [
        ("a\n".repeat(1024), true),
        ("a\n".repeat(1025), false),
        (format!("{}\n", "p".repeat(1024)), true),
        (format!("{}\n", "p".repeat(1025)), false),
        (at_bytes, true),
        (over_bytes, false),
    ] {
        let output = ToolOutput {
            tool: "list_files".to_owned(),
            server: None,
            arguments: None,
            result: Some(json!({"content":[{"type":"text","text":source}],"truncated":false})),
            content_items: None,
            error: None,
            plain_text: source,
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
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 60), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert_eq!(text.contains("file  "), rich, "{text}");
    }
}

// 셸 접기는 최신 출력과 명령·결과를 유지하며 스트리밍·폭 변경·설정·전체 펼치기를 반영한다.
#[test]
fn shell_tail_keeps_latest_output_and_restores_complete_source() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use super::key;
    use crate::{
        OutputPreferences, ToolRenderer,
        input::event::{KeyCode, KeyModifiers},
    };

    for tool in ["commandExecution", "run_command", "bash", "powershell"] {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_shell_tail_rows(3));
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolCall,
            })
            .unwrap();
        let lines = (0..12)
            .map(|n| format!("output {n:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut output = ToolOutput {
            tool: tool.to_owned(),
            server: None,
            arguments: Some(json!({"command":"echo test","cwd":"/workspace"})),
            result: None,
            content_items: None,
            error: None,
            plain_text: lines.clone(),
        };
        for latest in ["LATEST-A", "LATEST-B"] {
            let text = if tool == "run_command" {
                format!("status: 0\nstdout:\n{lines}\n{latest}\nstderr:\nwarning")
            } else {
                format!("{lines}\n{latest}")
            };
            output.result =
                Some(json!({"content":[{"type":"text","text":text}],"exitCode":0,"durationMs":7}));
            output.plain_text = text;
            session
                .parts_mut()
                .state
                .observe(AgentEvent::ActivityUpdated {
                    activity: activity(1),
                    update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
                })
                .unwrap();
            let original = session.session_output().unwrap().unwrap();
            let pin = session.appearance_pin();
            for width in [80, 24, 80] {
                let frame = session
                    .parts_mut()
                    .state
                    .prepare_frame(Size::new(width, 100), &pin)
                    .unwrap();
                let text = visible_rows(&frame.surface);
                let joined = text
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>();
                assert!(text.contains(latest), "{text}");
                assert!(text.contains("echo test"), "{text}");
                assert!(!text.contains("output 00"), "{text}");
                assert!(joined.contains("earlieroutputrowshidden"), "{text}");
                assert!(text.contains("output 11"), "{text}");
                if tool == "commandExecution" {
                    assert!(text.contains("Duration: 7 ms"), "{text}");
                }
                assert_eq!(session.session_output().unwrap().unwrap(), original);
            }
        }
        let pin = session.appearance_pin();
        session
            .parts_mut()
            .state
            .handle(
                key(KeyCode::Character('o'), KeyModifiers::CONTROL),
                Duration::ZERO,
            )
            .unwrap();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 100), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(
            text.contains("output 00") && text.contains("LATEST-B"),
            "{text}"
        );
        assert!(!text.contains("earlier output rows hidden"), "{text}");
        session
            .parts_mut()
            .state
            .handle(
                key(KeyCode::Character('o'), KeyModifiers::CONTROL),
                Duration::ZERO,
            )
            .unwrap();
        session = session
            .with_output_preferences(OutputPreferences::default().with_shell_tail_rows(u16::MAX));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 100), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("output 00"));
        session = session.with_output_preferences(
            OutputPreferences::default()
                .with_shell_tail_rows(0)
                .with_tool_head_rows(u16::MAX),
        );
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 100), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("output 00"));
        let expected = output.clone();
        session = session
            .with_output_preferences(
                OutputPreferences::default()
                    .with_shell_tail_rows(1)
                    .with_tool_head_rows(u16::MAX),
            )
            .with_tool_renderer(Some(ToolRenderer::new(move |input| {
                assert_eq!(input.output, Some(&expected));
                Some("CUSTOM FIRST\n\nCUSTOM LAST".to_owned())
            })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 100), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(
            text.contains("CUSTOM FIRST") && text.contains("CUSTOM LAST"),
            "{text}"
        );
    }
}

// 마지막 표시 행 수의 경계와 한글·탭·제어 문자·Markdown 원문이 접힌 화면에서도 안전하게 남는다.
#[test]
fn shell_tail_counts_visual_rows_and_keeps_literal_cells() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ToolRenderer};
    for (source, hidden) in [
        ("one\ntwo\nthree", false),
        ("zero\none\ntwo\nthree", true),
        ("old\nold\nold\n\t한글\n![x](a.png)\n\u{1b}[31m", true),
    ] {
        let output = ToolOutput {
            tool: "commandExecution".to_owned(),
            server: None,
            arguments: Some(json!({"command":"test"})),
            result: Some(json!({"content":[{"type":"text","text":source}]})),
            content_items: None,
            error: None,
            plain_text: source.to_owned(),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_shell_tail_rows(3));
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
        let original = session.session_output().unwrap().unwrap();
        let pin = session.appearance_pin();
        for width in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 80), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            assert_eq!(joined.contains("earlieroutputrowshidden"), hidden, "{text}");
            if source.contains("한글") {
                assert!(joined.contains("한글"), "{text}");
                assert!(text.contains("![x](a.png)"), "{text}");
                assert!(text.contains("^[[31m"), "{text}");
            } else {
                assert!(text.contains("one") && text.contains("three"), "{text}");
                assert!(!text.contains("zero"), "{text}");
            }
            assert!(frame.surface.rasters.is_empty());
        }
        assert_eq!(session.session_output().unwrap().unwrap(), original);
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Original shell source retained".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 80), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Original shell source retained"));
    }
}

// 보고된 셸 잘림·전체 출력 경로를 표시하고 모호한 메타데이터와 원본 콜백을 보존한다.
#[test]
fn shell_output_details_show_reported_limits_and_literal_paths() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ToolRenderer};
    let truncation = json!({"content":"tail", "truncated":true,"truncatedBy":"bytes","totalLines":100,"totalBytes":9000,"outputLines":1,"outputBytes":4,"lastLinePartial":true,"firstLineExceedsLimit":false,"maxLines":2000,"maxBytes":4});
    let mut lines = truncation.clone();
    lines["truncatedBy"] = json!("lines");
    lines["lastLinePartial"] = json!(false);
    lines["maxLines"] = json!(1);
    let mut complete = truncation.clone();
    complete["truncated"] = json!(false);
    complete["truncatedBy"] = json!(null);
    complete["lastLinePartial"] = json!(false);
    complete["totalLines"] = json!(1);
    complete["totalBytes"] = json!(4);
    let mut invalid = truncation.clone();
    invalid["outputBytes"] = json!(9001);
    let mut mismatch = truncation.clone();
    mismatch["content"] = json!("lost");
    for (details, rich, truncated) in [
        (
            json!({"truncation":truncation,"fullOutputPath":"/tmp/build output.log"}),
            true,
            true,
        ),
        (json!({"truncation":lines}), true, true),
        (json!({"truncation":complete}), true, false),
        (
            json!({"fullOutputPath":"![literal](file.png)"}),
            true,
            false,
        ),
        (json!({"truncation":invalid}), false, false),
        (json!({"truncation":mismatch}), false, false),
        (
            json!({"fullOutputPath":"/tmp/x","extension":"retained"}),
            false,
            false,
        ),
        (json!({"fullOutputPath":"/tmp/\u{1b}[31m"}), false, false),
        (json!({"fullOutputPath":"x".repeat(4096)}), true, false),
        (json!({"fullOutputPath":"x".repeat(4097)}), false, false),
    ] {
        let output = ToolOutput {
            tool: "bash".to_owned(),
            server: None,
            arguments: Some(json!({"command":"echo test"})),
            result: Some(json!({"content":[{"type":"text","text":"tail"}],"details":details})),
            content_items: None,
            error: None,
            plain_text: format!("tail\n{details}"),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_shell_tail_rows(3));
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
        let original = session.session_output().unwrap().unwrap();
        let pin = session.appearance_pin();
        for width in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 1000), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            assert_eq!(joined.contains("Outputtruncated"), truncated, "{text}");
            assert_eq!(
                text.lines().any(|line| line.trim() == "details"),
                !rich,
                "{text}"
            );
            if rich && details.get("fullOutputPath").is_some() {
                assert!(joined.contains("Fulloutputfile"), "{text}");
            }
            if details.pointer("/truncation/lastLinePartial") == Some(&json!(true)) && rich {
                assert!(joined.contains("Lastlineispartial"), "{text}");
            }
            if rich && truncated {
                assert!(joined.contains("Outputlines:1of100"), "{text}");
            }
            assert!(frame.surface.rasters.is_empty());
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Original details retained".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 40), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Original details retained"));
    }
}

// 메타데이터 표시 한도의 정확한 끝과 첫 초과 바이트에서 원문 fallback이 전환된다.
#[test]
fn shell_output_details_size_limit_preserves_literal_fallback() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::OutputPreferences;
    for extra in [0, 1] {
        let target = 256 * 1024 + extra;
        let mut content = "x".repeat(target - 300);
        let mut details = json!({"truncation":{"content":"","truncated":true,"truncatedBy":"bytes","totalLines":2,"totalBytes":999999,"outputLines":1,"outputBytes":0,"lastLinePartial":true,"firstLineExceedsLimit":false,"maxLines":2000,"maxBytes":999999}});
        for _ in 0..8 {
            details["truncation"]["content"] = json!(content);
            details["truncation"]["outputBytes"] = json!(content.len());
            let size = details.to_string().len();
            if size == target {
                break;
            }
            if size < target {
                content.push_str(&"x".repeat(target - size));
            } else {
                content.truncate(content.len() - (size - target));
            }
        }
        assert_eq!(details.to_string().len(), target);
        let output = ToolOutput {
            tool: "bash".to_owned(),
            server: None,
            arguments: None,
            result: Some(json!({"content":[{"type":"text","text":content}],"details":details})),
            content_items: None,
            error: None,
            plain_text: "Original large metadata retained".to_owned(),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_shell_tail_rows(1));
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
            .prepare_frame(Size::new(80, 4000), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert_eq!(text.contains("Output truncated"), extra == 0);
        assert_eq!(
            text.lines().any(|line| line.trim() == "details"),
            extra == 1
        );
    }
}

// 내장 명령의 진행 결과는 종료 코드를 꾸미지 않고 스트림별로 표시되며 최종 출력으로 교체된다.
#[test]
fn native_command_progress_renders_streams_until_authoritative_completion() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::OutputPreferences;
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_output_preferences(OutputPreferences::default().with_shell_tail_rows(5));
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ToolCall,
        })
        .unwrap();
    for completed in [false, true] {
        let output = ToolOutput {
            tool: "run_command".to_owned(),
            server: None,
            arguments: Some(json!({"command":"echo test"})),
            result: Some(if completed {
                json!({"content":[{"type":"text","text":"status: 0\nstdout:\nfinal output\nstderr:\n"}]})
            } else {
                json!({"content":[{"type":"text","text":json!({"stdout":"first output\n![literal](file.png)","stderr":"warning"}).to_string()}],"progress":true})
            }),
            content_items: None,
            error: None,
            plain_text: if completed {
                "final output".to_owned()
            } else {
                "first output".to_owned()
            },
        };
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
            })
            .unwrap();
        let pin = session.appearance_pin();
        for width in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 100), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            assert!(text.contains("stdout") && text.contains("stderr"), "{text}");
            assert_eq!(joined.contains("Exitstatus:0"), completed, "{text}");
            assert_eq!(text.contains("final output"), completed, "{text}");
            assert_eq!(text.contains("first output"), !completed, "{text}");
            assert_eq!(text.contains("![literal]"), !completed, "{text}");
            assert!(!text.contains("\"progress\""), "{text}");
            assert!(frame.surface.rasters.is_empty());
        }
    }
}

// 7만 행과 좁은 화면에서 6만 행 넘게 개행되는 한 줄도 최신 출력·원문·사용자 콜백을 보존한다.
#[test]
fn folded_shell_output_survives_large_row_counts_and_long_lines() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ToolRenderer};
    for source in [
        format!("EARLY\n{}LATEST", "row\n".repeat(70_000)),
        format!("EARLY{}LATEST", "x".repeat(1_500_000)),
    ] {
        let output = ToolOutput {
            tool: "commandExecution".to_owned(),
            server: None,
            arguments: Some(json!({"command":"emit test"})),
            result: Some(json!({"content":[{"type":"text","text":source}]})),
            content_items: None,
            error: None,
            plain_text: source.clone(),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_shell_tail_rows(5));
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
        for columns in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(columns, 80), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            assert!(joined.contains("LATEST"), "{text}");
            if source.contains('\n') {
                assert!(joined.contains("69997earlieroutputrows"), "{text}");
            }
            assert!(!text.contains("EARLY"), "{text}");
            assert!(joined.contains("earlieroutputrowshidden"), "{text}");
            assert!(text.contains("emit test"), "{text}");
        }
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            Some("Full large source available".to_owned())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 40), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Full large source available"));
    }
}

// 스트리밍으로 정의가 도착하면 각주를 연결하되 도구 로그와 원문 내보내기는 변환하지 않는다.
#[test]
fn footnote_frames_preserve_streaming_source_and_tool_literals() {
    for kind in [ActivityKind::AgentMessage, ActivityKind::ToolResult] {
        let assistant = kind == ActivityKind::AgentMessage;
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced);
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind,
            })
            .unwrap();
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot("Claim[^note].".into()),
            })
            .unwrap();
        let pin = session.appearance_pin();
        let initial = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 20), &pin)
            .unwrap();
        assert!(visible_rows(&initial.surface).contains("Claim[^note]"));
        let source = "Claim[^note].\n\n[^note]: A **bold** source.";
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(source.into()),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        for width in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 24), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface)
                .split_whitespace()
                .collect::<String>();
            assert!(
                text.contains(if assistant {
                    "Claim[note]"
                } else {
                    "Claim[^note]"
                }),
                "{text}"
            );
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
        assert!(original.contains("Claim[^note]"));
        assert!(original.contains("[^note]: A **bold** source."));
    }
}

// 커스텀 렌더러는 실제 펼침 상태를 받고 왕복 토글·폭 변경 때 재배치되며 원문은 바꾸지 않는다.
#[test]
fn tool_renderer_receives_expansion_state_across_cached_frames() {
    use std::time::Duration;

    use super::key;
    use crate::{
        ToolRenderer,
        input::event::{KeyCode, KeyModifiers},
    };

    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_tool_renderer(Some(ToolRenderer::new(|input| {
            assert_eq!(input.source, "retained original payload");
            Some(
                if input.expanded {
                    "Expanded detail"
                } else {
                    "Compact summary"
                }
                .into(),
            )
        })));
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
            update: ActivityUpdate::TextSnapshot("retained original payload".into()),
        })
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    for (toggle, width, expanded) in [
        (false, 80, false),
        (false, 80, false),
        (true, 80, true),
        (false, 24, true),
        (true, 24, false),
    ] {
        if toggle {
            session
                .parts_mut()
                .state
                .handle(
                    key(KeyCode::Character('o'), KeyModifiers::CONTROL),
                    Duration::ZERO,
                )
                .unwrap();
        }
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 24), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert_eq!(text.contains("Expanded detail"), expanded, "{text}");
        assert_eq!(text.contains("Compact summary"), !expanded, "{text}");
        session.parts_mut().state.commit_frame(&frame);
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
    assert!(original.contains("retained original payload"));
}

// 종료 결과는 payload 단어에서 추측하지 않고 실제 완료 이벤트 뒤 콜백·캐시에 반영한다.
#[test]
fn tool_renderer_receives_observed_terminal_outcomes() {
    use yo_core::{ActivityOutcome, Failure};

    use crate::{ToolRenderer, TranscriptActivityOutcome};

    for (outcome, expected) in [
        (ActivityOutcome::Completed, "Completion detail"),
        (ActivityOutcome::Interrupted, "Interruption detail"),
        (
            ActivityOutcome::Failed(Failure::new("Original failure")),
            "Failure detail",
        ),
    ] {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_tool_renderer(Some(ToolRenderer::new(|input| {
                assert_eq!(input.source, "payload says completed and failed");
                Some(
                    match input.outcome {
                        None => "Waiting for outcome",
                        Some(TranscriptActivityOutcome::Completed) => "Completion detail",
                        Some(TranscriptActivityOutcome::Interrupted) => "Interruption detail",
                        Some(TranscriptActivityOutcome::Failed) => "Failure detail",
                    }
                    .into(),
                )
            })));
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
                update: ActivityUpdate::TextSnapshot("payload says completed and failed".into()),
            })
            .unwrap();
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 24), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Waiting for outcome"));
        session.parts_mut().state.commit_frame(&frame);
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome,
            })
            .unwrap();
        for width in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 24), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            assert!(text.contains(expected), "{text}");
            assert!(!text.contains("Waiting for outcome"), "{text}");
            if expected == "Failure detail" {
                assert!(
                    text.split_whitespace()
                        .collect::<String>()
                        .contains("Originalfailure"),
                    "{text}"
                );
            }
            session.parts_mut().state.commit_frame(&frame);
        }
        let source = session.session_output().unwrap().unwrap();
        assert!(source.contains("payload says completed and failed"));
        assert!(!source.contains(expected));
    }
}

// Alt+O는 반영된 화면의 항목만 토글하고 개별 상태·렌더러 인수·원문을 유지하며 Ctrl+O는 전체를
// 재설정한다.
#[test]
fn individual_activity_expansion_preserves_other_items_and_custom_rendering() {
    use std::time::Duration;

    use yo_core::ActivityOutcome;

    use super::key;
    use crate::{
        ToolRenderer,
        input::event::{KeyCode, KeyModifiers},
    };

    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_tool_renderer(Some(ToolRenderer::new(|input| {
            Some(format!(
                "{} {}",
                input.source,
                if input.expanded {
                    "expanded"
                } else {
                    "compact"
                }
            ))
        })));
    for (index, text) in [(1, "FIRST"), (2, "SECOND")] {
        let state = &mut session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(index),
                kind: ActivityKind::ToolCall,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(index),
                update: ActivityUpdate::TextSnapshot(text.into()),
            })
            .unwrap();
    }
    for index in [1, 2] {
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(index),
                outcome: ActivityOutcome::Completed,
            })
            .unwrap();
    }
    let original = session.session_output().unwrap().unwrap();
    let render = |session: &mut TuiSession, width| {
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 32), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        session.parts_mut().state.commit_frame(&frame);
        text
    };
    render(&mut session, 80);
    session
        .parts_mut()
        .state
        .handle(
            key(KeyCode::Character('o'), KeyModifiers::ALT),
            Duration::ZERO,
        )
        .unwrap();
    let pin = session.appearance_pin();
    let protected = session
        .parts_mut()
        .state
        .prepare_frame_for_geometry(Size::new(80, 32), &pin, Duration::ZERO, 1)
        .unwrap();
    assert!(protected.publication.is_none());
    for width in [80, 24, 80] {
        let text = render(&mut session, width);
        assert!(text.contains("FIRST compact"), "{text}");
        assert!(text.contains("SECOND expanded"), "{text}");
    }
    session
        .parts_mut()
        .state
        .handle(key(KeyCode::Home, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    // 화면 반영 전 이동과 토글이 겹치면 이전 항목을 잘못 바꾸지 않는다.
    session
        .parts_mut()
        .state
        .handle(
            key(KeyCode::Character('o'), KeyModifiers::ALT),
            Duration::ZERO,
        )
        .unwrap();
    let text = render(&mut session, 80);
    assert!(text.contains("FIRST compact"));
    assert!(text.contains("SECOND expanded"));
    session
        .parts_mut()
        .state
        .handle(
            key(KeyCode::Character('o'), KeyModifiers::ALT),
            Duration::ZERO,
        )
        .unwrap();
    let text = render(&mut session, 24);
    assert!(text.contains("FIRST expanded"));
    assert!(text.contains("SECOND expanded"));
    session
        .parts_mut()
        .state
        .handle(
            key(KeyCode::Character('o'), KeyModifiers::ALT),
            Duration::ZERO,
        )
        .unwrap();
    let text = render(&mut session, 80);
    assert!(text.contains("FIRST compact"));
    assert!(text.contains("SECOND expanded"));
    for expanded in [true, false] {
        session
            .parts_mut()
            .state
            .handle(
                key(KeyCode::Character('o'), KeyModifiers::CONTROL),
                Duration::ZERO,
            )
            .unwrap();
        let text = render(&mut session, 80);
        for name in ["FIRST", "SECOND"] {
            assert!(
                text.contains(&format!(
                    "{name} {}",
                    if expanded { "expanded" } else { "compact" }
                )),
                "{text}"
            );
        }
    }
    session
        .parts_mut()
        .state
        .handle(key(KeyCode::End, KeyModifiers::NONE), Duration::ZERO)
        .unwrap();
    let pin = session.appearance_pin();
    let resumed = session
        .parts_mut()
        .state
        .prepare_frame_for_geometry(Size::new(80, 32), &pin, Duration::ZERO, 1)
        .unwrap();
    assert!(resumed.publication.is_some());
    assert_eq!(session.session_output().unwrap().unwrap(), original);
}

// 기본 로그의 개별 펼침은 스트리밍 revision과 실패 footer를 보존하고 이후 항목까지 펼치지 않는다.
#[test]
fn individual_default_log_expansion_survives_streaming_and_failure() {
    use std::time::Duration;

    use yo_core::{ActivityOutcome, Failure};

    use super::key;
    use crate::input::event::{KeyCode, KeyModifiers};

    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced);
    let payload = |prefix: &str, count| {
        (1..=count)
            .map(|n| format!("{prefix} {n:02}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let render = |session: &mut TuiSession, width| {
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 90), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        session.parts_mut().state.commit_frame(&frame);
        text
    };
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
            update: ActivityUpdate::TextSnapshot(payload("FIRST", 20)),
        })
        .unwrap();
    let compact = render(&mut session, 80);
    assert!(!compact.contains("FIRST 05"));
    session
        .parts_mut()
        .state
        .handle(
            key(KeyCode::Character('o'), KeyModifiers::ALT),
            Duration::ZERO,
        )
        .unwrap();
    assert!(render(&mut session, 80).contains("FIRST 05"));
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(payload("FIRST", 24)),
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityFinished {
            activity: activity(1),
            outcome: ActivityOutcome::Failed(Failure::new("Failure retained")),
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(2),
            kind: ActivityKind::ToolCall,
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(2),
            update: ActivityUpdate::TextSnapshot(payload("SECOND", 20)),
        })
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    for width in [80, 24, 80] {
        let text = render(&mut session, width);
        assert!(text.contains("FIRST 05"), "{text}");
        assert!(text.contains("FIRST 24"), "{text}");
        assert!(
            text.chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
                .contains("Failed:Failureretained"),
            "{text}"
        );
        assert!(!text.contains("SECOND 05"), "{text}");
        assert!(text.contains("SECOND 20"), "{text}");
    }
    // 최신 항목 토글도 기존 첫 항목의 선택을 잃지 않는다.
    session
        .parts_mut()
        .state
        .handle(
            key(KeyCode::Character('o'), KeyModifiers::ALT),
            Duration::ZERO,
        )
        .unwrap();
    let text = render(&mut session, 24);
    assert!(text.contains("FIRST 05"));
    assert!(text.contains("SECOND 05"));
    assert_eq!(session.session_output().unwrap().unwrap(), original);
    assert!(original.contains("FIRST 05"));
    assert!(original.contains("SECOND 05"));
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
