use super::*;

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
// 셸 접기는 최신 출력과 명령·결과를 유지하며 스트리밍·폭 변경·설정·전체 펼치기를 반영한다.
#[test]
fn shell_tail_keeps_latest_output_and_restores_complete_source() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use super::super::key;
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
