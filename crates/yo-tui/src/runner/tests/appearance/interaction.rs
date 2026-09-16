use super::support::*;

// 검색·페이지 열기·찾기는 명확한 제목과 원문 패널로 표시하고 URL/패턴을 Markdown으로 실행하지
// 않는다. 테마·폭·콜백 변경 뒤에도 구조화 인수와 전체 원문을 보존한다.
#[test]
fn web_actions_render_literal_details_and_allow_typed_customization() {
    use std::num::NonZeroU16;

    use serde_json::json;
    use yo_core::{ActivityOutcome, ToolOutput};

    use crate::{OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole, ToolRenderer};

    for (heading, action, detail) in [
        (
            "Web search",
            json!({"type":"search","queries":["Rust 한글","configurable UI"]}),
            "Query: Rust 한글\nQuery: configurable UI",
        ),
        (
            "Open web page",
            json!({"type":"openPage","url":"https://example.com/docs"}),
            "URL: https://example.com/docs",
        ),
        (
            "Find in web page",
            json!({"type":"findInPage","pattern":"![literal](data:image/png;base64,AAAA)\n```rust\nunsafe text"}),
            "Find: ![literal](data:image/png;base64,AAAA)\n```rust\nunsafe text",
        ),
        ("Web search", json!(null), "Details not reported"),
    ] {
        let output = ToolOutput {
            tool: "webSearch".into(),
            server: None,
            arguments: Some(json!({"query":"fallback", "action":action})),
            result: None,
            content_items: None,
            error: None,
            plain_text: format!("{heading}\n{detail}"),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(
                OutputPreferences::default()
                    .with_tool_head_rows(u16::MAX)
                    .with_max_body_width(NonZeroU16::new(32)),
            )
            .with_theme_overrides(
                ThemeOverrides::default()
                    .with_color(ThemeRole::CodeBackground, ThemeColor::Rgb(11, 22, 33)),
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
        for width in [80, 24, 80] {
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 50), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let joined = text
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            assert!(joined.contains(&heading.replace(' ', "")), "{text}");
            assert!(
                joined.contains(
                    &detail
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .collect::<String>()
                ),
                "{text}"
            );
            assert!(
                !text.contains("Arguments") && !text.contains("yo.tool-output"),
                "{text}"
            );
            assert!(frame.surface.rasters.is_empty());
            assert!(
                (0..frame.surface.size().height).any(|y| (0..frame.surface.size().width).any(
                    |x| {
                        frame
                            .surface
                            .cell(Point::new(x, y))
                            .unwrap()
                            .style()
                            .background
                            == Color::Rgb {
                                red: 11,
                                green: 22,
                                blue: 33,
                            }
                    }
                ))
            );
        }
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: ActivityOutcome::Completed,
            })
            .unwrap();
        let pin = session.appearance_pin();
        let completed = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 50), &pin)
            .unwrap();
        let text = visible_rows(&completed.surface);
        assert!(text.contains("Tool completed"), "{text}");
        assert!(!text.contains("Tool call prepared"), "{text}");
        let expected = output.clone();
        session = session.with_tool_renderer(Some(ToolRenderer::new(move |input| {
            assert_eq!(input.output, Some(&expected));
            assert_eq!(input.source, expected.plain_text);
            assert!(input.columns.get() <= 32);
            Some("Custom search presentation".into())
        })));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 20), &pin)
            .unwrap();
        assert!(visible_rows(&frame.surface).contains("Custom search presentation"));
    }
}

// Markdown 링크 목적지는 문단·강조·인라인 코드·표의 개행과 좁은 폭에서도 셀에 보존된다.
// 클릭 설정을 끄면 글자·원문·테마를 바꾸지 않고 링크 정보만 제거한다.
#[test]
fn markdown_hyperlinks_survive_reflow_and_can_be_disabled() {
    use crate::OutputPreferences;
    let source = "[**한글** `code`](https://example.com/docs) after\n\n| Name | Link |\n| --- | --- |\n| one | [table](https://example.com/table) |\n\n```text\n[code](https://example.com/not-a-link)\n```\n\n[unsafe](javascript:alert(1)) [local](file:///tmp/a)";
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced);
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::AgentMessage,
        })
        .unwrap();
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
        let pin = session.appearance_pin();
        let linked = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 80), &pin)
            .unwrap();
        let mut linked_text = String::new();
        let mut destinations = Vec::new();
        for y in 0..linked.surface.size().height {
            for x in 0..linked.surface.size().width {
                let cell = linked.surface.cell(Point::new(x, y)).unwrap();
                if let Some(link) = cell.hyperlink() {
                    destinations.push(link.destination().to_owned());
                    if let CellContent::Grapheme { text, .. } = cell.content() {
                        linked_text.push_str(text);
                    }
                }
            }
        }
        assert!(linked_text.contains("한글"), "{linked_text}");
        assert!(linked_text.contains("code"), "{linked_text}");
        assert!(linked_text.contains("table"), "{linked_text}");
        assert!(!linked_text.contains("after"), "{linked_text}");
        assert!(
            destinations
                .iter()
                .any(|url| url == "https://example.com/docs")
        );
        assert!(
            destinations
                .iter()
                .any(|url| url == "https://example.com/table")
        );
        assert!(
            destinations
                .iter()
                .all(|url| url == "https://example.com/docs" || url == "https://example.com/table")
        );
        session =
            session.with_output_preferences(OutputPreferences::default().with_hyperlinks(false));
        let pin = session.appearance_pin();
        let unlinked = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 80), &pin)
            .unwrap();
        assert!(!unlinked.surface.has_hyperlinks());
        for y in 0..linked.surface.size().height {
            for x in 0..linked.surface.size().width {
                let before = linked.surface.cell(Point::new(x, y)).unwrap();
                let after = unlinked.surface.cell(Point::new(x, y)).unwrap();
                assert_eq!(before.content(), after.content());
                assert_eq!(before.style(), after.style());
            }
        }
        assert_eq!(session.session_output().unwrap().unwrap(), original);
        session =
            session.with_output_preferences(OutputPreferences::default().with_hyperlinks(true));
    }
}

// 시작·재개 안내는 실제 frame에서 좁은 폭에도 보이며 재그리기마다 중복되거나 Turn을 시작하지
// 않는다.
#[test]
fn host_startup_notice_survives_width_changes_without_starting_work() {
    for resumed in [false, true] {
        let mut session = TuiSession::with_session_info(
            GlyphProfile::Rich,
            TuiSessionInfo::new("host:demo · reported-model", "~/yo").with_startup_notice(resumed),
            ColorCapability::TrueColor,
            MotionPreference::Reduced,
        );
        let title = if resumed {
            "Session resumed"
        } else {
            "New session"
        };
        for width in [80, 24, 80] {
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 15), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            assert!(text.contains(title), "{text}");
            assert!(
                text.split_whitespace()
                    .collect::<String>()
                    .contains("Backend:host:demo·reported-model"),
                "{text}"
            );
            assert!(text.contains("Workspace: ~/yo"), "{text}");
            assert!(!session.parts_mut().state.turn_active());
            assert!(frame.motion_demand.is_none());
        }
        let output = session.session_output().unwrap().unwrap();
        assert_eq!(output.matches(title).count(), 1);
    }
}

// 질문·승인 요청과 응답은 활동 테마와 단어 줄바꿈을 사용하며 원문과 결과를 보존한다.
// 사용자 Markdown/이미지/링크 문자열은 실행 가능한 표시가 아닌 그대로의 답으로 남는다.
#[test]
fn interaction_text_uses_activity_theme_and_literal_reflow_without_tool_callbacks() {
    use std::num::NonZeroU64;

    use yo_core::{ActivityOutcome, Failure, RequestId};

    use crate::{ThemeColor, ThemeOverrides, ThemeRole, ToolRenderer};
    let source = "Question 1 of 2\nWhich area?\n\nAnswer: **Runtime**\nNote: [literal](https://example.com)\n![x](data:image/png;base64,abc)\n\nRecorded; waiting for the remaining questions.";
    let request_id = RequestId::new(NonZeroU64::new(7).unwrap());
    for (kind, heading, source, expected) in [
        (
            ActivityKind::UserInputResponse { request_id },
            "Answer recorded",
            source,
            "Answer:**Runtime**",
        ),
        (
            ActivityKind::ApprovalResponse { request_id },
            "Approval response sent",
            "Decision: approved",
            "Decision:approved",
        ),
        (
            ActivityKind::ApprovalResponse { request_id },
            "Approval response sent",
            "Decision: declined",
            "Decision:declined",
        ),
        (
            ActivityKind::ApprovalRequest { request_id },
            "Approval required",
            "Working directory: /workspace/yo\n    Preserve indentation\n**Literal** request",
            "Workingdirectory:/workspace/yo",
        ),
        (
            ActivityKind::UserInputRequest { request_id },
            "Answer requested",
            source,
            "Answer:**Runtime**",
        ),
    ] {
        for outcome in [
            ActivityOutcome::Completed,
            ActivityOutcome::Failed(Failure::new("receipt interrupted")),
        ] {
            let mut session =
                TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
                    .with_theme_overrides(
                        ThemeOverrides::default()
                            .with_color(ThemeRole::Success, ThemeColor::Rgb(80, 180, 90)),
                    )
                    .with_tool_renderer(Some(ToolRenderer::new(|_| {
                        panic!("responses never enter tool customization")
                    })));
            let state = session.parts_mut().state;
            state
                .observe(AgentEvent::ActivityStarted {
                    activity: activity(1),
                    kind,
                })
                .unwrap();
            state
                .observe(AgentEvent::ActivityUpdated {
                    activity: activity(1),
                    update: ActivityUpdate::TextSnapshot(source.into()),
                })
                .unwrap();
            state
                .observe(AgentEvent::ActivityFinished {
                    activity: activity(1),
                    outcome: outcome.clone(),
                })
                .unwrap();
            let pin = session.appearance_pin();
            for width in [80, 24, 80] {
                let frame = session
                    .parts_mut()
                    .state
                    .prepare_frame(Size::new(width, 50), &pin)
                    .unwrap();
                let text = visible_rows(&frame.surface);
                if matches!(kind, ActivityKind::ApprovalRequest { .. }) {
                    assert!(
                        text.lines().any(|line| line.contains("/workspace/yo")),
                        "{text}"
                    );
                }
                let compact: String = text.split_whitespace().collect();
                assert!(
                    compact.contains(&heading.split_whitespace().collect::<String>()),
                    "{text}"
                );
                assert!(compact.contains(expected), "{text}");
                let y = (0..50)
                    .find(|y| {
                        (0..width)
                            .filter_map(|x| {
                                match frame.surface.cell(Point::new(x, *y)).unwrap().content() {
                                    CellContent::Grapheme { text, .. } => Some(text.as_ref()),
                                    _ => None,
                                }
                            })
                            .collect::<String>()
                            .contains(heading.split_whitespace().next().unwrap())
                    })
                    .unwrap();
                let style = frame.surface.cell(Point::new(2, y)).unwrap().style();
                assert!(style.attributes.contains(Attributes::BOLD));
                if outcome == ActivityOutcome::Completed {
                    assert_eq!(
                        style.foreground,
                        Color::Rgb {
                            red: 80,
                            green: 180,
                            blue: 90
                        }
                    );
                }
                for y in 0..50 {
                    for x in 0..width {
                        assert!(
                            frame
                                .surface
                                .cell(Point::new(x, y))
                                .unwrap()
                                .hyperlink()
                                .is_none()
                        );
                    }
                }
            }
            let plain = session.session_output().unwrap().unwrap();
            let unindented = plain
                .lines()
                .map(|line| line.strip_prefix("  ").unwrap_or(line))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(unindented.contains(source), "{plain}");
            if matches!(outcome, ActivityOutcome::Failed(_)) {
                assert!(plain.contains("Failed: receipt interrupted"));
            }
        }
    }
}

// 호스트가 확인한 명시적 Markdown 링크만 파일 목적지로 바꾸며 코드·이미지·원문은 그대로 둔다.
// 폭 변경, resolver 교체·제거, 링크 비활성화 후에도 기본 웹 링크와 원문 보존을 검증한다.
#[test]
fn host_link_resolver_preserves_default_web_links_and_source() {
    use std::{
        path::Path,
        sync::{Arc, Mutex},
    };

    use crate::{LinkResolver, OutputPreferences, surface::Hyperlink};
    let file = Hyperlink::from_file_path(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("Cargo.toml")
            .canonicalize()
            .unwrap(),
    )
    .unwrap();
    let replacement = Hyperlink::from_file_path(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/lib.rs")
            .canonicalize()
            .unwrap(),
    )
    .unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&calls);
    let target = file.clone();
    let resolver = LinkResolver::new(move |destination| {
        observed.lock().unwrap().push(destination.to_owned());
        (destination == "Cargo.toml").then(|| target.clone())
    });
    let source = "[**한글** `manifest`](Cargo.toml) [web](https://example.com/docs)\n\n| Name | File |\n| --- | --- |\n| row | [table](Cargo.toml) |\n\n```text\n[code](code-only)\n```\n\n![image](image-only) [unmapped](missing.rs) [raw](file:///tmp/untrusted)";
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_link_resolver(Some(resolver));
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::AgentMessage,
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(source.into()),
        })
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    assert!(calls.lock().unwrap().is_empty());
    for width in [80, 24, 80] {
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 90), &pin)
            .unwrap();
        let mut linked_text = String::new();
        let mut destinations = Vec::new();
        for y in 0..frame.surface.size().height {
            for x in 0..frame.surface.size().width {
                let cell = frame.surface.cell(Point::new(x, y)).unwrap();
                if let Some(link) = cell.hyperlink() {
                    destinations.push(link.destination().to_owned());
                    if link == &file
                        && let CellContent::Grapheme { text, .. } = cell.content()
                    {
                        linked_text.push_str(text);
                    }
                }
            }
        }
        assert!(linked_text.contains("한글"));
        assert!(linked_text.contains("manifest"));
        assert!(linked_text.contains("table"));
        assert!(
            destinations
                .iter()
                .any(|value| value == "https://example.com/docs")
        );
        assert!(
            destinations
                .iter()
                .all(|value| value == file.destination() || value == "https://example.com/docs")
        );
        session.parts_mut().state.commit_frame(&frame);
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
    assert!(
        calls
            .lock()
            .unwrap()
            .iter()
            .all(|value| value != "code-only" && value != "image-only")
    );
    let next = replacement.clone();
    session = session.with_link_resolver(Some(LinkResolver::new(move |destination| {
        (destination == "Cargo.toml").then(|| next.clone())
    })));
    let pin = session.appearance_pin();
    let frame = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(80, 90), &pin)
        .unwrap();
    let links = |surface: &Surface| {
        (0..surface.size().height)
            .flat_map(|y| {
                (0..surface.size().width).filter_map(move |x| {
                    surface.cell(Point::new(x, y)).unwrap().hyperlink().cloned()
                })
            })
            .collect::<Vec<_>>()
    };
    assert!(links(&frame.surface).contains(&replacement));
    assert!(!links(&frame.surface).contains(&file));
    session.parts_mut().state.commit_frame(&frame);
    session = session.with_link_resolver(Some(LinkResolver::new(|_| {
        panic!("disabled links must not resolve")
    })));
    session = session.with_output_preferences(OutputPreferences::default().with_hyperlinks(false));
    let pin = session.appearance_pin();
    let disabled = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(80, 90), &pin)
        .unwrap();
    assert!(!disabled.surface.has_hyperlinks());
    assert_eq!(
        visible_rows(&disabled.surface),
        visible_rows(&frame.surface)
    );
    assert_eq!(session.session_output().unwrap().unwrap(), original);
    session = session
        .with_link_resolver(None)
        .with_output_preferences(OutputPreferences::default().with_hyperlinks(true));
    let pin = session.appearance_pin();
    let default = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(80, 90), &pin)
        .unwrap();
    assert!(
        links(&default.surface)
            .iter()
            .all(|link| link.destination() == "https://example.com/docs")
    );
}

// 실행 호스트 링크 이벤트는 실제 appearance 경계에서 폭 재배치·매핑 교체·해제를 적용하고 원문을
// 유지한다.
#[test]
fn live_host_link_events_replace_and_clear_rendered_destinations() {
    use crate::{
        LinkResolver,
        runner::{AgentPoll, unix::apply_host_poll},
        surface::Hyperlink,
    };
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
            update: ActivityUpdate::TextSnapshot("[Workspace file](file.rs)".into()),
        })
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    for (width, path) in [
        (80, Some("/tmp/first.rs")),
        (24, Some("/tmp/second.rs")),
        (80, None),
    ] {
        let target = path.and_then(|path| Hyperlink::from_file_path(path::Path::new(path)));
        let expected = target.clone();
        let resolver = target.map(|target| {
            LinkResolver::new(move |destination| (destination == "file.rs").then(|| target.clone()))
        });
        let parts = session.parts_mut();
        assert!(
            apply_host_poll(parts.state, parts.appearance, AgentPoll::Links(resolver)).unwrap()
        );
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 20), &pin)
            .unwrap();
        let mut links = Vec::new();
        for y in 0..20 {
            for x in 0..width {
                if let Some(link) = frame.surface.cell(Point::new(x, y)).unwrap().hyperlink() {
                    links.push(link.clone());
                }
            }
        }
        if let Some(expected) = expected {
            assert!(!links.is_empty());
            assert!(links.iter().all(|link| link == &expected));
        } else {
            assert!(links.is_empty());
        }
        session.parts_mut().state.commit_frame(&frame);
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
}
