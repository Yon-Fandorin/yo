use super::{id, render_into, rendered_row};
use crate::{
    surface::Size,
    transcript::{TranscriptBody, TranscriptLayoutConfig, TranscriptState, TranscriptViewState},
};

// 일반 텍스트 출력도 화면과 같은 marker·들여쓰기·턴 간격을 사용해 종료 뒤 대화 맥락을 보존한다.
#[test]
fn plain_output_preserves_the_transcript_projection() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "가\n나".to_owned())
        .expect("unique user item");
    transcript.start_assistant(id(2)).expect("unique assistant");
    transcript
        .append_text(id(2), "답")
        .expect("streaming assistant");

    let output = transcript
        .plain_output(&TranscriptLayoutConfig::default())
        .unwrap();

    assert_eq!(output.as_deref(), Some("❯ 가\n  나\n\n• 답\n"));
}

// 빈 streaming 항목만 있으면 종료 뒤 의미 없는 marker나 빈 줄을 출력하지 않는다.
#[test]
fn empty_streaming_transcript_has_no_plain_output() {
    let mut transcript = TranscriptState::new();
    transcript.start_assistant(id(1)).expect("unique assistant");

    let output = transcript
        .plain_output(&TranscriptLayoutConfig::default())
        .unwrap();

    assert_eq!(output, None);
}

// terminal control 문자는 원문 byte로 재실행하지 않고 화면과 같은 가시 표기로 안전하게 남긴다.
#[test]
fn plain_output_projects_control_characters_safely() {
    let mut transcript = TranscriptState::new();
    transcript
        .push_user(id(1), "before\u{1b}after".to_owned())
        .expect("unique user item");

    let output = transcript
        .plain_output(&TranscriptLayoutConfig::default())
        .unwrap();

    assert_eq!(output.as_deref(), Some("❯ before^[after\n"));
}

// 화면의 Markdown 장식 제거가 저장/종료 출력이나 user·도구 원문으로 전파되지 않는다.
#[test]
fn markdown_rendering_preserves_source_output_and_plain_messages() {
    let source = "## Heading\n\n**bold** and `code`\n\n```rust\n  let value = 1;\n```";
    let mut transcript = TranscriptState::new();
    transcript.start_markdown_assistant(id(1)).unwrap();
    transcript.append_text(id(1), source).unwrap();
    let config = TranscriptLayoutConfig::default();
    let expected =
        "• ## Heading\n\n  **bold** and `code`\n\n  ```rust\n    let value = 1;\n  ```\n";
    assert_eq!(
        transcript.plain_output(&config).unwrap().as_deref(),
        Some(expected)
    );
    let (surface, _) = render_into(
        &transcript,
        Size::new(60, 20),
        &config,
        &mut TranscriptViewState::default(),
        None,
    );
    assert_eq!(rendered_row(&surface, 0), "• Heading");
    assert_eq!(rendered_row(&surface, 2), "  bold and code");
    let TranscriptBody::Message(message) = transcript.items()[0].body();
    assert_eq!(message.text(), source);
    for user_message in [false, true] {
        let mut plain = TranscriptState::new();
        if user_message {
            plain.push_user(id(1), "**literal**".into()).unwrap();
        } else {
            plain.start_assistant(id(1)).unwrap();
            plain.append_text(id(1), "**literal**").unwrap();
        }
        let (surface, _) = render_into(
            &plain,
            Size::new(30, 5),
            &config,
            &mut TranscriptViewState::default(),
            None,
        );
        assert!(rendered_row(&surface, 0).contains("**literal**"));
    }
}

// 개별 항목은 화면 한도 안이어도 대화 합계가 u16 행 한도를 넘는 경우 마지막 답변까지 내보낸다.
// 빈 줄·항목 간격·부분 대화의 선행 간격도 기존 투영과 동일하게 보존한다.
#[test]
fn plain_output_has_no_conversation_wide_surface_height_limit() {
    let mut transcript = TranscriptState::new();
    let source = "line\n".repeat(32_768);
    for index in 1..=3 {
        transcript.start_assistant(id(index)).unwrap();
        transcript.append_text(id(index), &source).unwrap();
    }
    transcript
        .push_user(id(4), "last question".to_owned())
        .unwrap();
    transcript.start_assistant(id(5)).unwrap();
    transcript.append_text(id(5), "last answer").unwrap();
    let config = TranscriptLayoutConfig::default();
    let output = transcript.plain_output(&config).unwrap().unwrap();
    assert_eq!(output.matches("line").count(), 3 * 32_768);
    assert_eq!(output.matches("• line").count(), 3);
    assert!(output.ends_with("❯ last question\n\n• last answer\n"));
    assert!(output.contains("  line\n\n\n• line"));
    let suffix = transcript
        .plain_output_slice(transcript.suffix(3), &config)
        .unwrap()
        .unwrap();
    assert_eq!(suffix, "\n\n❯ last question\n\n• last answer\n");
}

// 큰 구조화 도구 출력은 JSON으로 우회하지 않고 본문·제어 문자 표기·실패 footer를 끝까지 내보낸다.
#[test]
fn plain_output_pages_one_large_typed_tool_without_schema_fallback() {
    use yo_core::{ActivityKind, ToolOutput};

    use crate::transcript::TranscriptActivityOutcome;

    let mut transcript = TranscriptState::new();
    let body = format!("{}END\u{1b}", "한글\n".repeat(70_000));
    let profile = ToolOutput {
        tool: "run_command".into(),
        server: None,
        arguments: None,
        result: None,
        content_items: None,
        error: None,
        plain_text: body,
    }
    .to_snapshot()
    .unwrap();
    transcript
        .start_typed_activity_message(id(1), ActivityKind::ToolResult)
        .unwrap();
    transcript
        .append_text(id(1), &format!("Tool failed\n{profile}"))
        .unwrap();
    transcript
        .finish_activity_message(
            id(1),
            TranscriptActivityOutcome::Failed,
            Some("\nFailure: exit 1"),
        )
        .unwrap();
    let config = TranscriptLayoutConfig::default();
    let text = transcript.plain_output(&config).unwrap().unwrap();
    assert!(text.starts_with("• Tool failed\n  한글\n"));
    assert_eq!(text.matches("한글").count(), 70_000);
    assert!(text.ends_with("  END^[\n  Failure: exit 1\n"));
    assert!(!text.contains("yo.tool-output"));
    assert!(!text.contains('\u{1b}'));
}

// 의미별 모델 활동도 대형 본문을 원본 JSON으로 바꾸지 않으며 계획 상태·요약 메타데이터를 유지한다.
#[test]
fn plain_output_pages_large_model_profiles_and_plan_continuation_indent() {
    use std::num::NonZeroU16;

    use yo_core::{
        ActivityDocument, ActivityKind, ActivityNotice, ActivityPlan, ActivitySummary, NoticeLevel,
        PlanStep, PlanStepStatus, SummaryKind,
    };

    let text = format!("{}END", "line\n".repeat(70_000));
    let profiles = [
        (
            ActivityDocument {
                title: "Document".into(),
                markdown: text.clone(),
            }
            .to_snapshot()
            .unwrap(),
            "Document",
        ),
        (
            ActivityNotice {
                title: "Warning".into(),
                message: text.clone(),
                level: NoticeLevel::Warning,
            }
            .to_snapshot()
            .unwrap(),
            "Warning",
        ),
        (
            ActivitySummary {
                kind: SummaryKind::Compaction,
                summary: text.clone(),
                tokens_before: Some(123),
            }
            .to_snapshot()
            .unwrap(),
            "Before compaction: 123",
        ),
        (
            ActivityPlan {
                explanation: None,
                steps: vec![PlanStep {
                    text,
                    status: PlanStepStatus::InProgress,
                }],
            }
            .to_snapshot()
            .unwrap(),
            "[>] line",
        ),
    ];
    for (profile, heading) in profiles {
        let mut transcript = TranscriptState::new();
        transcript
            .start_typed_activity_message(id(1), ActivityKind::ModelWork)
            .unwrap();
        transcript
            .append_text(id(1), &format!("Model work\n{profile}"))
            .unwrap();
        let config = TranscriptLayoutConfig::default().with_max_body_width(NonZeroU16::new(24));
        let output = transcript.plain_output(&config).unwrap().unwrap();
        assert!(output.contains(heading), "missing {heading}");
        assert_eq!(output.matches("line").count(), 70_000);
        assert!(output.ends_with("END\n"));
        assert!(!output.contains("yo.activity-"));
        if heading.starts_with("[>]") {
            assert!(output.contains("\n      line\n"));
        }
    }
}

// 실행 메타데이터는 좁은 실제 surface에서도 개별 JSON 상자를 반복하지 않으며,
// 원본·plain 출력과 사용자 지정 renderer에 전달되는 완전한 ToolOutput은 변하지 않습니다.
#[test]
fn execution_result_details_wrap_compactly_without_changing_raw_or_custom_output() {
    use yo_core::{ActivityKind, ToolOutput};

    use crate::transcript::ToolRenderer;

    let output = ToolOutput {
        tool: "run_command".into(),
        server: None,
        arguments: Some(serde_json::json!({"command":"printf proof"})),
        result: Some(serde_json::json!({
            "content":[{"type":"text","text":"OUTPUT_PROOF"}],
            "call_id":"call-proof", "tool_id":"command-proof", "execution_host":"host-proof",
            "outcome":"completed", "truncated":false, "isError":false
        })),
        content_items: None,
        error: None,
        plain_text: "EXACT RAW OUTPUT\nunchanged".into(),
    };
    let snapshot = output.to_snapshot().unwrap();
    let source = format!("Tool result\n{snapshot}");
    let mut transcript = TranscriptState::new();
    transcript
        .start_typed_activity_message(id(1), ActivityKind::ToolResult)
        .unwrap();
    transcript.append_text(id(1), &source).unwrap();
    let config = TranscriptLayoutConfig::default();
    let original_plain = transcript.plain_output(&config).unwrap();
    for width in [20, 40] {
        let (surface, _) = render_into(
            &transcript,
            Size::new(width, 120),
            &config,
            &mut TranscriptViewState::default(),
            None,
        );
        let rows = (0..120)
            .map(|row| rendered_row(&surface, row))
            .collect::<Vec<_>>();
        let joined = rows.join("\n");
        let compact = joined
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        for value in [
            "Executiondetails",
            "Status:completed",
            "Tool:command-proof",
            "Call:call-proof",
            "Host:host-proof",
            "Error:false",
            "OUTPUT_PROOF",
        ] {
            assert!(
                compact.contains(value),
                "missing {value} at {width}: {joined}"
            );
        }
        assert!(
            !joined.contains("json"),
            "scalar metadata must not create JSON panels"
        );
        assert!(compact.find("OUTPUT_PROOF").unwrap() < compact.find("Executiondetails").unwrap());
        assert!(
            rows.iter().rposition(|row| !row.trim().is_empty()).unwrap() < 40,
            "execution details consumed excessive rows at {width}: {joined}"
        );
    }
    let TranscriptBody::Message(message) = transcript.items()[0].body();
    assert_eq!(message.text(), source);
    assert_eq!(transcript.plain_output(&config).unwrap(), original_plain);
    assert!(original_plain.unwrap().contains("EXACT RAW OUTPUT"));
    let custom = config.with_tool_renderer(Some(ToolRenderer::new(move |input| {
        assert_eq!(input.output, Some(&output));
        assert_eq!(input.source, "EXACT RAW OUTPUT\nunchanged");
        Some("CUSTOM_OUTPUT_PROOF".into())
    })));
    let (surface, _) = render_into(
        &transcript,
        Size::new(40, 20),
        &custom,
        &mut TranscriptViewState::default(),
        None,
    );
    let rendered = (0..20)
        .map(|row| rendered_row(&surface, row))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rendered.contains("CUSTOM_OUTPUT_PROOF"));
    assert!(!rendered.contains("Execution details"));
}

// 잘못된 값이나 새 메타데이터는 실행 상세로 오인하거나 숨기지 않고 generic JSON으로 남깁니다.
#[test]
fn malformed_and_unknown_execution_metadata_remain_visible() {
    use yo_core::{ActivityKind, ToolOutput};

    for (outcome, malformed) in [
        ("completed", false),
        ("failed", false),
        ("interrupted", false),
        ("completed", true),
    ] {
        let mut result = serde_json::json!({
            "content":[{"type":"text","text":"OUTPUT_PROOF"}],
            "call_id":"call-proof", "tool_id":"tool-proof", "execution_host":"host-proof",
            "outcome":outcome, "truncated":true, "isError":outcome != "completed",
            "retainedOutput":{"truncated":true},
            "future_key":"FUTURE_VALUE"
        });
        if malformed {
            result["isError"] = serde_json::json!("INVALID_BOOL");
        }
        let output = ToolOutput {
            tool: "custom_tool".into(),
            server: None,
            arguments: None,
            result: Some(result),
            content_items: None,
            error: None,
            plain_text: "raw".into(),
        };
        let mut transcript = TranscriptState::new();
        transcript
            .start_typed_activity_message(id(1), ActivityKind::ToolResult)
            .unwrap();
        transcript
            .append_text(
                id(1),
                &format!("Tool result\n{}", output.to_snapshot().unwrap()),
            )
            .unwrap();
        let (surface, _) = render_into(
            &transcript,
            Size::new(40, 150),
            &TranscriptLayoutConfig::default(),
            &mut TranscriptViewState::default(),
            None,
        );
        let text = (0..150)
            .map(|row| rendered_row(&surface, row))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("future_key") && text.contains("FUTURE_VALUE"));
        let compact = text
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        assert!(compact.contains("Outputtruncated"));
        assert!(compact.contains("Retainedoutput"));
        assert!(compact.contains("Somecapturedoutputwasomittedorunavailable."));
        if malformed {
            assert!(text.contains("INVALID_BOOL"));
            assert!(!text.contains("Execution details"));
        } else {
            assert!(text.contains("Execution details"));
            assert!(compact.contains(&format!("Status:{outcome}")));
            if outcome != "completed" {
                assert!(!compact.contains("Status:completed"));
            }
            assert!(!compact.contains("Truncated:true"));
        }
    }
}
