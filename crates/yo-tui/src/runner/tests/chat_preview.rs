use yo_core::{
    ActivityKind, ActivityUpdate, AgentCommand, AgentEvent, TranscriptRecord, UserInput,
};

use super::{activity, turn};
use crate::{
    ColorCapability, GlyphProfile, MotionPreference, PresentationMode, TuiSession, TuiSessionInfo,
    html::HtmlSurface,
    surface::{Attributes, CellContent, FrameDiff, Point, Size, Surface},
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
    for (profile, capability, label) in [
        (GlyphProfile::Rich, ColorCapability::TrueColor, "color"),
        (GlyphProfile::Ascii, ColorCapability::Unknown, "plain"),
    ] {
        for width in [20, 40, 88] {
            for scenario in ["welcome", "conversation", "working"] {
                let mut session = TuiSession::with_session_info(
                    profile,
                    TuiSessionInfo::new("Local Codex", "~/projects/yo"),
                    capability,
                    MotionPreference::Reduced,
                );
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
                            kind: ActivityKind::AgentMessage,
                        })
                        .unwrap();
                    state.observe(AgentEvent::ActivityUpdated {
                        activity: activity(1),
                        update: ActivityUpdate::TextSnapshot("I will separate your request from the response, keep the input visible,\nand make the next action clear.\n\nThe same layout works in narrow terminals and without color.".to_owned()),
                    }).unwrap();
                    if scenario == "working" {
                        state
                            .observe(AgentEvent::TurnStarted { turn: turn() })
                            .unwrap();
                    }
                }
                let pin = session.appearance_pin();
                let frame = session
                    .parts_mut()
                    .state
                    .prepare_frame(Size::new(width, 22), &pin)
                    .unwrap();
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
                    assert!(
                        frame
                            .surface
                            .cell(Point::new(2, 0))
                            .unwrap()
                            .style()
                            .attributes
                            .contains(Attributes::BOLD)
                    );
                }
                assert!(frame.cursor.x < width);
                assert!(frame.cursor.y < 22);
                if width == 88 && scenario != "working" {
                    assert!(visible.contains("@ files"));
                }
                let preview = format!(
                    "<section><h2>{scenario} · {label} · {width} columns</h2>{}</section>",
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
        "<!doctype html><html lang=en><meta charset=utf-8><title>yo chat — rendered fixtures</title><style>{} body{{--yo-default-background:#161a20;--yo-default-foreground:#d9dee7;background:var(--yo-default-background);color:var(--yo-default-foreground);font:15px/1.5 monospace;padding:32px}}section{{width:max-content;padding:24px;margin-bottom:32px;border:1px solid #343d49;border-radius:12px}}h2{{font:12px/1.4 monospace;color:#8995a5;margin:0 0 24px}}.yo-surface{{display:grid}}.yo-row{{height:1.5em}}.yo-glyph{{line-height:1.5}}</style>{previews}</html>",
        HtmlSurface::stylesheet()
    )
}
