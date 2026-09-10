//! Persistent offline visual check without a test harness writing into the terminal.
//! Run with `cargo run -p yo-tui --example chat_preview`, then enter `/preview`.

#[cfg(unix)]
mod unix {
    use std::{
        io,
        task::{Context, Poll},
    };

    use yo_core::{SubmissionRejection, SubmissionRejectionKind};
    use yo_tui::{
        AgentAction, AgentConnection, AgentPoll, DispatchOutcome, PendingDispatch,
        TerminationEvent, TerminationSource,
    };

    pub struct Offline;

    impl AgentConnection for Offline {
        type Error = io::Error;

        fn dispatch(&mut self, action: AgentAction) -> io::Result<DispatchOutcome> {
            match action {
                AgentAction::Submit(submission) | AgentAction::Steer { submission, .. } => {
                    Ok(DispatchOutcome::Rejected {
                        id: submission.id(),
                        rejection: SubmissionRejection::new(
                            SubmissionRejectionKind::Incompatible,
                            "Enter /preview to open the offline UI examples.",
                        ),
                    })
                },
                _ => Ok(DispatchOutcome::Queued),
            }
        }

        fn retry(&mut self, _: PendingDispatch) -> io::Result<DispatchOutcome> {
            Err(io::Error::other(
                "offline preview never queues provider requests",
            ))
        }

        fn poll(&mut self) -> io::Result<AgentPoll> {
            Ok(AgentPoll::Pending)
        }
        fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<()> {
            Poll::Pending
        }
    }

    impl TerminationSource for Offline {
        fn poll_termination(&mut self, _: &mut Context<'_>) -> Poll<TerminationEvent> {
            Poll::Pending
        }
    }
}

#[cfg(unix)]
fn main() {
    use std::{env, path::Path, process};

    use yo_tui::{
        AssistantRenderer, ColorCapability, DocumentRenderer, GlyphProfile, LinkResolver,
        MotionPreference, OutputPreferences, PresentationMode, ThemeColor, ThemeOverrides,
        ThemeRole, ToolRenderer, TranscriptActivityOutcome, TuiSession, TuiSessionInfo,
        run_session_with_mode, surface::Hyperlink,
    };

    let mut session = TuiSession::with_session_info(
        GlyphProfile::Rich,
        TuiSessionInfo::new("OFFLINE UI", "Enter /preview · no model connection"),
        ColorCapability::TrueColor,
        MotionPreference::Reduced,
    );
    let mut preferences = OutputPreferences::default();
    let mut theme_overrides = ThemeOverrides::default();
    for argument in env::args().skip(1) {
        match argument.as_str() {
            "--custom-colors" => {
                theme_overrides = theme_overrides
                    .with_color(ThemeRole::Accent, ThemeColor::Rgb(172, 156, 224))
                    .with_color(ThemeRole::Chart, ThemeColor::Rgb(230, 160, 40))
                    .with_color(ThemeRole::Chart2, ThemeColor::Rgb(52, 182, 216))
                    .with_color(ThemeRole::Chart3, ThemeColor::Rgb(180, 159, 215))
                    .with_color(ThemeRole::Chart4, ThemeColor::Rgb(151, 201, 156))
                    .with_color(ThemeRole::UserBackground, ThemeColor::Rgb(38, 35, 51))
                    .with_color(ThemeRole::CodeBackground, ThemeColor::Rgb(29, 29, 41))
                    .with_color(ThemeRole::CodeHeaderBackground, ThemeColor::Rgb(45, 42, 63))
                    .with_color(ThemeRole::DiffAddedBackground, ThemeColor::Rgb(28, 48, 43))
                    .with_color(
                        ThemeRole::DiffRemovedBackground,
                        ThemeColor::Rgb(57, 34, 46),
                    );
            },
            "--terminal-code-text" => {
                theme_overrides =
                    theme_overrides.with_color(ThemeRole::CodeText, ThemeColor::Terminal);
            },
            "--hide-reasoning" => {
                preferences = preferences.with_reasoning(false);
            },
            value if value.starts_with("--code-padding=") => {
                let padding = value
                    .trim_start_matches("--code-padding=")
                    .parse::<u16>()
                    .unwrap_or_else(|_| {
                        eprintln!("code padding must be an integer from 0 to 65535 (capped at 8)");
                        process::exit(2);
                    });
                preferences = preferences.with_code_padding(padding);
            },
            "--custom-links" => {
                let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../README.md")
                    .canonicalize()
                    .expect("preview README exists");
                let target = Hyperlink::from_file_path(&path)
                    .expect("preview README is a bounded local path");
                session = session.with_link_resolver(Some(LinkResolver::new(move |destination| {
                    (destination == "README.md").then(|| target.clone())
                })));
            },
            "--custom-answers" => {
                session = session.with_assistant_renderer(Some(AssistantRenderer::new(|input| {
                    input.source.starts_with("## Clearer output").then(|| {
                        format!(
                            "{}\n\n*Custom answer view · {} columns · {}*",
                            input.source,
                            input.columns,
                            if input.finalized {
                                "final"
                            } else {
                                "streaming"
                            }
                        )
                    })
                })));
            },
            "--custom-documents" => {
                session = session.with_document_renderer(Some(DocumentRenderer::new(|input| {
                    (input.document.title == "Waited for background terminal").then(|| {
                        if input.expanded {
                            format!("**Host document view**\n\n{}", input.document.markdown)
                        } else {
                            "**Host document view**\n\nOffline terminal status. Ctrl+O for detail."
                                .to_owned()
                        }
                    })
                })));
            },
            "--custom-tools" => {
                // This offline fixture has a known textual prefix; real hosts should
                // match their own adapter contract, not infer a universal tool schema.
                session = session.with_tool_renderer(Some(ToolRenderer::new(|input| {
                    let body = input.source.strip_prefix("docs.search\n")?;
                    let (arguments, result) = body.split_once("\nResult:\n")?;
                    let arguments = arguments.strip_prefix("Arguments:\n")?;
                    let (result, structured) = result.split_once("\n\nstructuredContent:\n")?;
                    if !input.expanded {
                        let summary = match input.outcome {
                            Some(TranscriptActivityOutcome::Failed) => "Search failed; partial result available.",
                            Some(TranscriptActivityOutcome::Interrupted) => "Search interrupted; partial result available.",
                            _ => result.lines().next().unwrap_or("No text result"),
                        };
                        return Some(format!(
                            "**docs.search** · {}\n\nCtrl+O for arguments and structured result.",
                            summary
                        ));
                    }
                    Some(format!(
                        "**docs.search**\n\n```yaml\n{arguments}\n```\n\n{}\n\n```json\n{structured}\n```",
                        result.replace('\n', "  \n")
                    ))
                })));
            },
            "--help" => {
                println!(
                    "chat_preview [--custom-colors] [--custom-tools] [--custom-documents] [--custom-answers] [--custom-links] [--hide-reasoning] [--code-padding=N] [--terminal-code-text]\nThen enter /preview for offline examples."
                );
                return;
            },
            _ => {
                eprintln!("Unknown preview option: {argument}");
                process::exit(2);
            },
        }
    }
    session = session
        .with_output_preferences(preferences)
        .with_theme_overrides(theme_overrides);
    run_session_with_mode(
        &mut unix::Offline,
        &mut unix::Offline,
        &mut session,
        PresentationMode::Fullscreen,
    )
    .expect("offline terminal preview");
}

#[cfg(not(unix))]
fn main() {
    eprintln!("This terminal preview requires Unix.");
}
