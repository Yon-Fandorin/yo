use super::super::*;

// 첫 agent_message_chunk가 prompt 수락 증거가 되고, 같은 message의 text delta와 종료가
// 하나의 AgentMessage Activity 및 재개 가능한 완료 Turn으로 순서대로 노출됩니다.
#[test]
fn maps_prompt_stream_and_completion_to_semantic_events() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        response(3, json!({ "sessionId": "grok-session-a" })),
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "grok-session-a",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "messageId": "message-a",
                    "content": { "type": "text", "text": "hello" }
                }
            }
        }),
        response(
            4,
            json!({
                "stopReason": "end_turn",
                "usage": {
                    "inputTokens": 100,
                    "outputTokens": 25,
                    "totalTokens": 125,
                    "thoughtTokens": 10,
                    "cachedReadTokens": 60,
                    "cachedWriteTokens": 5
                }
            }),
        ),
    ];
    let (mut backend, sent) = backend(messages);
    create_session(&mut backend, session_id);

    let evidence = backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("say hello"),
        })
        .unwrap();
    assert!(matches!(
        evidence,
        BackendCommandEvidence::RequestAccepted(_)
    ));
    assert_eq!(sent.0.borrow()[3]["method"], "session/prompt");

    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityStarted {
            kind: ActivityKind::AgentMessage,
            ..
        })
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityUpdated { .. })
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished {
            outcome: ActivityOutcome::Completed,
            ..
        })
    ));
    let BackendPoll::Event(BackendEvent::ActivityStarted {
        activity: usage_activity,
        kind: ActivityKind::ModelWork,
    }) = backend.poll_event().unwrap()
    else {
        panic!("Grok prompt usage must start one ModelWork Activity");
    };
    let BackendPoll::Event(BackendEvent::ActivityUpdated {
        activity,
        update: yo_core::ActivityUpdate::TextSnapshot(receipt),
    }) = backend.poll_event().unwrap()
    else {
        panic!("Grok prompt usage must be emitted as one durable text snapshot");
    };
    assert_eq!(activity, usage_activity);
    assert_eq!(
        serde_json::from_str::<Value>(&receipt).unwrap(),
        json!({
            "schema": "grok.acp-prompt-usage-receipt/v1",
            "source_profile": "grok.acp.prompt-response.usage/v1",
            "prompt_request_id": 4,
            "usage": {
                "input_tokens": 100,
                "output_tokens": 25,
                "total_tokens": 125,
                "reasoning_tokens": 10,
                "cache_read_input_tokens": 60,
                "cache_write_input_tokens": 5
            }
        })
    );
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed,
        }) if activity == usage_activity
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ResumableTurnFinished { turn, .. })
            if turn == active_turn
    ));
}

// Grok Build 1.0.5가 표준 PromptResponse.usage 대신 제공하는 `_meta.usage`의
// whole-prompt 값만 턴별 영수증으로 보존하고 sibling last-call 값은 사용하지 않습니다.
#[test]
fn maps_grok_meta_prompt_usage_to_semantic_receipt() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        response(3, json!({ "sessionId": "grok-session-a" })),
        response(
            4,
            json!({
                "stopReason": "end_turn",
                "_meta": {
                    "inputTokens": 10,
                    "outputTokens": 2,
                    "cachedReadTokens": 1,
                    "reasoningTokens": 1,
                    "usage": {
                        "inputTokens": 14_851,
                        "outputTokens": 48,
                        "totalTokens": 14_899,
                        "cachedReadTokens": 11_648,
                        "cacheCreationTokens": 7,
                        "reasoningTokens": 34,
                        "modelCalls": 1,
                        "numTurns": 1
                    }
                }
            }),
        ),
    ];
    let (mut backend, _) = backend(messages);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("test"),
        })
        .unwrap();

    let BackendPoll::Event(BackendEvent::ActivityStarted {
        activity: usage_activity,
        kind: ActivityKind::ModelWork,
    }) = backend.poll_event().unwrap()
    else {
        panic!("Grok meta prompt usage must start one ModelWork Activity");
    };
    let BackendPoll::Event(BackendEvent::ActivityUpdated {
        activity,
        update: yo_core::ActivityUpdate::TextSnapshot(receipt),
    }) = backend.poll_event().unwrap()
    else {
        panic!("Grok meta prompt usage must be emitted as one durable text snapshot");
    };
    assert_eq!(activity, usage_activity);
    assert_eq!(
        serde_json::from_str::<Value>(&receipt).unwrap(),
        json!({
            "schema": "grok.acp-prompt-usage-receipt/v1alpha1",
            "source_profile": "grok.acp.prompt-response.meta-usage/v1",
            "prompt_request_id": 4,
            "model_calls": 1,
            "num_turns": 1,
            "usage": {
                "input_tokens": 14_851,
                "output_tokens": 48,
                "total_tokens": 14_899,
                "reasoning_tokens": 34,
                "cache_read_input_tokens": 11_648,
                "cache_write_input_tokens": 7
            }
        })
    );
}

// Grok이 whole-prompt ledger의 과소 집계 가능성을 표시하면 완전한 영수증으로
// 오인하지 않고 usage Activity 없이 원래 완료 Turn만 보존합니다.
#[test]
fn omits_incomplete_grok_meta_prompt_usage() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        response(3, json!({ "sessionId": "grok-session-a" })),
        response(
            4,
            json!({
                "stopReason": "end_turn",
                "_meta": {
                    "usage": {
                        "inputTokens": 100,
                        "outputTokens": 25,
                        "totalTokens": 125,
                        "cachedReadTokens": 60,
                        "cacheCreationTokens": 5,
                        "reasoningTokens": 10,
                        "usageIsIncomplete": true
                    }
                }
            }),
        ),
    ];
    let (mut backend, _) = backend(messages);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("test"),
        })
        .unwrap();

    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ResumableTurnFinished { turn, .. })
            if turn == active_turn
    ));
}

// Grok prompt usage의 필수 token 값이 음수이면 영수증을 추측 보정하지 않고 Turn 종료
// 전에 Protocol 실패로 닫아 잘못된 cache 수치가 durable Activity가 되지 않게 합니다.
#[test]
fn rejects_malformed_grok_prompt_usage() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        response(3, json!({ "sessionId": "grok-session-a" })),
        response(
            4,
            json!({
                "stopReason": "end_turn",
                "usage": {
                    "inputTokens": 100,
                    "outputTokens": 25,
                    "totalTokens": 125,
                    "thoughtTokens": 10,
                    "cachedReadTokens": -1,
                    "cachedWriteTokens": 5
                }
            }),
        ),
    ];
    let (mut backend, _) = backend(messages);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("test"),
        })
        .unwrap();

    let failure = backend.poll_event().unwrap_err();

    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
    assert!(failure.message().contains("cachedReadTokens"));
}

// Grok vendor whole-prompt usage가 필수 cache 값을 잘못 보내면 sibling 값이나 0으로
// 대체하지 않고 표준 usage와 같은 Protocol 실패 경계를 유지합니다.
#[test]
fn rejects_malformed_grok_meta_prompt_usage() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        response(3, json!({ "sessionId": "grok-session-a" })),
        response(
            4,
            json!({
                "stopReason": "end_turn",
                "_meta": {
                    "usage": {
                        "inputTokens": 100,
                        "outputTokens": 25,
                        "totalTokens": 125,
                        "cachedReadTokens": -1,
                        "cacheCreationTokens": 5,
                        "reasoningTokens": 10
                    }
                }
            }),
        ),
    ];
    let (mut backend, _) = backend(messages);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("test"),
        })
        .unwrap();

    let failure = backend.poll_event().unwrap_err();

    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
    assert!(failure.message().contains("cachedReadTokens"));
}

// prompt가 permission JSON-RPC request보다 먼저 끝났다고 수락하면 wire request가 영원히
// 미응답으로 남으므로, 선택 또는 취소 없이 온 terminal response는 Protocol 실패로 닫습니다.
#[test]
fn rejects_prompt_completion_with_an_unresolved_permission_request() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        response(3, json!({ "sessionId": "grok-session-a" })),
        json!({
            "jsonrpc": "2.0",
            "id": "permission-a",
            "method": "session/request_permission",
            "params": {
                "sessionId": "grok-session-a",
                "toolCall": { "toolCallId": "tool-a", "title": "Run cargo test" },
                "options": [
                    { "optionId": "once", "name": "Once", "kind": "allow_once" },
                    { "optionId": "reject", "name": "Reject", "kind": "reject_once" }
                ]
            }
        }),
        response(4, json!({ "stopReason": "end_turn" })),
    ];
    let (mut backend, _) = backend(messages);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("test"),
        })
        .unwrap();

    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityStarted {
            kind: ActivityKind::ApprovalRequest { .. },
            ..
        })
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityUpdated { .. })
    ));
    let failure = backend.poll_event().unwrap_err();

    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
    assert!(failure.message().contains("unresolved permission"));
}

// Grok도 같은 검증된 지침 snapshot을 ACP text 입력으로 받아 Codex 전용 이름 해석에 의존하지 않는다.
#[test]
fn resolved_skill_snapshot_reaches_grok_prompt() {
    use yo_core::{InputReference, ResolvedSkill, SkillReference, SkillReferenceScope};
    let reference = SkillReference::new(
        "review",
        "host",
        "/skills/review/SKILL.md",
        "review",
        SkillReferenceScope::User,
        1,
        "revision",
    );
    let input = UserInput::with_references(
        "use $review",
        vec![InputReference::skill(4..11, reference.clone())],
    )
    .unwrap()
    .with_resolved_skill(
        ResolvedSkill::new(reference, "# Review\nVerify the exact implementation.").unwrap(),
    )
    .unwrap();
    let expected = input.model_input().to_owned();
    let session_id = session(1);
    let (mut backend, sent) = backend([
        response(3, json!({"sessionId":"grok-session-a"})),
        response(4, json!({"stopReason":"end_turn"})),
    ]);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(session_id, 1),
            input,
        })
        .unwrap();
    let sent = sent.0.borrow();
    let request = sent
        .iter()
        .find(|request| request["method"] == "session/prompt")
        .unwrap();
    assert_eq!(
        request["params"]["prompt"],
        json!([{"type":"text", "text":expected}])
    );
}
