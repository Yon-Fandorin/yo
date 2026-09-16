use super::support::*;

// 경고 안내는 완료 상태로 바뀌지 않고 warning palette·개행을 유지하며 원문 내보내기도 읽을 수 있다.
#[test]
fn retry_and_limit_notices_keep_warning_style_after_delivery() {
    use yo_core::{ActivityNotice, ActivityOutcome, Failure, NoticeLevel};

    use crate::{ThemeColor, ThemeOverrides, ThemeRole};

    for notice in [ActivityNotice {
        title: "Retry announced".to_owned(),
        message: "temporary disconnect\nCodex will retry.".to_owned(),
        level: NoticeLevel::Warning,
    }, ActivityNotice {
        title: "Response limit reached".to_owned(),
        message: "The response stopped before completion.\nPartial answer text is retained. Unfinished tool calls were not executed.".to_owned(),
        level: NoticeLevel::Warning,
    }, ActivityNotice {
        title: "Model rerouted".to_owned(),
        message: "Codex reported a model change.\nFrom: requested\nTo: actual\nReason: highRiskCyberActivity".to_owned(),
        level: NoticeLevel::Warning,
    }] {
    for outcome in [
        ActivityOutcome::Completed,
        ActivityOutcome::Interrupted,
        ActivityOutcome::Failed(Failure::new("notice delivery failed")),
    ] {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_theme_overrides(
                ThemeOverrides::default()
                    .with_color(ThemeRole::Warning, ThemeColor::Rgb(210, 170, 90)),
            );
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ModelWork,
            })
            .unwrap();
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(notice.to_snapshot().unwrap()),
            })
            .unwrap();
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextDelta(String::new()),
            })
            .unwrap();
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: outcome.clone(),
            })
            .unwrap();
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(40, 20), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(text.contains(&notice.title), "{text}");
        assert!(text.contains(&notice.message.chars().take(20).collect::<String>()), "{text}");
        assert!(!text.contains("Model work completed"), "{text}");
        let title = frame.surface.cell(Point::new(2, 0)).unwrap();
        assert_eq!(
            title.style().foreground,
            Color::Rgb {
                red: 210,
                green: 170,
                blue: 90
            }
        );
        assert!(title.style().attributes.contains(Attributes::BOLD));
        let plain = session.session_output().unwrap().unwrap();
        assert!(plain.contains(&notice.title));
        assert!(!plain.contains(ActivityNotice::SCHEMA));
        if matches!(outcome, ActivityOutcome::Interrupted) {
            assert!(plain.contains("Interrupted"));
        }
        if matches!(outcome, ActivityOutcome::Failed(_)) {
            assert!(plain.contains("notice delivery failed"));
        }
    }
    }
}

// 시작 경고는 가짜 Turn 없이 표시하고 경고 색상·문자 원문을 보존하며 Working을 시작하지 않는다.
#[test]
fn session_notice_renders_before_any_turn_with_custom_warning_style() {
    use yo_core::{ActivityNotice, NoticeLevel};

    use crate::{ThemeColor, ThemeOverrides, ThemeRole};

    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_theme_overrides(
            ThemeOverrides::default().with_color(ThemeRole::Warning, ThemeColor::Rgb(210, 170, 90)),
        );
    session
        .parts_mut()
        .state
        .observe_notice(ActivityNotice {
            title: "Configuration warning".to_owned(),
            message: "Literal **setting**\nFile: config.toml".to_owned(),
            level: NoticeLevel::Warning,
        })
        .unwrap();
    assert!(!session.parts_mut().state.turn_active());
    let pin = session.appearance_pin();
    let frame = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(40, 20), &pin)
        .unwrap();
    let text = visible_rows(&frame.surface);
    assert!(text.contains("Configuration warning"), "{text}");
    assert!(text.contains("Literal **setting**"), "{text}");
    assert!(text.contains("File: config.toml"), "{text}");
    assert!(!text.contains("Working"), "{text}");
    assert!(!text.contains("Thinking"), "{text}");
    assert!(frame.motion_demand.is_none());
    let title = frame.surface.cell(Point::new(2, 0)).unwrap();
    assert_eq!(
        title.style().foreground,
        Color::Rgb {
            red: 210,
            green: 170,
            blue: 90
        }
    );
    assert!(title.style().attributes.contains(Attributes::BOLD));
    let plain = session.session_output().unwrap().unwrap();
    assert!(plain.contains("Literal **setting**"));
    assert!(!plain.contains(ActivityNotice::SCHEMA));
}

// 압축 안내는 같은 항목의 시작 문구를 완료 snapshot으로 교체하고 accent 설정과 원문을 유지한다.
#[test]
fn context_compaction_updates_one_styled_notice_without_thinking_or_raw_json() {
    use yo_core::{ActivityNotice, ActivityOutcome, NoticeLevel};

    use crate::{ThemeColor, ThemeOverrides, ThemeRole};

    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_theme_overrides(
            ThemeOverrides::default().with_color(ThemeRole::Accent, ThemeColor::Rgb(90, 170, 210)),
        );
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ModelWork,
        })
        .unwrap();
    for title in ["Compacting context", "Context compacted"] {
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(
                    ActivityNotice {
                        title: title.to_owned(),
                        message: "Conversation context".to_owned(),
                        level: NoticeLevel::Info,
                    }
                    .to_snapshot()
                    .unwrap(),
                ),
            })
            .unwrap();
        if title == "Context compacted" {
            session
                .parts_mut()
                .state
                .observe(AgentEvent::ActivityFinished {
                    activity: activity(1),
                    outcome: ActivityOutcome::Completed,
                })
                .unwrap();
        }
        assert_eq!(session.parts_mut().state.transcript().items().len(), 1);
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(40, 20), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(text.contains(title), "{text}");
        assert!(!text.contains("Thinking"), "{text}");
        assert!(!text.contains("Model work completed"), "{text}");
        assert!(!text.contains(ActivityNotice::SCHEMA), "{text}");
        let heading = frame.surface.cell(Point::new(2, 0)).unwrap();
        assert_eq!(
            heading.style().foreground,
            Color::Rgb {
                red: 90,
                green: 170,
                blue: 210
            }
        );
        assert!(heading.style().attributes.contains(Attributes::BOLD));
    }
    let plain = session.session_output().unwrap().unwrap();
    assert!(plain.contains("Context compacted"));
    assert!(!plain.contains("Compacting context"));
}

// 압축·분기 요약은 접어도 원문과 실패 footer를 유지하며 Ctrl+O로 코드 강조까지 복원한다.
#[test]
fn activity_summary_folds_markdown_preserving_source_and_outcome() {
    use std::time::Duration;

    use yo_core::{ActivityOutcome, ActivitySummary, Failure, SummaryKind};

    use super::key;
    use crate::{
        OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole,
        input::event::{KeyCode, KeyModifiers},
    };

    for kind in [SummaryKind::Compaction, SummaryKind::Branch] {
        let summary = ActivitySummary {
            kind,
            summary: format!(
                "## Decisions\n\n{}\n\n```rust\nuse std::fmt;\n```",
                (0..20)
                    .map(|n| format!("- retained decision {n:02}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            tokens_before: (kind == SummaryKind::Compaction).then_some(12000),
        };
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(1))
            .with_theme_overrides(
                ThemeOverrides::default()
                    .with_color(ThemeRole::Accent, ThemeColor::Rgb(90, 170, 210)),
            );
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
                update: ActivityUpdate::TextSnapshot(summary.to_snapshot().unwrap()),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextDelta(String::new()),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: ActivityOutcome::Failed(Failure::new("delivery stopped")),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        for line in summary.summary.lines() {
            assert!(original.contains(line), "{original}");
        }
        assert!(!original.contains(ActivitySummary::SCHEMA));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(60, 55), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(text.contains("Ctrl+O expand"), "{text}");
        assert!(!text.contains("retained decision 10"), "{text}");
        assert!(text.contains("delivery stopped"), "{text}");
        assert_eq!(
            text.contains("12000 tokens"),
            kind == SummaryKind::Compaction
        );
        let heading = frame.surface.cell(Point::new(2, 0)).unwrap();
        assert_eq!(
            heading.style().foreground,
            Color::Rgb {
                red: 90,
                green: 170,
                blue: 210
            }
        );
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
            .prepare_frame(Size::new(60, 55), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(text.contains("retained decision 10"), "{text}");
        assert!(text.contains("use std::fmt;"), "{text}");
        assert!(!text.contains("Ctrl+O expand"), "{text}");
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
}

// 빈 요약은 안내를 남기고 좁은 폭에서도 schema를 노출하지 않으며 Markdown 이미지를 첨부로 실행하지
// 않는다.
#[test]
fn activity_summary_empty_and_narrow_image_fallbacks_are_visible() {
    use std::{io::Cursor, time::Duration};

    use ::image::{ImageFormat, RgbImage};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use yo_core::{ActivityOutcome, ActivitySummary, SummaryKind};

    use super::key;
    use crate::input::event::{KeyCode, KeyModifiers};
    let mut encoded = Cursor::new(Vec::new());
    RgbImage::new(120, 60)
        .write_to(&mut encoded, ImageFormat::Png)
        .unwrap();
    let source_image = format!(
        "![untrusted](data:image/png;base64,{})",
        STANDARD.encode(encoded.into_inner())
    );
    for source in ["", source_image.as_str()] {
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
                    ActivitySummary {
                        kind: SummaryKind::Branch,
                        summary: source.to_owned(),
                        tokens_before: None,
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
        session
            .parts_mut()
            .state
            .handle(
                key(KeyCode::Character('o'), KeyModifiers::CONTROL),
                Duration::ZERO,
            )
            .unwrap();
        let pin = session.appearance_pin();
        for width in [4, 24, 60] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 40), &pin)
                .unwrap();
            assert!(frame.surface.rasters.is_empty());
            let text = visible_rows(&frame.surface);
            assert!(!text.contains(ActivitySummary::SCHEMA), "{text}");
            if source.is_empty() && width == 60 {
                assert!(text.contains("No summary text was provided."), "{text}");
            }
        }
        let plain = session.session_output().unwrap().unwrap();
        assert!(
            plain
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>()
                .contains(source)
        );
    }
}

// 공개 요약만 숨기고 계획·경고·압축 요약과 실패 footer는 유지하며 설정을 바꾸면 원문을 복원한다.
#[test]
fn public_reasoning_visibility_preserves_other_activity_and_plain_export() {
    use yo_core::{
        ActivityNotice, ActivityOutcome, ActivitySummary, Failure, NoticeLevel, SummaryKind,
    };

    use crate::OutputPreferences;

    let populate = |session: &mut TuiSession| {
        for (id, text) in [
            (
                1,
                ActivitySummary {
                    kind: SummaryKind::Reasoning,
                    summary: "**Visible reasoning body**".to_owned(),
                    tokens_before: None,
                }
                .to_snapshot()
                .unwrap(),
            ),
            (
                2,
                ActivitySummary {
                    kind: SummaryKind::Compaction,
                    summary: "Keep compaction visible".to_owned(),
                    tokens_before: Some(900),
                }
                .to_snapshot()
                .unwrap(),
            ),
            (
                3,
                ActivityNotice {
                    title: "Keep warning visible".to_owned(),
                    message: "Configuration issue".to_owned(),
                    level: NoticeLevel::Warning,
                }
                .to_snapshot()
                .unwrap(),
            ),
            (4, "Plan\n- Keep plan visible".to_owned()),
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
                    update: ActivityUpdate::TextSnapshot(text),
                })
                .unwrap();
            state
                .observe(AgentEvent::ActivityFinished {
                    activity: activity(id),
                    outcome: if id == 1 {
                        ActivityOutcome::Failed(Failure::new("summary delivery stopped"))
                    } else {
                        ActivityOutcome::Completed
                    },
                })
                .unwrap();
        }
    };
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_output_preferences(OutputPreferences::default().with_reasoning(false));
    populate(&mut session);
    let source = session.session_output().unwrap().unwrap();
    assert!(source.contains("**Visible reasoning body**"));
    for visible in [false, true, false] {
        session =
            session.with_output_preferences(OutputPreferences::default().with_reasoning(visible));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(60, 45), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert_eq!(text.contains("Visible reasoning body"), visible, "{text}");
        assert_eq!(text.contains("Summary hidden"), !visible, "{text}");
        for retained in [
            "Keep compaction visible",
            "Keep warning visible",
            "Keep plan visible",
            "summary delivery stopped",
        ] {
            assert!(text.contains(retained), "{text}");
        }
        assert!(!text.contains(ActivitySummary::SCHEMA), "{text}");
        assert_eq!(session.session_output().unwrap().unwrap(), source);
    }
}

// 실제 다이어그램 프레임은 코드 팔레트를 사용하고 표시 설정·폭을 바꿔도 내보낼 Mermaid 원문을
// 유지한다.
#[test]
fn mermaid_frame_preferences_preserve_source_and_palette() {
    use yo_core::ActivityOutcome;

    use crate::{OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole};
    let source = "```mermaid\ngraph TD; A[Build] --> B[Test]\n```";
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_theme_overrides(
            ThemeOverrides::default()
                .with_color(ThemeRole::CodeBackground, ThemeColor::Rgb(25, 35, 45)),
        );
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
            update: ActivityUpdate::TextSnapshot(source.to_owned()),
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityFinished {
            activity: activity(1),
            outcome: ActivityOutcome::Completed,
        })
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    assert!(original.contains("graph TD; A[Build] --> B[Test]"));
    for (enabled, width) in [(true, 80), (false, 80), (true, 16), (true, 80)] {
        session =
            session.with_output_preferences(OutputPreferences::default().with_diagrams(enabled));
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 40), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        if enabled && width == 80 {
            assert!(!text.contains("graph TD"), "{text}");
            assert!(text.contains("Build") && text.contains("Test"), "{text}");
            let row = (0..40).find(|y| {
                (0..width).any(|x| matches!(frame.surface.cell(Point::new(x,*y)).unwrap().content(), CellContent::Grapheme {text,..} if text.as_ref() == "│"))
            }).expect("diagram connector row");
            assert_eq!(
                frame
                    .surface
                    .cell(Point::new(3, row))
                    .unwrap()
                    .style()
                    .background,
                Color::Rgb {
                    red: 25,
                    green: 35,
                    blue: 45
                }
            );
        } else if !enabled {
            assert!(text.contains("graph TD"), "{text}");
        }
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
}

// 계획은 상태별 팔레트·본문 들여쓰기를 유지하고 실패·완료가 단계 상태나 문자 원문을 바꾸지 않는다.
#[test]
fn activity_plan_styles_wraps_and_preserves_reported_progress() {
    use yo_core::{ActivityOutcome, ActivityPlan, Failure, PlanStep, PlanStepStatus};

    use crate::{ThemeColor, ThemeOverrides, ThemeRole};
    let plan = ActivityPlan {
        explanation: Some("**literal explanation**".to_owned()),
        steps: vec![
            PlanStep {
                text: "Completed".to_owned(),
                status: PlanStepStatus::Completed,
            },
            PlanStep {
                text: "ABCDEFGHIJKLMNO".to_owned(),
                status: PlanStepStatus::InProgress,
            },
            PlanStep {
                text: "Pending".to_owned(),
                status: PlanStepStatus::Pending,
            },
        ],
    };
    for outcome in [
        ActivityOutcome::Completed,
        ActivityOutcome::Failed(Failure::new("plan delivery failed")),
    ] {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_theme_overrides(
                ThemeOverrides::default()
                    .with_color(ThemeRole::Success, ThemeColor::Rgb(80, 180, 90))
                    .with_color(ThemeRole::Accent, ThemeColor::Rgb(90, 140, 220)),
            );
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
                update: ActivityUpdate::TextSnapshot(plan.to_snapshot().unwrap()),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextDelta(String::new()),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: outcome.clone(),
            })
            .unwrap();
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(20, 40), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(text.contains("[x] Completed"), "{text}");
        assert!(text.contains("[>] ABCDEFGHIJKLMN"), "{text}");
        assert!(text.contains("      O"), "{text}");
        assert!(text.contains("[ ] Pending"), "{text}");
        for (needle, color, bold) in [
            (
                "[x]",
                Color::Rgb {
                    red: 80,
                    green: 180,
                    blue: 90,
                },
                false,
            ),
            (
                "[>]",
                Color::Rgb {
                    red: 90,
                    green: 140,
                    blue: 220,
                },
                true,
            ),
        ] {
            let y = (0..40)
                .find(|y| {
                    let row = (0..20)
                        .filter_map(|x| {
                            match frame.surface.cell(Point::new(x, *y)).unwrap().content() {
                                CellContent::Grapheme { text, .. } => Some(text.as_ref()),
                                _ => None,
                            }
                        })
                        .collect::<String>();
                    row.contains(needle)
                })
                .unwrap();
            let style = frame.surface.cell(Point::new(2, y)).unwrap().style();
            assert_eq!(style.foreground, color);
            if bold {
                assert!(style.attributes.contains(Attributes::BOLD));
            }
        }
        let plain = session.session_output().unwrap().unwrap();
        assert!(plain.contains("1/3 completed"), "{plain}");
        assert!(plain.contains("**literal explanation**"), "{plain}");
        assert!(plain.contains("ABCDEFGHIJKLMNO"), "{plain}");
        assert!(!plain.contains(ActivityPlan::SCHEMA));
        if matches!(outcome, ActivityOutcome::Failed(_)) {
            assert!(
                text.chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
                    .contains("plandeliveryfailed"),
                "{text}"
            );
        }
    }
}

// 빈 계획은 0/0 진행률을 만들지 않고 명시적 안내를 표시한다.
#[test]
fn activity_plan_empty_steps_have_an_explicit_fallback() {
    use yo_core::{ActivityOutcome, ActivityPlan};
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
                ActivityPlan {
                    explanation: None,
                    steps: Vec::new(),
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
    let pin = session.appearance_pin();
    let frame = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(40, 15), &pin)
        .unwrap();
    let text = visible_rows(&frame.surface);
    assert!(text.contains("No steps provided."), "{text}");
    assert!(!text.contains("0/0"), "{text}");
    assert!(!text.contains("Thinking"), "{text}");
    assert!(!text.contains(ActivityPlan::SCHEMA), "{text}");
}

// 제안 계획은 Markdown 본문을 교체해 폭별로 다시 배치하고 중단 후에도 제목·원문·footer를 유지한다.
#[test]
fn proposed_plan_reflows_final_document_without_draft_or_json() {
    use std::time::Duration;

    use yo_core::{ActivityDocument, ActivityOutcome};

    use super::key;
    use crate::{
        OutputPreferences,
        input::event::{KeyCode, KeyModifiers},
    };
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_output_preferences(OutputPreferences::default().with_reasoning(false));
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ModelWork,
        })
        .unwrap();
    let final_source = "## Final approach\n\n- Keep the source\n\n```rust\nuse std::fmt;\n```\n\n| Check | Value |\n| --- | --- |\n| width | reflow |";
    for source in ["## Draft only", final_source] {
        session
            .parts_mut()
            .state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(
                    ActivityDocument {
                        title: "Proposed plan".to_owned(),
                        markdown: source.to_owned(),
                    }
                    .to_snapshot()
                    .unwrap(),
                ),
            })
            .unwrap();
    }
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextDelta(String::new()),
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityFinished {
            activity: activity(1),
            outcome: ActivityOutcome::Interrupted,
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .handle(
            key(KeyCode::Character('o'), KeyModifiers::CONTROL),
            Duration::ZERO,
        )
        .unwrap();
    let original = session.session_output().unwrap().unwrap();
    for line in final_source.lines() {
        assert!(original.contains(line), "{original}");
    }
    assert!(!original.contains("Draft only"));
    assert!(!original.contains(ActivityDocument::SCHEMA));
    let pin = session.appearance_pin();
    for width in [60, 24, 60] {
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 55), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(text.contains("Proposed plan"), "{text}");
        assert!(text.contains("Final approach"), "{text}");
        assert!(text.contains("use std::fmt;"), "{text}");
        assert!(text.contains("reflow"), "{text}");
        assert!(text.contains("Interrupted"), "{text}");
        assert!(!text.contains("Draft only"), "{text}");
        assert!(!text.contains("Summary hidden"), "{text}");
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
}

// 보고된 턴 시간은 좁은 폭에서 다시 줄바꿈해도 값·출처·원문 내보내기를 유지한다.
#[test]
fn reported_turn_duration_reflows_without_losing_precision() {
    use yo_core::{ActivityNotice, ActivityOutcome, NoticeLevel};

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
                ActivityNotice {
                    title: "Turn completed".to_owned(),
                    message: "Duration: 1m 2.345s (62345 ms, reported by Codex)".to_owned(),
                    level: NoticeLevel::Info,
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
    let source = session.session_output().unwrap().unwrap();
    assert!(source.contains("62345 ms, reported by Codex"));
    let pin = session.appearance_pin();
    for width in [60, 24, 60] {
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 35), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        let joined = text
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        assert!(
            joined.contains("Duration:1m2.345s(62345ms,reportedbyCodex)"),
            "{text}"
        );
        assert!(text.contains("Turn completed"), "{text}");
        assert!(!text.contains(ActivityNotice::SCHEMA), "{text}");
        assert_eq!(session.session_output().unwrap().unwrap(), source);
    }
}

// 파일 읽기·리소스의 Rust 원문은 사용자 지정 코드 색상을 쓰며 오류·비파일 출력은 일반 텍스트로
// 공통 diff 콘텐츠는 프로바이더 이름 없이 추가·삭제 테마를 쓰고 좁은 화면에서도 원문을 보존한다.
#[test]
fn generic_diff_content_uses_theme_and_preserves_plain_source() {
    use serde_json::json;
    use yo_core::ToolOutput;

    use crate::{OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole};
    let patch = "--- old\n+++ new\n@@ -1 +1 @@\n-old value\n+new value\n";
    let output = ToolOutput {
        tool: "workspace_patch".into(),
        server: None,
        arguments: None,
        result: None,
        content_items: Some(
            json!([{"type":"diff","title":"File change · file.rs","text":patch,"source":{"oldText":"old value\n","newText":"new value\n"}}]),
        ),
        error: None,
        plain_text: patch.into(),
    };
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_output_preferences(OutputPreferences::default().with_tool_head_rows(u16::MAX))
        .with_theme_overrides(
            ThemeOverrides::default()
                .with_color(ThemeRole::DiffAdded, ThemeColor::Rgb(20, 214, 120))
                .with_color(ThemeRole::DiffRemoved, ThemeColor::Rgb(215, 21, 120)),
        );
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
            update: ActivityUpdate::TextSnapshot(output.to_snapshot().unwrap()),
        })
        .unwrap();
    let pin = session.appearance_pin();
    for width in [80, 24, 80] {
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 40), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        for (needle, color) in [
            (
                "-old value",
                Color::Rgb {
                    red: 215,
                    green: 21,
                    blue: 120,
                },
            ),
            (
                "+new value",
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
                .find(|(_, row)| row.contains(needle))
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
        assert!(!text.contains("oldText"), "{text}");
        session.parts_mut().state.commit_frame(&frame);
    }
    assert!(
        session
            .session_output()
            .unwrap()
            .unwrap()
            .contains("-old value")
    );
}

// 공개 추론 색상은 숨김 안내에도 적용하지만 압축 요약·원문과 다른 테마 역할을 바꾸지 않는다.
#[test]
fn reasoning_color_is_independent_and_survives_resize_and_visibility() {
    use yo_core::{ActivitySummary, SummaryKind};

    use crate::{OutputPreferences, ThemeColor, ThemeOverrides, ThemeRole};
    for visible in [true, false] {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(
                OutputPreferences::default()
                    .with_reasoning(visible)
                    .with_tool_head_rows(u16::MAX),
            )
            .with_theme_overrides(
                ThemeOverrides::default()
                    .with_color(ThemeRole::Muted, ThemeColor::Rgb(60, 70, 80))
                    .with_color(ThemeRole::ReasoningText, ThemeColor::Rgb(152, 118, 84)),
            );
        for (id, kind, summary) in [
            (1, SummaryKind::Reasoning, "Reasoning words"),
            (2, SummaryKind::Compaction, "Compaction words"),
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
                    update: ActivityUpdate::TextSnapshot(
                        ActivitySummary {
                            kind,
                            summary: summary.into(),
                            tokens_before: None,
                        }
                        .to_snapshot()
                        .unwrap(),
                    ),
                })
                .unwrap();
        }
        let source = session.session_output().unwrap().unwrap();
        assert!(source.contains("Reasoning words"));
        let pin = session.appearance_pin();
        for width in [80, 24, 80] {
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 40), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            for (needle, color) in [
                (
                    if visible {
                        "Reasoning words"
                    } else {
                        "Summary hidden"
                    },
                    Color::Rgb {
                        red: 152,
                        green: 118,
                        blue: 84,
                    },
                ),
                (
                    "Compaction words",
                    Color::Rgb {
                        red: 60,
                        green: 70,
                        blue: 80,
                    },
                ),
            ] {
                let (y, row) = text
                    .lines()
                    .enumerate()
                    .find(|(_, row)| row.contains(needle))
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
            session.parts_mut().state.commit_frame(&frame);
        }
        assert_eq!(session.session_output().unwrap().unwrap(), source);
    }
}

// 완료된 사용량만 짧게 접고 여러 프로바이더의 상세·개별 관측·원문은 펼침과 폭 변경 뒤에도 보존한다.
#[test]
fn completed_usage_compacts_without_losing_observations_or_source() {
    use serde_json::json;
    use yo_core::ActivityOutcome;

    use super::key;
    use crate::{
        OutputPreferences,
        input::event::{KeyCode, KeyModifiers},
    };
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_output_preferences(OutputPreferences::default());
    for (id, schema, profile, input) in [
        (
            1,
            "codex.app-server-token-usage-receipt/v1",
            "codex.app-server.thread-token-usage-updated/v1",
            120,
        ),
        (
            2,
            "grok.acp-prompt-usage-receipt/v1",
            "grok.acp.prompt-response.usage/v1",
            240,
        ),
    ] {
        let receipt=json!({"schema":schema,"source_profile":profile,"turn_id":"turn-a","prompt_request_id":42,
            "usage":{"input_tokens":input,"output_tokens":30,"total_tokens":input+30,"reasoning_tokens":12,"cache_read_input_tokens":80,"cache_write_input_tokens":0}}).to_string();
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
                update: ActivityUpdate::TextSnapshot(receipt),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(id),
                outcome: ActivityOutcome::Completed,
            })
            .unwrap();
    }
    let original = session.session_output().unwrap().unwrap();
    assert_eq!(original.matches("Cache read: 80").count(), 2);
    for (toggle, width, expanded) in [
        (false, 80, false),
        (false, 24, false),
        (true, 24, true),
        (false, 80, true),
        (true, 80, false),
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
        let joined = text.split_whitespace().collect::<String>();
        assert!(joined.contains("Input:120"));
        assert!(joined.contains("Input:240"));
        assert_eq!(joined.contains("Cacheread:80"), expanded);
        assert_eq!(
            text.matches("Usage ·").count(),
            if expanded { 0 } else { 2 }
        );
        session.parts_mut().state.commit_frame(&frame);
        assert_eq!(session.session_output().unwrap().unwrap(), original);
    }
}

// 호스트 상태는 키 순서대로 별도 행에 표시하고 좁은 폭에서는 grapheme을 자르지 않고 생략한다.
// 갱신·삭제·테마 변경은 대화 원문을 바꾸지 않고 낮은 화면의 입력과 대화 공간을 보존한다.
#[test]
fn host_status_line_updates_without_changing_conversation_source() {
    use crate::{
        ThemeColor, ThemeOverrides, ThemeRole, TuiStatusLine,
        runner::{AgentPoll, unix::apply_agent_poll},
    };
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
            update: ActivityUpdate::TextSnapshot("Preserved conversation".into()),
        })
        .unwrap();
    let source = session.session_output().unwrap();
    assert!(source.as_ref().unwrap().contains("Preserved conversation"));
    let status = TuiStatusLine::new([
        ("z-worker", "Worker: ready"),
        ("a-checks", "Checks: 한글 passed"),
    ])
    .unwrap();
    assert!(
        apply_agent_poll(
            session.parts_mut().state,
            AgentPoll::StatusLine(status.clone())
        )
        .unwrap()
    );
    assert!(!apply_agent_poll(session.parts_mut().state, AgentPoll::StatusLine(status)).unwrap());
    for width in [80, 24, 3, 80] {
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(width, 30), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        if width == 80 {
            assert!(
                text.split_whitespace()
                    .collect::<String>()
                    .contains("Checks:한글passed·Worker:ready")
            );
        } else if width == 24 {
            assert!(
                text.split_whitespace()
                    .collect::<String>()
                    .contains("Checks:한글passed")
            );
            assert!(text.contains("..."));
        } else {
            assert!(text.contains('.'));
        }
        session.parts_mut().state.commit_frame(&frame);
        assert_eq!(session.session_output().unwrap(), source);
    }
    session = session.with_theme_overrides(
        ThemeOverrides::default().with_color(ThemeRole::Muted, ThemeColor::Rgb(12, 34, 56)),
    );
    assert!(session.set_status_line(TuiStatusLine::new([("checks", "Checks: updated")]).unwrap()));
    let pin = session.appearance_pin();
    let updated = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(80, 30), &pin)
        .unwrap();
    assert!(visible_rows(&updated.surface).contains("Checks: updated"));
    assert_eq!(
        updated
            .surface
            .cell(Point::new(0, 28))
            .unwrap()
            .style()
            .foreground,
        Color::Rgb {
            red: 12,
            green: 34,
            blue: 56
        }
    );
    assert!(!visible_rows(&updated.surface).contains("Worker: ready"));
    session.parts_mut().state.commit_frame(&updated);
    assert!(session.set_status_line(TuiStatusLine::default()));
    let empty = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(80, 30), &pin)
        .unwrap();
    assert!(!visible_rows(&empty.surface).contains("Checks:"));
    assert_eq!(session.session_output().unwrap(), source);
}

// 답변 콜백은 본문·폭·완료 상태만 받고 실패 원문·다른 역할·내보내기를 바꾸지 않는다.
#[test]
fn assistant_renderer_preserves_outcomes_roles_source_and_reflow() {
    use std::sync::{Arc, Mutex};

    use yo_core::{ActivityOutcome, Failure};

    use crate::{AssistantRenderer, Theme};
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&seen);
    let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
        .with_assistant_renderer(Some(AssistantRenderer::new(move |input| {
            captured.lock().unwrap().push((
                input.source.to_owned(),
                input.columns.get(),
                input.finalized,
            ));
            Some(format!(
                "## Adapted answer\n\n```rust\nlet ready = true;\n```\n\nWidth {}",
                input.columns
            ))
        })));
    let state = session.parts_mut().state;
    state
        .observe_record(TranscriptRecord::CommandCommitted(
            AgentCommand::StartTurn {
                turn: turn(),
                input: UserInput::from("USER SOURCE"),
            },
        ))
        .unwrap();
    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::AgentMessage,
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot("Original **answer**".to_owned()),
        })
        .unwrap();
    let pin = session.appearance_pin();
    let frame = session
        .parts_mut()
        .state
        .prepare_frame(Size::new(80, 40), &pin)
        .unwrap();
    assert!(visible_rows(&frame.surface).contains("Adapted answer"));
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|(_, _, finalized)| !finalized)
    );
    session.parts_mut().state.commit_frame(&frame);
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityFinished {
            activity: activity(1),
            outcome: ActivityOutcome::Failed(Failure::new("ACTUAL **failure**")),
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
            update: ActivityUpdate::TextSnapshot("TOOL SOURCE".to_owned()),
        })
        .unwrap();
    session
        .parts_mut()
        .state
        .observe(AgentEvent::ActivityFinished {
            activity: activity(2),
            outcome: ActivityOutcome::Completed,
        })
        .unwrap();
    let original = session.session_output().unwrap();
    for theme in [Theme::Default, Theme::Light, Theme::Mono] {
        session = session.with_theme(theme);
        for width in [80, 24, 80] {
            let pin = session.appearance_pin();
            let frame = session
                .parts_mut()
                .state
                .prepare_frame(Size::new(width, 40), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            for expected in [
                "Adapted answer",
                "USER SOURCE",
                "TOOL SOURCE",
                "ACTUAL **failure**",
            ] {
                assert!(
                    text.split_whitespace()
                        .collect::<String>()
                        .contains(&expected.split_whitespace().collect::<String>()),
                    "{text}"
                );
            }
            assert!(text.contains(&format!("Width {}", width - 2)));
            session.parts_mut().state.commit_frame(&frame);
            assert_eq!(session.session_output().unwrap(), original);
        }
    }
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .all(|(source, _, _)| source == "Original **answer**")
    );
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|(_, _, finalized)| *finalized)
    );
    for (renderer, expected) in [
        (
            Some(AssistantRenderer::new(|_| {
                Some("Second presentation".into())
            })),
            "Second presentation",
        ),
        (
            Some(AssistantRenderer::new(|_| Some("\u{301}".into()))),
            "Original answer",
        ),
        (
            Some(AssistantRenderer::new(|_| {
                Some("x".repeat(AssistantRenderer::MAX_RENDERED_BYTES + 1))
            })),
            "Original answer",
        ),
        (Some(AssistantRenderer::new(|_| None)), "Original answer"),
        (None, "Original answer"),
        (
            Some(AssistantRenderer::new(|_| Some(String::new()))),
            "ACTUAL **failure**",
        ),
    ] {
        session = session.with_assistant_renderer(renderer);
        let pin = session.appearance_pin();
        let frame = session
            .parts_mut()
            .state
            .prepare_frame(Size::new(80, 40), &pin)
            .unwrap();
        let text = visible_rows(&frame.surface);
        assert!(
            text.split_whitespace()
                .collect::<String>()
                .contains(&expected.split_whitespace().collect::<String>()),
            "{text}"
        );
        assert!(text.contains("ACTUAL **failure**"), "{text}");
        session.parts_mut().state.commit_frame(&frame);
        assert_eq!(session.session_output().unwrap(), original);
    }
}

// 공개 요약과 공급자 추론 모두 긴 본문을 접고 좁은 폭에서 펼쳐도 원문·끝부분·실패를 보존한다.
#[test]
fn reasoning_folding_preserves_tail_failure_and_source_after_resize() {
    use yo_core::{ActivityOutcome, ActivityReasoning, ActivitySummary, Failure, SummaryKind};

    use super::key;
    use crate::{
        OutputPreferences,
        input::event::{KeyCode, KeyModifiers},
    };
    let body = (0..20)
        .map(|line| format!("Reasoning line {line:02}\n\n"))
        .collect::<String>();
    let sources = [
        ActivitySummary {
            kind: SummaryKind::Reasoning,
            summary: body.clone(),
            tokens_before: None,
        }
        .to_snapshot()
        .unwrap(),
        ActivityReasoning {
            content: serde_json::json!(body),
        }
        .to_snapshot()
        .unwrap(),
    ];
    for source in sources {
        let mut session = TuiSession::new(ColorCapability::TrueColor, MotionPreference::Reduced)
            .with_output_preferences(OutputPreferences::default().with_tool_head_rows(2));
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
                update: ActivityUpdate::TextSnapshot(source),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: ActivityOutcome::Failed(Failure::new("Retained failure")),
            })
            .unwrap();
        let original = session.session_output().unwrap().unwrap();
        for (toggle, width, expanded) in [(false, 80, false), (true, 24, true), (true, 80, false)] {
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
                .prepare_frame(Size::new(width, 60), &pin)
                .unwrap();
            let text = visible_rows(&frame.surface);
            let flat = text.split_whitespace().collect::<String>();
            assert_eq!(flat.contains("rowshidden"), !expanded, "{text}");
            assert_eq!(flat.contains("Reasoningline08"), expanded, "{text}");
            assert!(flat.contains("Reasoningline19"), "{text}");
            assert!(flat.contains("Retainedfailure"), "{text}");
            session.parts_mut().state.commit_frame(&frame);
            assert_eq!(session.session_output().unwrap().unwrap(), original);
        }
    }
}
