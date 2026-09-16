use super::support::*;

// 터미널 입력의 fence·이미지·제어 문자는 실행되지 않고 폭 변경 뒤에도 코드 원문으로 남는다.
#[test]
fn terminal_input_document_keeps_markdown_and_controls_literal() {
    use std::{io::Cursor, time::Duration};

    use ::image::{ImageFormat, RgbImage};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use yo_core::{ActivityDocument, ActivityOutcome};

    use super::key;
    use crate::input::event::{KeyCode, KeyModifiers};

    let mut encoded = Cursor::new(Vec::new());
    RgbImage::new(8, 4)
        .write_to(&mut encoded, ImageFormat::Png)
        .unwrap();
    let input = format!(
        "```\n![literal](data:image/png;base64,{})\n# literal heading\n\u{3}\u{1b}[31m\n끝까지 보이는 입력",
        STANDARD.encode(encoded.into_inner())
    );
    let source = format!("````text\nProcess: test-7\nCommand: cargo test\nInput:\n{input}\n````");
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced);
    let state = session.parts_mut().state;
    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ModelWork,
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(
                ActivityDocument {
                    title: "Terminal input sent".to_owned(),
                    markdown: source,
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
    state
        .handle(
            key(KeyCode::Character('o'), KeyModifiers::CONTROL),
            Duration::ZERO,
        )
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    assert!(original.contains("![literal](data:image/png;base64,"));
    assert!(original.contains("# literal heading"));
    assert!(!original.contains(ActivityDocument::SCHEMA));
    let pin = session.appearance_pin();
    for width in [60, 24, 60] {
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 65), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(frame.surface.rasters.is_empty());
        assert!(text.contains("Terminal input sent"), "{text}");
        assert!(text.contains("```"), "{text}");
        assert!(text.contains("# literal heading"), "{text}");
        assert!(text.contains("^C^[[31m"), "{text}");
        assert!(
            text.chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
                .contains("끝까지보이는입력"),
            "{text}"
        );
        assert!(!text.contains(ActivityDocument::SCHEMA), "{text}");
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
}

// 문서 콜백은 문서에만 적용되고 펼침·폭·실패 상태를 받으며 교체·실패 대체·원문 보존을 지원한다.
#[test]
fn document_renderer_preserves_ownership_source_and_fallback() {
    use yo_core::{ActivityDocument, ActivityOutcome, ActivitySummary, Failure, SummaryKind};

    use super::key;
    use crate::{
        DocumentRenderer, OutputPreferences, TranscriptActivityOutcome,
        input::event::{KeyCode, KeyModifiers},
    };
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX))
        .with_document_renderer(Some(DocumentRenderer::new(|input| {
            assert_eq!(input.document.title, "Host document");
            assert_eq!(input.document.markdown, "Original document body");
            assert_eq!(input.outcome, Some(TranscriptActivityOutcome::Failed));
            assert!(input.columns.get() <= 80);
            Some(
                if input.expanded {
                    "Expanded detail"
                } else {
                    "Compact summary"
                }
                .into(),
            )
        })));
    for (id, source) in [
        (
            1,
            ActivityDocument {
                title: "Host document".into(),
                markdown: "Original document body".into(),
            }
            .to_snapshot()
            .unwrap(),
        ),
        (
            2,
            ActivitySummary {
                kind: SummaryKind::Reasoning,
                summary: "Keep public summary".into(),
                tokens_before: None,
            }
            .to_snapshot()
            .unwrap(),
        ),
    ] {
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(id),
                kind: ActivityKind::ModelWork,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(id),
                update: ActivityUpdate::TextSnapshot(source),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(id),
                outcome: ActivityOutcome::Failed(Failure::new("Retained failure")),
            })
            .unwrap();
    }
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
            .prepare_frame(Size::new(width, 40), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(!text.contains("yo.activity-document"));
        assert!(text.contains("Host document"));
        assert!(
            text.split_whitespace()
                .collect::<String>()
                .contains("Retainedfailure"),
            "{text}"
        );
        assert!(text.contains("Keep public summary"));
        assert_eq!(text.contains("Expanded detail"), expanded);
        assert_eq!(text.contains("Compact summary"), !expanded);
        session.parts_mut().state.commit_frame(&frame);
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
    for (renderer, expected) in [
        (
            Some(DocumentRenderer::new(|_| Some("Replacement body".into()))),
            "Replacement body",
        ),
        (
            Some(DocumentRenderer::new(|_| Some("\u{301}".into()))),
            "Original document body",
        ),
        (
            Some(DocumentRenderer::new(|_| {
                Some("x".repeat(yo_core::ToolOutput::MAX_SNAPSHOT_BYTES + 1))
            })),
            "Original document body",
        ),
        (
            Some(DocumentRenderer::new(|_| None)),
            "Original document body",
        ),
        (None, "Original document body"),
    ] {
        session = session.with_document_renderer(renderer);
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 40), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(!text.contains("yo.activity-document"));
        assert!(text.contains(expected), "{text}");
        assert!(
            text.split_whitespace()
                .collect::<String>()
                .contains("Retainedfailure"),
            "{text}"
        );
        session.parts_mut().state.commit_frame(&frame);
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
}

// 호스트 문서는 실제 poll 경계에서 Turn 없이 나타나고 커스텀 문서 렌더러·원문·개별 펼침을 유지한다.
#[test]
fn host_document_poll_renders_without_a_turn_and_keeps_original() {
    use std::time::Duration;

    use yo_core::ActivityDocument;

    use super::key;
    use crate::{
        DocumentRenderer, TuiDocument,
        input::event::{KeyCode, KeyModifiers},
        runner::{AgentPoll, unix::apply_agent_poll},
    };

    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_document_renderer(Some(DocumentRenderer::new(|input| {
            assert_eq!(input.document.title, "Host guide");
            Some(
                if input.expanded {
                    "**Expanded host guide**"
                } else {
                    "**Compact host guide**"
                }
                .into(),
            )
        })));
    let document = TuiDocument::new(ActivityDocument {
        title: "Host guide".into(),
        markdown: "Original **host** document\n\n```rust\nuse std::path::Path;\n```".into(),
    })
    .unwrap();
    assert!(apply_agent_poll(session.parts_mut().state, AgentPoll::Document(document)).unwrap());
    assert!(!session.parts_mut().state.turn_active());
    let source = session.session_output().unwrap().unwrap();
    assert!(source.contains("Original **host** document"));
    assert!(!source.contains("Compact host guide"));
    for (width, toggle, expanded) in [(80, false, false), (24, true, true), (80, false, true)] {
        if toggle {
            session
                .parts_mut()
                .state
                .handle(
                    key(KeyCode::Character('o'), KeyModifiers::ALT),
                    Duration::ZERO,
                )
                .unwrap();
        }
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 32), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert_eq!(text.contains("Expanded host guide"), expanded, "{text}");
        assert_eq!(text.contains("Compact host guide"), !expanded, "{text}");
        assert!(!text.contains("Working"));
        session.parts_mut().state.commit_frame(&frame);
        assert_eq!(session.session_output().unwrap().unwrap(), source);
    }
}

// 세션 문서 생성자는 기존 문서 wire 한도를 그대로 적용하고 첫 초과와 빈 제목을 거부한다.
#[test]
fn host_document_validates_before_publication() {
    use yo_core::{ActivityDocument, ToolOutput};

    use crate::TuiDocument;

    assert!(
        TuiDocument::new(ActivityDocument {
            title: String::new(),
            markdown: "body".into()
        })
        .is_none()
    );
    let mut document = ActivityDocument {
        title: "Guide".into(),
        markdown: String::new(),
    };
    let overhead = document.to_snapshot().unwrap().len();
    document.markdown = "x".repeat(ToolOutput::MAX_SNAPSHOT_BYTES - overhead);
    assert!(TuiDocument::new(document.clone()).is_some());
    document.markdown.push('x');
    assert!(TuiDocument::new(document).is_none());
}

// 호스트의 초기 펼침은 전역 상태와 독립적으로 적용되지만 이후 사용자 토글은 항상 우선한다.
#[test]
fn host_document_initial_expansion_is_optional_and_user_overridable() {
    use std::time::Duration;

    use yo_core::ActivityDocument;

    use super::key;
    use crate::{
        DocumentRenderer, TuiDocument,
        input::event::{KeyCode, KeyModifiers},
        runner::{AgentPoll, unix::apply_agent_poll},
    };

    for global in [false, true] {
        for initial in [None, Some(false), Some(true)] {
            let mut session =
                TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
                    .with_document_renderer(Some(DocumentRenderer::new(|input| {
                        Some(
                            if input.expanded {
                                "Expanded guide"
                            } else {
                                "Compact guide"
                            }
                            .into(),
                        )
                    })));
            if global {
                session
                    .parts_mut()
                    .state
                    .handle(
                        key(KeyCode::Character('o'), KeyModifiers::CONTROL),
                        Duration::ZERO,
                    )
                    .unwrap();
            }
            let document = TuiDocument::new(ActivityDocument {
                title: "Guide".into(),
                markdown: "Original source".into(),
            })
            .unwrap();
            let document = initial.map_or_else(
                || document.clone(),
                |expanded| document.clone().with_expanded(expanded),
            );
            apply_agent_poll(session.parts_mut().state, AgentPoll::Document(document)).unwrap();
            let original = session.session_output().unwrap();
            for (toggle, expected, width) in [
                (false, initial.unwrap_or(global), 80),
                (true, !initial.unwrap_or(global), 24),
                (false, !initial.unwrap_or(global), 80),
            ] {
                if toggle {
                    session
                        .parts_mut()
                        .state
                        .handle(
                            key(KeyCode::Character('o'), KeyModifiers::ALT),
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
                assert_eq!(
                    text.contains("Expanded guide"),
                    expected,
                    "{global}/{initial:?}: {text}"
                );
                assert_eq!(text.contains("Compact guide"), !expected, "{text}");
                session.parts_mut().state.commit_frame(&frame);
                assert_eq!(session.session_output().unwrap(), original);
            }
        }
    }
}

// 공급자가 전달한 추론은 공개 요약과 구분되며 숨김·폭 변경·커스텀 문서에도 원문과 실패를 보존한다.
#[test]
fn provider_reasoning_visibility_customization_and_export_preserve_source() {
    use yo_core::{ActivityOutcome, ActivityReasoning, Failure};

    use crate::{DocumentRenderer, OutputPreferences};
    for content in [
        serde_json::json!("**Original thought**"),
        serde_json::json!({"type":"future", "text":"![literal](data:image/png;base64,AA==)"}),
    ] {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX));
        let state = session.parts_mut().state;
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ModelWork,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(
                    ActivityReasoning {
                        content: content.clone(),
                    }
                    .to_snapshot()
                    .unwrap(),
                ),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: ActivityOutcome::Failed(Failure::new("Delivery stopped")),
            })
            .unwrap();
        let source = session.session_output().unwrap().unwrap();
        assert!(source.contains("Original thought") || source.contains("![literal]"));
        for custom in [false, true] {
            session = session.with_document_renderer(custom.then(|| {
                DocumentRenderer::new(|input| {
                    assert_eq!(input.document.title, "Agent reasoning");
                    assert!(
                        input.document.markdown.contains("Original thought")
                            || input.document.markdown.contains("![literal]")
                    );
                    Some("Custom reasoning".into())
                })
            }));
            for (columns, visible) in [(80, true), (24, false), (24, true), (80, false)] {
                session = session.with_output_preferences(
                    OutputPreferences::default()
                        .with_tool_head_rows(u16::MAX)
                        .with_reasoning(visible),
                );
                let pin = session.appearance_pin();
                let frame = session
                    .parts_mut()
                    .state
                    .prepare_frame(Size::new(columns, 35), &pin)
                    .unwrap();
                let text = visible_rows(&frame.surface);
                assert!(text.contains("Agent reasoning"), "{text}");
                assert!(!text.contains("Public reasoning summary"));
                assert_eq!(text.contains("Reasoning hidden"), !visible, "{text}");
                assert_eq!(
                    text.contains("Custom reasoning"),
                    visible && custom,
                    "{text}"
                );
                if !custom {
                    assert_eq!(
                        text.split_whitespace()
                            .collect::<String>()
                            .contains("Originalthought")
                            || text
                                .split_whitespace()
                                .collect::<String>()
                                .contains("![literal]"),
                        visible,
                        "{text}"
                    );
                }
                assert!(
                    text.split_whitespace()
                        .collect::<String>()
                        .contains("Deliverystopped"),
                    "{text}"
                );
                session.parts_mut().state.commit_frame(&frame);
                assert_eq!(session.session_output().unwrap().unwrap(), source);
            }
        }
    }
}
