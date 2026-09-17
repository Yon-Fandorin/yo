use super::*;

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
