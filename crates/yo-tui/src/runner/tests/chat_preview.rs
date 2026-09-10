use std::{num::NonZeroU64, time::Duration};

use yo_core::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentCommand,
    AgentEvent, Failure, RequestId, TranscriptRecord, TurnId, TurnRef, UserInput,
};

use super::{activity, key, turn};
use crate::{
    ColorCapability, GlyphProfile, MotionPreference, PresentationMode, Theme, TuiSession,
    TuiSessionInfo,
    html::HtmlSurface,
    input::event::{InputEvent, KeyCode, KeyModifiers},
    runner::preview_agent::media_response,
    surface::{Attributes, CellContent, Color, FrameDiff, Point, Size, Surface},
    terminal::{AnsiEncoder, TerminalOps},
};

fn text(surface: &Surface) -> String {
    let mut output = String::new();
    for y in 0..surface.size().height {
        for x in 0..surface.size().width {
            match surface.cell(Point::new(x, y)).unwrap().content() {
                CellContent::Grapheme { text, .. } => output.push_str(text),
                CellContent::Blank => output.push(' '),
                CellContent::Continuation { .. } => {},
            }
        }
        output.push('\n');
    }
    output
}

// 실제 Session→Surface 경로로 빈 화면·대화·작업 중 화면을 여러 폭에서 확인한다.
// 안내는 화면에만 있고 저장/출력되는 대화에는 들어가지 않아야 한다.
#[test]
fn chat_preview_preserves_content_and_exports_real_frames() {
    let mut previews = String::new();
    for (profile, capability, theme, label) in [
        (
            GlyphProfile::Rich,
            ColorCapability::TrueColor,
            Theme::Default,
            "color",
        ),
        (
            GlyphProfile::Rich,
            ColorCapability::TrueColor,
            Theme::Light,
            "light",
        ),
        (
            GlyphProfile::Rich,
            ColorCapability::TrueColor,
            Theme::Mono,
            "mono",
        ),
        (
            GlyphProfile::Rich,
            ColorCapability::Limited,
            Theme::Light,
            "light-indexed",
        ),
        (
            GlyphProfile::Ascii,
            ColorCapability::Unknown,
            Theme::Default,
            "plain",
        ),
    ] {
        for width in [20, 40, 88] {
            for scenario in [
                "welcome",
                "conversation",
                "working",
                "markdown",
                "tables",
                "diff",
                "changes",
                "usage",
                "plan",
                "tools",
                "error",
                "interrupted",
                "draft",
                "history",
                "commands",
                "long-tools",
                "multi-turn",
                "approval",
                "interview",
                "syntax",
                "charts",
                "images",
                "media-errors",
            ] {
                let mut session = TuiSession::with_session_info(
                    profile,
                    TuiSessionInfo::new("Local Codex", "~/projects/yo"),
                    capability,
                    MotionPreference::Reduced,
                )
                .with_theme(theme);
                session.set_presentation_mode(PresentationMode::Fullscreen);
                if scenario != "welcome" {
                    let state = session.parts_mut().state;
                    state
                        .observe_record(TranscriptRecord::CommandCommitted(
                            AgentCommand::StartTurn {
                                turn: turn(),
                                input: UserInput::from(
                                    "Make the chat easier to read. 한글도 확인해줘.",
                                ),
                            },
                        ))
                        .unwrap();
                    state
                        .observe(AgentEvent::ActivityStarted {
                            activity: activity(1),
                            kind: if scenario == "approval" {
                                ActivityKind::ApprovalRequest {
                                    request_id: RequestId::new(NonZeroU64::new(1).unwrap()),
                                }
                            } else if scenario == "interview" {
                                ActivityKind::UserInputRequest {
                                    request_id: RequestId::new(NonZeroU64::new(1).unwrap()),
                                }
                            } else if scenario == "changes" {
                                ActivityKind::FileChange
                            } else if matches!(scenario, "usage" | "plan") {
                                ActivityKind::ModelWork
                            } else if matches!(
                                scenario,
                                "tools" | "error" | "interrupted" | "long-tools"
                            ) {
                                ActivityKind::ToolCall
                            } else {
                                ActivityKind::AgentMessage
                            },
                        })
                        .unwrap();
                    let long_response = (1..=36)
                        .map(|n| {
                            format!("Check {n:02}: rendering and Unicode wrapping passed.  \n")
                        })
                        .collect::<String>();
                    let media = media_response(scenario);
                    let response = if let Some(response) = media.as_ref() {
                        response
                    } else if scenario == "approval" {
                        "Run the layout checks?\n\nCommand: cargo test --locked -p yo-tui\nScope: this request only\n\nPreview only: no command will run."
                    } else if scenario == "interview" {
                        "Question 1 of 2 · Priority\n\nWhat should the chat make easiest?\n1. Reading code and explanations\n2. Reviewing changes and approvals\n\nEnter a number or your own answer."
                    } else if matches!(scenario, "history" | "long-tools") {
                        &long_response
                    } else if matches!(scenario, "tools" | "error" | "interrupted") {
                        "[SIMULATED] cargo test\nChecking layout and Unicode wrapping...\n**literal log content**"
                    } else if scenario == "tables" {
                        "| Component | Status | Tests |\n| :--- | :---: | ---: |\n| Rendering | **ready** | 32 |\n| 한글 wrapping | ready | 12 |\n| Theme | `mono` | 8 |"
                    } else if scenario == "changes" {
                        "update: src/settings.rs\n@@ -1,2 +1,3 @@\n-let theme = \"fixed\";\n+let theme = \"selected\";\n+render(theme);\n context"
                    } else if scenario == "plan" {
                        "Plan\n1/3 completed\n[x] Inspect the renderer\n[>] Connect backend events\n[ ] Verify navigation"
                    } else if scenario == "usage" {
                        r#"{"schema":"codex.app-server-token-usage-receipt/v1","source_profile":"codex.app-server.thread-token-usage-updated/v1","turn_id":"preview-only","model_context_window":200000,"usage":{"input_tokens":12500,"output_tokens":1800,"total_tokens":14300,"reasoning_tokens":600,"cache_read_input_tokens":9800,"cache_write_input_tokens":0}}"#
                    } else if scenario == "diff" {
                        "```diff\n--- a/settings.rs\n+++ b/settings.rs\n@@ -1,2 +1,2 @@\n-let theme = \"fixed\";\n+let theme = \"selected\";\n render(theme);\n```"
                    } else if scenario == "markdown" {
                        "## Clearer output\n\n**Readable prose**, `inline code`, and lists.\n\n- Preserve your layout\n- Check 한글 and wrapping\n\n```rust\nfn main() {\n    println!(\"hello\");\n}\n```"
                    } else {
                        "I will separate your request from the response, keep the input visible,\nand make the next action clear.\n\nThe same layout works in narrow terminals and without color."
                    };
                    state
                        .observe(AgentEvent::ActivityUpdated {
                            activity: activity(1),
                            update: ActivityUpdate::TextSnapshot(response.to_owned()),
                        })
                        .unwrap();
                    if matches!(scenario, "tools" | "error" | "interrupted") {
                        state
                            .observe(AgentEvent::ActivityFinished {
                                activity: activity(1),
                                outcome: match scenario {
                                    "error" => ActivityOutcome::Failed(Failure::new(
                                        "Fixture missing. Check the path and try again.",
                                    )),
                                    "interrupted" => ActivityOutcome::Interrupted,
                                    _ => ActivityOutcome::Completed,
                                },
                            })
                            .unwrap();
                    }
                    if matches!(
                        scenario,
                        "working" | "history" | "long-tools" | "approval" | "interview"
                    ) {
                        state
                            .observe(AgentEvent::TurnStarted { turn: turn() })
                            .unwrap();
                    }
                    if scenario == "draft" {
                        let draft = (1..=24)
                            .map(|n| format!("Requirement {n}: preserve the conversation."))
                            .collect::<Vec<_>>()
                            .join("\n");
                        state
                            .handle(InputEvent::Paste(draft), Duration::ZERO)
                            .unwrap();
                    }
                    if scenario == "history" {
                        state
                            .handle(key(KeyCode::Home, KeyModifiers::NONE), Duration::ZERO)
                            .unwrap();
                    }
                    if scenario == "commands" {
                        state
                            .handle(InputEvent::Paste("/".to_owned()), Duration::ZERO)
                            .unwrap();
                    }
                    if scenario == "multi-turn" {
                        state
                            .observe(AgentEvent::ActivityFinished {
                                activity: activity(1),
                                outcome: ActivityOutcome::Completed,
                            })
                            .unwrap();
                        for (index, (request, answer)) in [
                            ("도구 결과도 읽기 쉽게 정리해줘.", "## Validation\n\n- **Passed:** layout and theme checks\n- **Preserved:** original tool output"),
                            ("Keep the final result concise.", "Ready. The response stays readable, with details available when needed."),
                        ].into_iter().enumerate() {
                            let turn = TurnRef::new(turn().session_id(), TurnId::new(NonZeroU64::new(index as u64 + 2).unwrap()));
                            let activity = ActivityRef::new(turn, ActivityId::new(NonZeroU64::MIN));
                            state.observe_record(TranscriptRecord::CommandCommitted(AgentCommand::StartTurn {
                                turn, input: UserInput::from(request),
                            })).unwrap();
                            state.observe(AgentEvent::ActivityStarted { activity, kind: ActivityKind::AgentMessage }).unwrap();
                            state.observe(AgentEvent::ActivityUpdated { activity, update: ActivityUpdate::TextSnapshot(answer.to_owned()) }).unwrap();
                            state.observe(AgentEvent::ActivityFinished { activity, outcome: ActivityOutcome::Completed }).unwrap();
                        }
                    }
                }
                let pin = session.appearance_pin();
                let frame = session
                    .parts_mut()
                    .state
                    .prepare_frame(Size::new(width, 22), &pin)
                    .unwrap();
                for y in 0..frame.surface.size().height {
                    for x in 0..width {
                        let style = frame.surface.cell(Point::new(x, y)).unwrap().style();
                        if theme == Theme::Mono || capability == ColorCapability::Unknown {
                            assert_eq!(style.foreground, Color::Default);
                            assert_eq!(style.background, Color::Default);
                        }
                        if capability == ColorCapability::Limited {
                            assert!(!matches!(style.foreground, Color::Rgb { .. }));
                            assert!(!matches!(style.background, Color::Rgb { .. }));
                        }
                    }
                }
                let visible = text(&frame.surface);
                if scenario == "welcome" {
                    assert!(session.session_output().unwrap().is_none());
                    if width >= 40 {
                        assert!(visible.contains("Let's build something."));
                        assert!(visible.contains("Ask anything"));
                    }
                } else {
                    assert!(!visible.contains("Let's build something."));
                    let output = session.session_output().unwrap().unwrap();
                    assert!(!output.contains("Ask anything"));
                    assert!(output.contains("Make the chat"));
                    if scenario == "plan" {
                        assert!(output.contains("1/3 completed"), "{output}");
                        assert!(!output.contains("Thinking"));
                        assert!(visible.contains("Plan"));
                    } else if scenario == "images" {
                        assert!(output.contains("data:image/png;base64,"));
                        assert!(!visible.contains("base64,"));
                    } else if scenario == "charts" {
                        assert!(output.contains("```chart"));
                        assert!(!visible.contains("```chart"));
                    }
                    if matches!(scenario, "conversation" | "working") {
                        assert!(
                            frame
                                .surface
                                .cell(Point::new(2, 0))
                                .unwrap()
                                .style()
                                .attributes
                                .contains(Attributes::BOLD)
                        );
                    } else if scenario == "markdown" {
                        assert!(output.contains("## Clearer output"));
                        assert!(output.contains("```rust"));
                        assert!(!visible.contains("```rust"));
                        if width == 88 {
                            assert!(visible.contains("Clearer output"));
                        }
                    } else if scenario == "tables" {
                        assert!(output.contains("| :--- | :---: | ---: |"));
                        assert!(!visible.contains("| :--- | :---: | ---: |"));
                        if width == 88 {
                            assert!(visible.contains("Component"));
                        }
                    } else if matches!(scenario, "tools" | "error" | "interrupted") {
                        assert!(output.contains("**literal log content**"));
                        if width == 88 {
                            assert!(visible.contains("**literal log content**"));
                        }
                    } else if scenario == "diff" {
                        assert!(output.contains("```diff"));
                        assert!(!visible.contains("```diff"));
                        if width == 88 {
                            assert!(visible.contains("+let theme"));
                        }
                    }
                }
                assert!(frame.cursor.x < width);
                assert!(frame.cursor.y < 22);
                if matches!(scenario, "approval" | "interview") && width >= 40 {
                    assert!(visible.contains(if scenario == "approval" {
                        "Preview only: no command will run."
                    } else {
                        "Enter a number or your own answer."
                    }));
                    assert!(!visible.contains("Ask anything"));
                }
                if scenario == "commands" && width >= 40 {
                    assert!(visible.contains("Enter select"));
                    assert!(!visible.contains("Enter send"));
                }
                if width == 88
                    && !matches!(
                        scenario,
                        "working"
                            | "history"
                            | "long-tools"
                            | "commands"
                            | "approval"
                            | "interview"
                    )
                {
                    assert!(visible.contains("@ files"));
                }
                let preview = format!(
                    "<section class={label}><h2>{scenario} · {label} · {width} columns</h2>{}</section>",
                    HtmlSurface::render(&frame.surface)
                );
                previews.push_str(&preview);
                if let Some(directory) = std::env::var_os("YO_TUI_PREVIEW_DIR") {
                    let directory = std::path::PathBuf::from(directory);
                    std::fs::create_dir_all(&directory).unwrap();
                    std::fs::write(
                        directory.join(format!("{scenario}-{label}-{width}.html")),
                        document(&preview),
                    )
                    .unwrap();
                    let blank = Surface::new(frame.surface.size()).unwrap();
                    let diff = FrameDiff::between(&blank, &frame.surface);
                    let mut encoder = AnsiEncoder::new(Vec::new());
                    encoder.encode(&TerminalOps::from_diff(&diff)).unwrap();
                    std::fs::write(
                        directory.join(format!("{scenario}-{label}-{width}.ansi")),
                        encoder.into_inner(),
                    )
                    .unwrap();
                }
            }
        }
    }
    if let Some(directory) = std::env::var_os("YO_TUI_PREVIEW_DIR") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("chat.html"), document(&previews)).unwrap();
    }
}

fn document(previews: &str) -> String {
    format!(
        "<!doctype html><html lang=en><meta charset=utf-8><title>yo chat — rendered fixtures</title><style>{} body{{--yo-default-background:#161a20;--yo-default-foreground:#d9dee7;background:var(--yo-default-background);color:var(--yo-default-foreground);font:15px/1.5 monospace;padding:32px}}section{{width:max-content;padding:24px;margin-bottom:32px;border:1px solid #343d49;border-radius:12px}}.light,.light-indexed{{--yo-default-background:#fafbfc;--yo-default-foreground:#243142;background:var(--yo-default-background);color:var(--yo-default-foreground)}}h2{{font:12px/1.4 monospace;color:#8995a5;margin:0 0 24px}}.yo-surface{{display:grid}}.yo-row{{height:1.5em}}.yo-glyph{{line-height:1.5}}</style>{previews}</html>",
        HtmlSurface::stylesheet()
    )
}
