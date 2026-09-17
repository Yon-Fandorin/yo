use super::*;

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
