use std::time::Duration;

use yo_core::{ActivityKind, ActivityOutcome, ActivityUpdate, AgentEvent, Failure, TurnOutcome};

use super::{activity, key, rendered_row, turn};
use crate::{
    ColorCapability, GlyphProfile, MotionPreference,
    appearance::{AppearanceCandidate, AppearanceState},
    input::event::{KeyCode, KeyModifiers},
    runner::state::{StateEffect, TuiState},
    surface::{Attributes, Point, Size},
    transcript::TranscriptBody,
};

// Chat은 아홉 번째 로그 행부터 접고 실제 Ctrl+O 경로로 펼치며 실패 footer와 원문은 보존한다.
#[test]
fn long_tool_logs_expand_without_changing_output_or_failure() {
    for count in [8, 9, 30] {
        let mut state = TuiState::new();
        let pin = AppearanceState::default().pin();
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolCall,
            })
            .unwrap();
        let log = (1..=count)
            .map(|n| format!("LOG {n:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(log.clone()),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: ActivityOutcome::Failed(Failure::new("Check the fixture path")),
            })
            .unwrap();
        let original = state.session_output(&pin).unwrap().unwrap();
        let size = Size::new(88, 48);
        let screen = |state: &TuiState| {
            (0..size.height)
                .map(|y| rendered_row(state, size, y))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let compact = screen(&state);
        assert_eq!(compact.contains("Ctrl+O expand"), count > 8);
        assert!(compact.contains("Check the fixture path"));
        assert!(compact.contains(&format!("LOG {count:02}")));
        if count > 8 {
            assert!(!compact.contains("LOG 03"));
        }
        assert_eq!(
            state
                .handle(
                    key(KeyCode::Character('o'), KeyModifiers::CONTROL),
                    Duration::ZERO
                )
                .unwrap(),
            StateEffect::Redraw
        );
        let expanded = screen(&state);
        assert!(expanded.contains("LOG 03"));
        assert!(!expanded.contains("rows hidden"));
        assert_eq!(state.session_output(&pin).unwrap().unwrap(), original);
        for line in log.lines() {
            assert!(original.contains(line));
        }
    }
}

// 완료·실패·중단 후 Working 행을 공백으로 다시 그리며, 이전 애니메이션 demand도 제거한다.
#[test]
fn every_turn_terminal_outcome_clears_working_and_disarms_motion() {
    for outcome in [
        TurnOutcome::Completed,
        TurnOutcome::Interrupted,
        TurnOutcome::Failed(Failure::new("test failure")),
    ] {
        let mut state = TuiState::new();
        let size = Size::new(88, 14);
        let pin = AppearanceState::default().pin();
        state
            .observe(AgentEvent::TurnStarted { turn: turn() })
            .unwrap();
        let busy = state.prepare_frame(size, &pin).unwrap();
        assert!(busy.motion_demand.is_some());
        let working_row = (0..size.height)
            .find(|&y| rendered_row(&state, size, y).contains("Working"))
            .unwrap();
        assert_eq!(
            state
                .observe(AgentEvent::TurnFinished {
                    turn: turn(),
                    outcome
                })
                .unwrap(),
            StateEffect::Redraw
        );
        let idle = state.prepare_frame(size, &pin).unwrap();
        assert!(idle.motion_demand.is_none());
        assert_eq!(rendered_row(&state, size, working_row), "");
        assert!(!state.turn_active());
    }
}

// 도구 완료 표시는 진행형 label만 교체하고 실제 payload(동일 문구 포함)는 그대로 보존한다.
#[test]
fn finished_tool_replaces_only_its_progress_heading() {
    for outcome in [
        ActivityOutcome::Completed,
        ActivityOutcome::Interrupted,
        ActivityOutcome::Failed(Failure::new("test failure")),
    ] {
        let mut state = TuiState::new();
        let tool = activity(1);
        state
            .observe(AgentEvent::ActivityStarted {
                activity: tool,
                kind: ActivityKind::ToolCall,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: tool,
                update: ActivityUpdate::TextSnapshot("payload says Running tool…".to_owned()),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: tool,
                outcome,
            })
            .unwrap();
        let TranscriptBody::Message(message) = state.transcript().items()[0].body();
        let text = message.text();
        assert!(!text.starts_with("Running tool…"));
        assert!(text.contains("\npayload says Running tool…"));
    }
}

// agent message의 streaming delta를 먼저 표시하더라도 final snapshot이 다르면 화면 문자열을
// authoritative 결과로 교체하고 완료 뒤 그대로 남긴다.
#[test]
fn renders_the_authoritative_agent_message_snapshot() {
    let mut state = TuiState::new();
    let message = activity(1);
    state
        .observe(AgentEvent::ActivityStarted {
            activity: message,
            kind: ActivityKind::AgentMessage,
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: message,
            update: ActivityUpdate::TextDelta("partial".to_owned()),
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: message,
            update: ActivityUpdate::TextSnapshot("complete answer".to_owned()),
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityFinished {
            activity: message,
            outcome: ActivityOutcome::Completed,
        })
        .unwrap();

    assert_eq!(
        rendered_row(&state, Size::new(24, 3), 0),
        "• complete answer"
    );
}

// non-message Activity의 빈 delta는 label 뒤에 보이지 않는 줄 바꿈을 누적하지 않고
// transcript와 화면 revision을 그대로 유지한다.
#[test]
fn empty_activity_delta_does_not_add_placeholder_lines() {
    let mut state = TuiState::new();
    let tool = activity(1);
    state
        .observe(AgentEvent::ActivityStarted {
            activity: tool,
            kind: ActivityKind::ToolCall,
        })
        .unwrap();
    let before = state.transcript().clone();

    assert_eq!(
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: tool,
                update: ActivityUpdate::TextDelta(String::new()),
            })
            .unwrap(),
        StateEffect::Unchanged
    );

    assert_eq!(state.transcript(), &before);
}

// tool과 file-change Activity는 agent message가 없어도 서로 다른 완료 관찰로 transcript에
// 계속 남아 코딩 작업이 chat text만으로 축소되지 않는다.
#[test]
fn retains_completed_tool_and_file_change_observations() {
    let mut state = TuiState::new();
    let tool = activity(1);
    let file = activity(2);
    for (activity, kind) in [
        (tool, ActivityKind::ToolCall),
        (file, ActivityKind::FileChange),
    ] {
        state
            .observe(AgentEvent::ActivityStarted { activity, kind })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            })
            .unwrap();
    }

    assert_eq!(
        rendered_row(&state, Size::new(30, 12), 0),
        "• Tool completed"
    );
    assert_eq!(
        rendered_row(&state, Size::new(30, 12), 2),
        "• Changes completed"
    );
}

// Turn의 실패 event는 활성 상태를 닫고 사용자에게 backend 오류 내용을 별도 transcript
// 항목으로 남긴다.
#[test]
fn renders_turn_failure_and_clears_active_state() {
    let mut state = TuiState::new();
    state
        .observe(AgentEvent::TurnStarted { turn: turn() })
        .unwrap();

    state
        .observe(AgentEvent::TurnFinished {
            turn: turn(),
            outcome: TurnOutcome::Failed(Failure::new("provider stopped")),
        })
        .unwrap();

    assert!(!state.turn_active());
    assert_eq!(
        rendered_row(&state, Size::new(36, 3), 0),
        "• Turn failed: provider stopped"
    );
}

// 같은 Markdown payload라도 모델 답변만 서식을 적용하고 도구 로그는 문자 그대로 표시한다.
#[test]
fn activity_kind_selects_markdown_without_rewriting_tool_payload() {
    for (kind, formatted) in [
        (ActivityKind::AgentMessage, true),
        (ActivityKind::ToolCall, false),
    ] {
        let mut state = TuiState::new();
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot("**literal** and `code`".to_owned()),
            })
            .unwrap();
        let size = Size::new(60, 12);
        let visible = (0..12)
            .map(|y| rendered_row(&state, size, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(visible.contains("**literal**"), !formatted);
        assert!(visible.contains(if formatted {
            "literal and code"
        } else {
            "**literal** and `code`"
        }));
    }
}

// 로그 내부의 실패 문구는 본문 스타일로 남고, 실제 outcome만 제목·footer 색을 바꾼다.
#[test]
fn tool_outcome_styles_are_typed_and_payload_is_literal() {
    let appearance = AppearanceState::new(AppearanceCandidate::for_profile_with_host_preferences(
        GlyphProfile::Rich,
        ColorCapability::TrueColor,
        MotionPreference::Reduced,
    ))
    .unwrap();
    let pin = appearance.pin();
    let styles = pin.snapshot().styles().transcript.activity;
    for (outcome, expected_color) in [
        (ActivityOutcome::Completed, styles.success.foreground),
        (ActivityOutcome::Interrupted, styles.warning.foreground),
        (
            ActivityOutcome::Failed(Failure::new("actual failure")),
            styles.error.foreground,
        ),
    ] {
        let mut state = TuiState::new();
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ToolCall,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot("Failed: from log\n**literal**".to_owned()),
            })
            .unwrap();
        let running = state.prepare_frame(Size::new(60, 12), &pin).unwrap();
        assert_eq!(
            running
                .surface
                .cell(Point::new(2, 0))
                .unwrap()
                .style()
                .foreground,
            styles.heading.foreground
        );
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: outcome.clone(),
            })
            .unwrap();
        let frame = state.prepare_frame(Size::new(60, 12), &pin).unwrap();
        let heading = frame.surface.cell(Point::new(2, 0)).unwrap().style();
        assert_eq!(heading.foreground, expected_color);
        assert!(heading.attributes.contains(Attributes::BOLD));
        assert_eq!(
            frame.surface.cell(Point::new(2, 1)).unwrap().style(),
            styles.body
        );
        assert!(rendered_row(&state, Size::new(60, 12), 2).contains("**literal**"));
        let TranscriptBody::Message(message) = state.transcript().items()[0].body();
        assert!(message.text().contains("Failed: from log\n**literal**"));
        match outcome {
            ActivityOutcome::Failed(_) => {
                assert_eq!(
                    frame.surface.cell(Point::new(2, 3)).unwrap().style(),
                    styles.error
                );
                assert!(message.text().ends_with("Failed: actual failure"));
            },
            ActivityOutcome::Interrupted => {
                assert_eq!(
                    message
                        .text()
                        .to_ascii_lowercase()
                        .matches("interrupted")
                        .count(),
                    1
                )
            },
            ActivityOutcome::Completed => {},
        }
    }
}

// 실제 FileChange Activity도 diff 배경을 사용하고 좁은 폭에서 줄바꿈·펼치기 후 원문을 보존한다.
#[test]
fn file_change_activity_uses_diff_panels_and_retains_literal_source() {
    for width in [20, 40, 88] {
        let mut state = TuiState::new();
        let appearance =
            AppearanceState::new(AppearanceCandidate::for_profile_with_host_preferences(
                GlyphProfile::Rich,
                ColorCapability::TrueColor,
                MotionPreference::Reduced,
            ))
            .unwrap();
        let pin = appearance.pin();
        let source = "update: src/settings.rs\n@@ -1,3 +1,4 @@\n-old_value\n+new_value_with_a_long_identifier\n+```rust\n+![literal](data:image/png;base64,AAAA)\n context";
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::FileChange,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(source.to_owned()),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: ActivityOutcome::Completed,
            })
            .unwrap();
        let original = state.session_output(&pin).unwrap().unwrap();
        let TranscriptBody::Message(message) = state.transcript().items()[0].body();
        assert!(message.text().contains(source));
        state
            .handle(
                key(KeyCode::Character('o'), KeyModifiers::CONTROL),
                Duration::ZERO,
            )
            .unwrap();
        let size = Size::new(width, 60);
        let frame = state.prepare_frame(size, &pin).unwrap();
        let screen = (0..size.height)
            .map(|y| rendered_row(&state, size, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains("+3 -1"));
        assert!(screen.contains("+```rust"));
        assert!(frame.surface.rasters.is_empty());
        let removed_row = (0..size.height)
            .find(|&y| rendered_row(&state, size, y).contains("-old_value"))
            .unwrap();
        let added_row = (0..size.height)
            .find(|&y| rendered_row(&state, size, y).contains("+new_value"))
            .unwrap();
        let removed = frame
            .surface
            .cell(Point::new(width - 1, removed_row))
            .unwrap()
            .style()
            .background;
        let added = frame
            .surface
            .cell(Point::new(width - 1, added_row))
            .unwrap()
            .style()
            .background;
        assert_ne!(removed, added);
        assert_eq!(state.session_output(&pin).unwrap().unwrap(), original);
    }
}

// 접힌 diff의 배경 행도 본문과 함께 이동하고 실제 실패 이유는 접힘 바깥에 남는다.
#[test]
fn collapsed_file_change_keeps_tail_colors_and_failure_footer() {
    let mut state = TuiState::new();
    let appearance = AppearanceState::new(AppearanceCandidate::for_profile_with_host_preferences(
        GlyphProfile::Rich,
        ColorCapability::TrueColor,
        MotionPreference::Reduced,
    ))
    .unwrap();
    let pin = appearance.pin();
    let patch = format!(
        "update: src/settings.rs\n@@ -0,0 +1,20 @@\n{}",
        (1..=20)
            .map(|n| format!("+addition_{n:02}\n"))
            .collect::<String>()
    );
    state
        .observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::FileChange,
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(patch),
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityFinished {
            activity: activity(1),
            outcome: ActivityOutcome::Failed(Failure::new("Edit was not applied")),
        })
        .unwrap();
    let size = Size::new(60, 32);
    let screen = (0..size.height)
        .map(|y| rendered_row(&state, size, y))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(screen.contains("rows hidden"));
    assert!(screen.contains("addition_20"));
    assert!(!screen.contains("addition_10"));
    assert!(screen.contains("Edit was not applied"));
    let frame = state.prepare_frame(size, &pin).unwrap();
    let tail = (0..size.height)
        .find(|&y| rendered_row(&state, size, y).contains("addition_20"))
        .unwrap();
    let hint = (0..size.height)
        .find(|&y| rendered_row(&state, size, y).contains("rows hidden"))
        .unwrap();
    assert_ne!(
        frame
            .surface
            .cell(Point::new(59, tail))
            .unwrap()
            .style()
            .background,
        frame
            .surface
            .cell(Point::new(59, hint))
            .unwrap()
            .style()
            .background
    );
}

// 완료된 사용량만 footer에 남기고 실행 중·중단된 관측은 최신 확정값을 바꾸지 않는다.
#[test]
fn usage_receipts_render_readably_and_commit_metrics_only_on_completion() {
    let receipt = |input| {
        format!(
            r#"{{"schema":"codex.app-server-token-usage-receipt/v1","source_profile":"codex.app-server.thread-token-usage-updated/v1","turn_id":"turn-a","model_context_window":200000,"usage":{{"input_tokens":{input},"output_tokens":30,"total_tokens":{},"reasoning_tokens":12,"cache_read_input_tokens":80,"cache_write_input_tokens":0}}}}"#,
            input + 30
        )
    };
    let mut state = TuiState::new();
    let size = Size::new(100, 32);
    let screen = |state: &TuiState| {
        (0..size.height)
            .map(|y| rendered_row(state, size, y))
            .collect::<Vec<_>>()
            .join("\n")
    };
    for (id, input, outcome) in [
        (1, 120, ActivityOutcome::Completed),
        (2, 240, ActivityOutcome::Interrupted),
        (3, 360, ActivityOutcome::Completed),
    ] {
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(id),
                kind: ActivityKind::ModelWork,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(id),
                update: ActivityUpdate::TextSnapshot(receipt(input)),
            })
            .unwrap();
        let pending = screen(&state);
        assert!(pending.contains(&format!("Input: {input} · Output: 30")));
        assert!(!pending.contains("codex.app-server-token-usage"));
        assert!(!pending.contains(&format!("Last: {input} in")));
        if id == 3 {
            state
                .observe(AgentEvent::ActivityUpdated {
                    activity: activity(id),
                    update: ActivityUpdate::TextDelta(String::new()),
                })
                .unwrap();
        }
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(id),
                outcome,
            })
            .unwrap();
        assert!(screen(&state).contains("Last: 120 in / 30 out"));
    }
    assert!(!screen(&state).contains("Last: 240 in"));
    assert!(screen(&state).contains("Cache read: 80 · Cache write: 0"));
    assert!(screen(&state).contains("Context window: 200000 tokens"));
    // 좁은 footer는 backend/입력 안내를 밀어내지 않고 상세 값은 대화에 유지한다.
    for width in [20, 40] {
        state
            .prepare_frame(Size::new(width, 24), &AppearanceState::default().pin())
            .unwrap();
    }
}

// 알려진 schema의 잘못된 수치를 정상 사용량이나 0으로 표시하지 않는다.
#[test]
fn malformed_usage_does_not_publish_metrics_or_dump_receipt_json() {
    let mut state = TuiState::new();
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
                "{\"schema\":\"codex.app-server-token-usage-receipt/v1\",\"usage\":{}}".to_owned(),
            ),
        })
        .unwrap();
    state
        .observe(AgentEvent::ActivityFinished {
            activity: activity(1),
            outcome: ActivityOutcome::Completed,
        })
        .unwrap();
    let size = Size::new(100, 24);
    let screen = (0..size.height)
        .map(|y| rendered_row(&state, size, y))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(screen.contains("Usage unavailable"));
    assert!(!screen.contains("Last:"));
    assert!(!screen.contains("codex.app-server-token-usage"));
}

// 완료된 공개 요약은 진행형 Thinking 제목을 남기지 않으며 제목 변경이 즉시 redraw를 요청한다.
// payload의 동일 문구는 유지하고 plan snapshot 같은 별도 제목은 건드리지 않는다.
#[test]
fn model_work_terminal_heading_redraws_without_rewriting_payload() {
    for (outcome, heading) in [
        (ActivityOutcome::Completed, "Model work completed"),
        (ActivityOutcome::Interrupted, "Model work interrupted"),
        (
            ActivityOutcome::Failed(Failure::new("reason")),
            "Model work failed",
        ),
    ] {
        let mut state = TuiState::new();
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: ActivityKind::ModelWork,
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot("payload mentions Thinking…".into()),
            })
            .unwrap();
        assert_eq!(
            state
                .observe(AgentEvent::ActivityFinished {
                    activity: activity(1),
                    outcome
                })
                .unwrap(),
            StateEffect::Redraw
        );
        let TranscriptBody::Message(message) = state.transcript().items()[0].body();
        assert!(message.text().starts_with(heading), "{}", message.text());
        assert!(message.text().contains("payload mentions Thinking…"));
    }
}

// 질문·승인 설명은 prompt에 없어도 원래 번호·비활성 상태와 함께 대화 및 내보내기에 남긴다.
#[test]
fn request_choice_descriptions_remain_readable_after_completion() {
    use yo_core::{ActivityApproval, ActivityQuestion, ApprovalChoice, QuestionChoice, RequestId};
    for approval in [false, true] {
        let mut state = TuiState::new();
        let request_id = RequestId::new(super::nonzero(83));
        let description = "첫 번째 설명\n끝까지 읽을 수 있는 설명";
        let snapshot = if approval {
            ActivityApproval {
                related_change: None,
                plain_text: "Original request".into(),
                choices: vec![
                    ApprovalChoice {
                        label: "Allow".into(),
                        description: description.into(),
                        enabled: false,
                    },
                    ApprovalChoice {
                        label: "Decline".into(),
                        description: "Do not execute".into(),
                        enabled: true,
                    },
                ],
                decline_choice: Some(2),
            }
            .to_snapshot()
            .unwrap()
        } else {
            ActivityQuestion {
                plain_text: "Original request".into(),
                allow_notes: false,
                previous_question: false,
                draft: None,
                draft_choice: None,
                choices: vec![
                    QuestionChoice {
                        label: "Allow".into(),
                        description: description.into(),
                    },
                    QuestionChoice {
                        label: "Decline".into(),
                        description: "Do not execute".into(),
                    },
                ],
            }
            .to_snapshot()
            .unwrap()
        };
        state
            .observe(AgentEvent::ActivityStarted {
                activity: activity(1),
                kind: if approval {
                    ActivityKind::ApprovalRequest { request_id }
                } else {
                    ActivityKind::UserInputRequest { request_id }
                },
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityUpdated {
                activity: activity(1),
                update: ActivityUpdate::TextSnapshot(snapshot.clone()),
            })
            .unwrap();
        state
            .observe(AgentEvent::ActivityFinished {
                activity: activity(1),
                outcome: ActivityOutcome::Completed,
            })
            .unwrap();
        let pin = AppearanceState::default().pin();
        let output = state.session_output(&pin).unwrap().unwrap();
        let readable = output.lines().map(str::trim).collect::<Vec<_>>().join("\n");
        assert!(
            readable.contains("Original request\n\nChoices:\n1. Allow"),
            "{output}"
        );
        assert!(
            readable.contains("첫 번째 설명\n끝까지 읽을 수 있는 설명"),
            "{output}"
        );
        assert!(readable.contains("2. Decline\nDo not execute"), "{output}");
        assert_eq!(output.contains("(unavailable)"), approval);
        assert!(!output.contains("schema"));
        for width in [80, 24, 80] {
            let frame = state.prepare_frame(Size::new(width, 40), &pin).unwrap();
            state.commit_frame(&frame);
            let text = (0..40)
                .map(|y| rendered_row(&state, Size::new(width, 40), y))
                .collect::<String>();
            assert!(text.contains("Do not execute"), "{text}");
        }
        let mut tool = TuiState::new();
        tool.observe(AgentEvent::ActivityStarted {
            activity: activity(1),
            kind: ActivityKind::ToolCall,
        })
        .unwrap();
        tool.observe(AgentEvent::ActivityUpdated {
            activity: activity(1),
            update: ActivityUpdate::TextSnapshot(snapshot.clone()),
        })
        .unwrap();
        assert!(
            tool.session_output(&pin)
                .unwrap()
                .unwrap()
                .contains(&snapshot)
        );
    }
}
