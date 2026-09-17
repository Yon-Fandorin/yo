use super::*;

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

    use super::super::key;
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

    use super::super::key;
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

    use super::super::key;
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
