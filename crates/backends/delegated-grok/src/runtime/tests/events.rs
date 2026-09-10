use yo_core::{ActivityApproval, ToolOutput};

use super::*;

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

// 빈 title도 name과 rawInput에서 실행 내용을 복원할 수 있으면 operation이 보이는 단발
// 승인으로 투영하고, 선택한 원래 optionId를 동일 wire request에 돌려줍니다.
#[test]
fn maps_permission_options_and_returns_the_selected_once_decision() {
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
                "toolCall": {
                    "toolCallId": "tool-a",
                    "title": "",
                    "name": "terminal",
                    "rawInput": { "command": "cargo test" }
                },
                "options": [
                    { "optionId": "always", "name": "Always", "kind": "allow_always" },
                    { "optionId": "once", "name": "Once", "kind": "allow_once" },
                    { "optionId": "reject", "name": "Reject", "kind": "reject_once" }
                ]
            }
        }),
        response(4, json!({ "stopReason": "end_turn" })),
    ];
    let (mut backend, sent) = backend(messages);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("test"),
        })
        .unwrap();

    let BackendPoll::Event(BackendEvent::ActivityStarted {
        activity,
        kind: ActivityKind::ApprovalRequest { request_id },
    }) = backend.poll_event().unwrap()
    else {
        panic!("permission must start an approval request");
    };
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityUpdated {
            activity: observed,
            update: yo_core::ActivityUpdate::TextSnapshot(summary),
        }) if observed == activity && ActivityApproval::from_snapshot(&summary).is_some_and(|profile| profile.plain_text.starts_with("terminal: {\"command\":\"cargo test\"}"))
    ));
    let request = yo_core::ActivityRequestRef::new(activity, request_id);
    backend
        .execute_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::Approval(ApprovalDecision::Approved),
        })
        .unwrap();

    let sent = sent.0.borrow();
    let response = sent.last().unwrap();
    assert_eq!(response["id"], "permission-a");
    assert_eq!(response["result"]["outcome"]["outcome"], "selected");
    assert_eq!(response["result"]["outcome"]["optionId"], "once");
}

// process가 제한 argv를 무시하거나 vendor 동작이 바뀌어 permission request가 와도,
// read-only review는 사용자 승인으로 승격하지 않고 wire 거절 뒤 Protocol 실패합니다.
#[test]
fn read_only_review_rejects_permission_requests_without_an_approval_activity() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let (mut backend, sent) = backend_with_profile(
        [
            response(3, json!({ "sessionId": "grok-session-a" })),
            permission_request("permission-a", Some("Run cargo test")),
        ],
        true,
    );
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("review"),
        })
        .unwrap();

    let failure = backend.poll_event().unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
    assert!(failure.message().contains("read-only delegated review"));
    assert!(backend.approvals.is_empty());
    assert!(backend.pending_events.is_empty());
    let rejection = sent.0.borrow().last().cloned().unwrap();
    assert_eq!(rejection["id"], "permission-a");
    assert_eq!(rejection["error"]["code"], -32000);
}

// permission request가 실행 가능한 tool 제목 없이 도착하면 승인 Activity를 만들거나
// 사용자에게 빈 요청을 노출하지 않고 Protocol 실패로 닫습니다.
#[test]
fn rejects_permission_requests_without_an_actionable_tool_title() {
    for tool_call in [
        json!({}),
        json!({ "title": "" }),
        json!({ "title": "   " }),
        json!({ "name": "terminal", "rawInput": { "command": "   " } }),
        json!({ "name": "terminal", "rawInput": [null, " "] }),
        json!({ "name": "terminal", "rawInput": { "nested": { "value": null } } }),
    ] {
        let session_id = session(1);
        let active_turn = turn(session_id, 1);
        let mut permission = permission_request("permission-a", None);
        permission["params"]["toolCall"] = tool_call;
        let (mut backend, sent) = backend([
            response(3, json!({ "sessionId": "grok-session-a" })),
            permission,
        ]);
        create_session(&mut backend, session_id);
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: active_turn,
                input: UserInput::from("test"),
            })
            .unwrap();

        let failure = backend.poll_event().unwrap_err();
        assert_eq!(failure.kind(), BackendFailureKind::Protocol);
        assert!(failure.message().contains("actionable"));
        assert!(backend.approvals.is_empty());
        assert!(backend.pending_events.is_empty());
        assert_eq!(
            sent.0.borrow().last().unwrap(),
            &json!({
                "jsonrpc": "2.0",
                "id": "permission-a",
                "result": { "outcome": { "outcome": "selected", "optionId": "reject" } }
            })
        );
    }
}

// 서로 다른 update로 온 tool name과 rawInput은 하나의 ToolCall identity로 누적하고,
// 승인 뒤의 반복 name은 이를 축소하지 않으며 결과는 별도 ToolResult로 투영합니다.
#[test]
fn keeps_tool_call_identity_when_tool_result_follows_an_approval_round_trip() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let mut call = tool_call("tool-a", "in_progress", Some(""));
    call["params"]["update"]["name"] = json!("terminal");
    let messages = [
        response(3, json!({ "sessionId": "grok-session-a" })),
        call,
        tool_raw_input_update("tool-a", json!({ "command": "cargo test" })),
        permission_request("permission-a", Some("Run cargo test")),
        tool_result("tool-a", "all tests passed"),
        response(4, json!({ "stopReason": "end_turn" })),
    ];
    let (mut backend, _) = backend(messages);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("run tests"),
        })
        .unwrap();

    let tool_activity = expect_activity_started(&mut backend, ActivityKind::ToolCall);
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityUpdated {
            activity,
            update: yo_core::ActivityUpdate::TextSnapshot(identity),
        }) if activity == tool_activity && identity == "terminal"
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityUpdated {
            activity,
            update: yo_core::ActivityUpdate::TextSnapshot(identity),
        }) if activity == tool_activity && identity == "terminal: {\"command\":\"cargo test\"}"
    ));

    let (approval_activity, request_id) = match backend.poll_event().unwrap() {
        BackendPoll::Event(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ApprovalRequest { request_id },
        }) => (activity, request_id),
        other => panic!("permission must start an approval request, got {other:?}"),
    };
    expect_activity_update(&mut backend, approval_activity);
    let request = yo_core::ActivityRequestRef::new(approval_activity, request_id);
    backend
        .execute_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::Approval(ApprovalDecision::Approved),
        })
        .unwrap();

    let approval_finished = backend.poll_event().unwrap();
    assert!(matches!(
        approval_finished,
        BackendPoll::Event(BackendEvent::ActivityFinished { activity, .. })
            if activity == approval_activity
    ));
    let response_activity = match backend.poll_event().unwrap() {
        BackendPoll::Event(BackendEvent::ActivityStarted {
            activity,
            kind:
                ActivityKind::ApprovalResponse {
                    request_id: observed,
                },
        }) => {
            assert_eq!(observed, request_id);
            activity
        },
        other => panic!("approval response must start an Activity, got {other:?}"),
    };
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityUpdated { activity, update:yo_core::ActivityUpdate::TextSnapshot(text) })
            if activity == response_activity && text.contains("Allow this operation once.")
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished { activity, .. })
            if activity == response_activity
    ));

    let result_activity = expect_activity_started(&mut backend, ActivityKind::ToolResult);
    assert_ne!(result_activity, tool_activity);
    let BackendPoll::Event(BackendEvent::ActivityUpdated {
        activity,
        update: yo_core::ActivityUpdate::TextSnapshot(result),
    }) = backend.poll_event().unwrap()
    else {
        panic!("tool content must update the ToolResult Activity");
    };
    assert_eq!(activity, result_activity);
    let output = ToolOutput::from_snapshot(&result).unwrap();
    assert_eq!(output.tool, "terminal");
    assert_eq!(output.arguments, Some(json!({"command":"cargo test"})));
    assert_eq!(
        output.content_items,
        Some(json!([{"type":"text","text":"all tests passed"}]))
    );
    assert!(output.plain_text.contains("all tests passed"));
    for activity in [result_activity, tool_activity] {
        assert!(matches!(
            backend.poll_event().unwrap(),
            BackendPoll::Event(BackendEvent::ActivityFinished {
                activity: observed,
                outcome: ActivityOutcome::Completed,
            }) if observed == activity
        ));
    }
}

// name과 rawInput이 어느 순서로 나뉘어 도착해도 기존 field를 잃지 않고 같은 ToolCall
// Activity의 `name: input` snapshot으로 합쳐 부분 update 순서에 의존하지 않습니다.
#[test]
fn merges_split_tool_identity_in_either_arrival_order() {
    for (mut initial, mut later, first_snapshot) in [
        (
            json!({ "name": "terminal" }),
            json!({ "rawInput": { "command": "cargo test" } }),
            "terminal",
        ),
        (
            json!({ "rawInput": { "command": "cargo test" } }),
            json!({ "name": "terminal" }),
            "{\"command\":\"cargo test\"}",
        ),
    ] {
        for update in [&mut initial, &mut later] {
            update["toolCallId"] = json!("tool-a");
            update["status"] = json!("in_progress");
        }
        let session_id = session(1);
        let (mut backend, _) = backend([
            response(3, json!({ "sessionId": "grok-session-a" })),
            session_update("tool_call", initial),
            session_update("tool_call_update", later),
        ]);
        create_session(&mut backend, session_id);
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("inspect"),
            })
            .unwrap();

        let activity = expect_activity_started(&mut backend, ActivityKind::ToolCall);
        for expected in [first_snapshot, "terminal: {\"command\":\"cargo test\"}"] {
            assert!(matches!(
                backend.poll_event().unwrap(),
                BackendPoll::Event(BackendEvent::ActivityUpdated {
                    activity: observed,
                    update: yo_core::ActivityUpdate::TextSnapshot(snapshot),
                }) if observed == activity && snapshot == expected
            ));
        }
    }
}

// messageId가 없는 agent/thought stream은 ToolCall과 승인 경계를 넘어서 같은 Activity를
// 재사용하지 않고, 각 경계 뒤에 새 Activity를 시작합니다.
#[test]
fn splits_unidentified_agent_and_thought_chunks_at_tool_and_approval_boundaries() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        response(3, json!({ "sessionId": "grok-session-a" })),
        text_update("agent_message_chunk", "before tool"),
        tool_call("tool-a", "in_progress", Some("Run cargo test")),
        text_update("agent_message_chunk", "after tool"),
        text_update("agent_thought_chunk", "before approval"),
        permission_request("permission-a", Some("Run cargo test")),
        text_update("agent_thought_chunk", "after approval"),
    ];
    let (mut backend, _) = backend(messages);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("inspect"),
        })
        .unwrap();

    let agent_before = expect_activity_started(&mut backend, ActivityKind::AgentMessage);
    expect_activity_update(&mut backend, agent_before);
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished { activity, .. })
            if activity == agent_before
    ));

    let tool_activity = expect_activity_started(&mut backend, ActivityKind::ToolCall);
    expect_activity_update(&mut backend, tool_activity);

    let agent_after = expect_activity_started(&mut backend, ActivityKind::AgentMessage);
    assert_ne!(agent_after, agent_before);
    expect_activity_update(&mut backend, agent_after);

    let thought_before = expect_activity_started(&mut backend, ActivityKind::ModelWork);
    expect_activity_update(&mut backend, thought_before);

    for activity in [agent_after, thought_before] {
        assert!(matches!(
            backend.poll_event().unwrap(),
            BackendPoll::Event(BackendEvent::ActivityFinished { activity: observed, .. })
                if observed == activity
        ));
    }
    let (approval_activity, request_id) = match backend.poll_event().unwrap() {
        BackendPoll::Event(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ApprovalRequest { request_id },
        }) => (activity, request_id),
        other => panic!("permission must start an approval request, got {other:?}"),
    };
    expect_activity_update(&mut backend, approval_activity);
    backend
        .execute_command(AgentCommand::RespondToActivity {
            request: yo_core::ActivityRequestRef::new(approval_activity, request_id),
            response: ActivityResponse::Approval(ApprovalDecision::Approved),
        })
        .unwrap();
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished { activity, .. })
            if activity == approval_activity
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityStarted {
            kind: ActivityKind::ApprovalResponse { .. },
            ..
        })
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityUpdated { update:yo_core::ActivityUpdate::TextSnapshot(text),.. })
            if text.contains("Response sent to agent.")
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished { .. })
    ));

    let thought_after = expect_activity_started(&mut backend, ActivityKind::ModelWork);
    assert_ne!(thought_after, thought_before);
    expect_activity_update(&mut backend, thought_after);
}

// Yo의 이진 승인은 단발 결정이므로 peer가 persistent 선택지만 제시할 때 이를 선택해
// 권한을 확대하지 않고 Protocol 실패로 닫습니다.
#[test]
fn rejects_permission_requests_without_once_options() {
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
                "options": [
                    { "optionId": "always", "kind": "allow_always" },
                    { "optionId": "never", "kind": "reject_always" }
                ]
            }
        }),
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
    assert!(failure.message().contains("allow_once"));
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

// 출력이 빈 완료 호출도 상관관계가 있는 ToolResult를 남기며, 그 ACP ToolCallId는 다음
// Turn까지 tombstone으로 유지되어 재사용을 새 Activity로 잘못 열지 않습니다.
#[test]
fn rejects_a_completed_tool_call_id_reused_in_a_later_turn() {
    let session_id = session(1);
    let first_turn = turn(session_id, 1);
    let second_turn = turn(session_id, 2);
    let completed_tool = || {
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "grok-session-a",
                "update": {
                    "sessionUpdate": "tool_call",
                    "toolCallId": "tool-a",
                    "status": "completed"
                }
            }
        })
    };
    let messages = [
        response(3, json!({ "sessionId": "grok-session-a" })),
        completed_tool(),
        response(4, json!({ "stopReason": "end_turn" })),
        completed_tool(),
    ];
    let (mut backend, _) = backend(messages);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: first_turn,
            input: UserInput::from("first"),
        })
        .unwrap();

    let tool_activity = expect_activity_started(&mut backend, ActivityKind::ToolCall);
    expect_activity_update(&mut backend, tool_activity);
    let result_activity = expect_activity_started(&mut backend, ActivityKind::ToolResult);
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityUpdated {
            activity,
            update: yo_core::ActivityUpdate::TextSnapshot(result),
        }) if activity == result_activity
            && ToolOutput::from_snapshot(&result).is_some_and(|output|
                output.tool == "tool-a" && output.content_items.is_none() && output.result.is_none())
    ));
    for activity in [result_activity, tool_activity] {
        assert!(matches!(
            backend.poll_event().unwrap(),
            BackendPoll::Event(BackendEvent::ActivityFinished {
                activity: observed,
                ..
            }) if observed == activity
        ));
    }
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ResumableTurnFinished { .. })
    ));

    backend
        .execute_command(AgentCommand::StartTurn {
            turn: second_turn,
            input: UserInput::from("second"),
        })
        .unwrap();
    let failure = backend.poll_event().unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
    assert!(failure.message().contains("duplicate Grok ACP tool call"));
}

// 상대가 한 Turn에 고유 messageId를 무제한 발급해 state를 키우지 못하도록, 정확한
// per-Turn 상한까지는 event를 투영하고 다음 신규 Activity는 삽입 전에 거절합니다.

// ACP 계획은 전체 목록을 교체하고 도구 호출 중에도 같은 Activity를 유지하며 빈 목록도 전달한다.
#[test]
fn grok_plan_replaces_steps_across_tools_and_closes_with_turn() {
    use yo_core::{ActivityPlan, ActivityUpdate, PlanStepStatus};
    for stop in ["end_turn", "cancelled"] {
        let session_id = session(1);
        let entry = |content: &str, status: &str| json!({"content":content,"priority":"high","status":status});
        let (mut backend, _) = backend([
            response(3, json!({"sessionId":"grok-session-a"})),
            session_update(
                "plan",
                json!({"entries":[entry("Read", "pending"), entry("Test", "pending")]}),
            ),
            tool_call("tool-a", "completed", Some("Read")),
            session_update("plan", json!({"entries":[entry("Revised", "in_progress")]})),
            session_update(
                "plan",
                json!({"entries": if stop == "cancelled" { vec![entry("Revised", "in_progress")] } else { vec![] }}),
            ),
            response(4, json!({"stopReason":stop})),
            session_update("plan", json!({"entries":[entry("Next", "completed")]})),
            response(5, json!({"stopReason":"end_turn"})),
        ]);
        create_session(&mut backend, session_id);
        let mut previous = None;
        for number in [1, 2] {
            backend
                .execute_command(AgentCommand::StartTurn {
                    turn: turn(session_id, number),
                    input: UserInput::from("plan"),
                })
                .unwrap();
            let mut plans = Vec::new();
            let mut finished = Vec::new();
            let mut ended = false;
            for _ in 0..64 {
                match backend.poll_event().unwrap() {
                    BackendPoll::Event(BackendEvent::ActivityUpdated {
                        activity,
                        update: ActivityUpdate::TextSnapshot(text),
                    }) => {
                        if let Some(plan) = ActivityPlan::from_snapshot(&text) {
                            plans.push((activity, plan));
                        }
                    },
                    BackendPoll::Event(BackendEvent::ActivityFinished { activity, outcome }) => {
                        finished.push((activity, outcome))
                    },
                    BackendPoll::Event(
                        BackendEvent::ResumableTurnFinished { .. }
                        | BackendEvent::TurnFinished { .. },
                    ) => {
                        ended = true;
                        break;
                    },
                    BackendPoll::Event(_) | BackendPoll::Pending => {},
                    other => panic!("unexpected plan event: {other:?}"),
                }
            }
            assert!(ended);
            let id = plans[0].0;
            assert_ne!(Some(id), previous);
            assert!(plans.iter().all(|(activity, _)| *activity == id));
            if number == 1 {
                assert_eq!(plans.len(), 3);
                assert_eq!(plans[0].1.steps.len(), 2);
                assert_eq!(plans[1].1.steps.len(), 1);
                assert_eq!(plans[1].1.steps[0].text, "[high] Revised");
                assert_eq!(plans[1].1.steps[0].status, PlanStepStatus::InProgress);
                assert_eq!(plans[2].1.steps.is_empty(), stop == "end_turn");
                if stop == "cancelled" {
                    assert_eq!(plans[2].1.steps[0].status, PlanStepStatus::InProgress);
                }
            } else {
                assert_eq!(plans[0].1.steps[0].status, PlanStepStatus::Completed);
            }
            let outcomes: Vec<_> = finished
                .iter()
                .filter(|(activity, _)| *activity == id)
                .collect();
            assert_eq!(outcomes.len(), 1);
            assert!(
                matches!(&outcomes[0].1, ActivityOutcome::Interrupted)
                    == (number == 1 && stop == "cancelled")
            );
            previous = Some(id);
        }
    }
}

// 잘못된 계획과 표시 한도 초과는 Activity를 열기 전에 거부해 부분 계획을 남기지 않는다.
#[test]
fn grok_plan_rejects_invalid_entries_before_publishing() {
    use yo_core::{ActivityPlan, ActivityUpdate, PlanStep, PlanStepStatus};
    let overhead = ActivityPlan {
        explanation: None,
        steps: vec![PlanStep {
            text: "[medium] ".into(),
            status: PlanStepStatus::Pending,
        }],
    }
    .to_snapshot()
    .unwrap()
    .len();
    let limit = ToolOutput::MAX_SNAPSHOT_BYTES - overhead;
    for update in [
        json!({}),
        json!({"entries":[{"content":"x","priority":"urgent","status":"pending"}]}),
        json!({"entries":[{"content":"x","priority":"low","status":"unknown"}]}),
        json!({"entries":[{"content":"x".repeat(limit + 1),"priority":"medium","status":"pending"}]}),
    ] {
        let session_id = session(1);
        let (mut backend, _) = backend([
            response(3, json!({"sessionId":"grok-session-a"})),
            session_update("plan", update),
        ]);
        create_session(&mut backend, session_id);
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("plan"),
            })
            .unwrap();
        assert_eq!(
            backend.poll_event().unwrap_err().kind(),
            BackendFailureKind::Protocol
        );
        assert!(backend.messages.is_empty());
        assert!(backend.pending_events.is_empty());
    }
    let session_id = session(1);
    let (mut backend, _) = backend([
        response(3, json!({"sessionId":"grok-session-a"})),
        session_update(
            "plan",
            json!({"entries":[{"content":"x".repeat(limit),"priority":"medium","status":"pending"}]}),
        ),
    ]);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(session_id, 1),
            input: UserInput::from("plan"),
        })
        .unwrap();
    let activity = expect_activity_started(&mut backend, ActivityKind::ModelWork);
    let BackendPoll::Event(BackendEvent::ActivityUpdated {
        activity: observed,
        update: ActivityUpdate::TextSnapshot(text),
    }) = backend.poll_event().unwrap()
    else {
        panic!("bounded plan missing")
    };
    assert_eq!(observed, activity);
    assert_eq!(text.len(), ToolOutput::MAX_SNAPSHOT_BYTES);
    assert!(ActivityPlan::from_snapshot(&text).is_some());
}

// ACP 부분 갱신은 생략한 결과·인자를 유지하고 명시적인 빈 content만 지우며 원래 콘텐츠를 보존한다.
#[test]
fn grok_tool_output_preserves_partial_fields_and_explicit_empty_content() {
    use yo_core::ActivityUpdate;
    let session_id = session(1);
    let image = json!({"type":"image","mimeType":"image/png","data":"AA=="});
    let diff = json!({"type":"diff","path":"/work/file","oldText":"old","newText":"new"});
    let (mut backend, _) = backend([
        response(3, json!({"sessionId":"grok-session-a"})),
        session_update(
            "tool_call",
            json!({"toolCallId":"tool-a","name":"inspect","status":"in_progress","rawInput":{"path":"a"},"content":[{"type":"content","content":{"type":"text","text":"partial"}},{"type":"content","content":image},diff]}),
        ),
        session_update(
            "tool_call_update",
            json!({"toolCallId":"tool-a","rawOutput":{"matches":3},"_meta":{"source":"fixture"}}),
        ),
        session_update(
            "tool_call_update",
            json!({"toolCallId":"tool-a","content":[]}),
        ),
        session_update(
            "tool_call_update",
            json!({"toolCallId":"tool-a","rawInput":{"path":"b"}}),
        ),
        session_update(
            "tool_call_update",
            json!({"toolCallId":"tool-a","status":"failed"}),
        ),
        response(4, json!({"stopReason":"end_turn"})),
    ]);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(session_id, 1),
            input: UserInput::from("inspect"),
        })
        .unwrap();
    let mut outputs = Vec::new();
    let mut failed = Vec::new();
    let mut released = false;
    let mut ended = false;
    for _ in 0..64 {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(text),
            }) => {
                if let Some(output) = ToolOutput::from_snapshot(&text) {
                    outputs.push((activity, output));
                }
            },
            BackendPoll::Event(BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Failed(_),
            }) => {
                failed.push(activity);
                released = backend
                    .tools
                    .get("tool-a")
                    .is_some_and(|binding| binding.output == json!({}));
            },
            BackendPoll::Event(BackendEvent::ResumableTurnFinished { .. }) => {
                ended = true;
                break;
            },
            BackendPoll::Event(_) | BackendPoll::Pending => {},
            other => panic!("unexpected tool output: {other:?}"),
        }
    }
    assert!(ended);
    assert_eq!(outputs.len(), 4);
    let id = outputs[0].0;
    assert!(
        outputs
            .iter()
            .all(|(activity, output)| *activity == id && output.tool == "inspect")
    );
    assert_eq!(outputs[0].1.arguments, Some(json!({"path":"a"})));
    assert_eq!(outputs[0].1.content_items.as_ref().unwrap()[1], image);
    assert_eq!(
        outputs[0].1.content_items.as_ref().unwrap()[2]["source"],
        diff
    );
    assert!(
        outputs[0].1.content_items.as_ref().unwrap()[2]["text"]
            .as_str()
            .unwrap()
            .contains("-old\n")
    );
    assert_eq!(outputs[1].1.content_items, outputs[0].1.content_items);
    assert_eq!(
        outputs[1].1.result,
        Some(json!({"rawOutput":{"matches":3},"_meta":{"source":"fixture"}}))
    );
    assert_eq!(outputs[2].1.content_items, Some(json!([])));
    assert!(!outputs[2].1.plain_text.contains("partial"));
    assert_eq!(outputs[3].1.arguments, Some(json!({"path":"b"})));
    assert_eq!(outputs[3].1.result, outputs[1].1.result);
    assert!(failed.contains(&id));
    assert!(outputs.iter().all(|(_, output)| output.error.is_none()));
    assert!(released);
    assert!(backend.tools.is_empty());
}

// ACP 파일 비교는 문맥 hunk·새 파일·개행 없음·경계 크기를 보존하고 미지원 입력은 원문으로 남긴다.
#[test]
fn grok_diff_content_has_context_and_preserves_source_boundaries() {
    use yo_core::ActivityUpdate;
    let old = (0..40).map(|n| format!("line{n}\n")).collect::<String>();
    let new = old
        .replace("line1\n", "changed1\n")
        .replace("line30\n", "changed30\n");
    for (index, (old, new)) in [
        (json!(old), new),
        (Value::Null, "created".into()),
        (json!("same"), "same".into()),
        (json!(42), "invalid".into()),
        (Value::Null, "x".repeat(256 * 1024)),
        (Value::Null, "x".repeat(256 * 1024 + 1)),
    ]
    .into_iter()
    .enumerate()
    {
        let source = json!({"type":"diff","path":"/work/file\nname","oldText":old,"newText":new,"_meta":{"kept":true}});
        let session_id = session(1);
        let (mut backend, _) = backend([
            response(3, json!({"sessionId":"grok-session-a"})),
            session_update(
                "tool_call",
                json!({"toolCallId":"tool-a","status":"completed","content":[source.clone()]}),
            ),
        ]);
        create_session(&mut backend, session_id);
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("compare"),
            })
            .unwrap();
        let mut observed = None;
        for _ in 0..8 {
            if let BackendPoll::Event(BackendEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(text),
                ..
            }) = backend.poll_event().unwrap()
                && let Some(output) = ToolOutput::from_snapshot(&text)
            {
                observed = Some(output);
                break;
            }
        }
        let output = observed.expect("typed result");
        let item = &output.content_items.as_ref().unwrap()[0];
        if index == 3 || index == 5 {
            assert_eq!(item, &source);
            continue;
        }
        assert_eq!(item["source"], source);
        let text = item["text"].as_str().unwrap();
        if index == 0 {
            assert_eq!(text.matches("@@ -").count(), 2);
            assert!(text.contains("-line1\n+changed1\n"));
            assert!(!text.contains("line15"));
            assert!(text.starts_with("--- \"/work/file\\nname\"\n"));
        } else if index == 1 {
            assert!(text.starts_with("--- /dev/null\n"));
            assert!(text.contains("+created\n\\ No newline at end of file"));
        } else if index == 2 {
            assert!(text.is_empty());
        }
    }
}

// 파일 변경 본문은 같은 호출 ID에서 여러 파일·부분 갱신·명시적 삭제를 반영하고 실패 상태를
// 유지한다.
#[test]
fn grok_file_change_body_tracks_reported_diffs_and_clearing() {
    use yo_core::ActivityUpdate;
    let session_id = session(1);
    let first = json!({"type":"diff","path":"a.rs","oldText":"old\n","newText":"new\n"});
    let second = json!({"type":"diff","path":"b\n.rs","oldText":null,"newText":"created\n"});
    let (mut backend, _) = backend([
        response(3, json!({"sessionId":"grok-session-a"})),
        session_update(
            "tool_call",
            json!({"toolCallId":"edit-a","kind":"edit","title":"Patch files","status":"in_progress","content":[first,second]}),
        ),
        session_update(
            "tool_call_update",
            json!({"toolCallId":"edit-a","rawOutput":{"note":"partial"}}),
        ),
        session_update(
            "tool_call_update",
            json!({"toolCallId":"edit-a","content":[]}),
        ),
        session_update(
            "tool_call_update",
            json!({"toolCallId":"edit-a","status":"failed"}),
        ),
        response(4, json!({"stopReason":"end_turn"})),
    ]);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(session_id, 1),
            input: UserInput::from("edit"),
        })
        .unwrap();
    let mut call = None;
    let mut snapshots = Vec::new();
    let mut failed = false;
    for _ in 0..60 {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::FileChange,
            }) => {
                assert!(call.replace(activity).is_none());
            },
            BackendPoll::Event(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(text),
            }) if Some(activity) == call => snapshots.push(text),
            BackendPoll::Event(BackendEvent::ActivityFinished { activity, outcome })
                if Some(activity) == call =>
            {
                failed = matches!(outcome, ActivityOutcome::Failed(_))
            },
            BackendPoll::Event(BackendEvent::TurnFinished { .. }) => break,
            _ => {},
        }
    }
    assert!(failed);
    assert_eq!(snapshots.len(), 4);
    assert!(snapshots[1].starts_with("update: \"a.rs\"\n"));
    assert!(snapshots[1].contains("add: \"b\\n.rs\"\n"));
    assert!(snapshots[1].contains("-old\n+new\n"));
    assert!(snapshots[1].contains("+created\n"));
    assert_eq!(snapshots[1], snapshots[2]);
    assert_eq!(snapshots[0], snapshots[3]);
}

// 승인 선택은 원래 순번·범위·wire ID를 유지하고 관련 파일은 같은 미종료 호출에만 연결한다.
#[test]
fn grok_approval_choices_preserve_scope_and_exact_file_binding() {
    use yo_core::{ActivityRequestRef, ActivityUpdate};
    for (kind, call_id, status, linked) in [
        ("edit", "tool-a", "pending", true),
        ("read", "tool-a", "pending", false),
        ("edit", "other", "pending", false),
        ("edit", "tool-a", "completed", false),
    ] {
        for (ordinal, selected) in [(1, "always"), (3, "reject"), (4, "once"), (5, "never")] {
            let session_id = session(1);
            let (mut backend, sent) = backend([
                response(3, json!({"sessionId":"grok-session-a"})),
                session_update(
                    "tool_call",
                    json!({"toolCallId":call_id,"kind":kind,"status":status,"title":"Edit settings"}),
                ),
                json!({"jsonrpc":"2.0","id":"permission-a","method":"session/request_permission","params":{
                    "sessionId":"grok-session-a","toolCall":{"toolCallId":"tool-a","title":"Edit settings","rawInput":{"path":"settings.rs"}},
                    "options":[
                        {"optionId":"always","name":"Remember allowance","kind":"allow_always"},
                        {"optionId":"unknown","name":"Future decision","kind":"future"},
                        {"optionId":"reject","name":"Reject once","kind":"reject_once"},
                        {"optionId":"once","name":"Allow once","kind":"allow_once"},
                        {"optionId":"never","name":"Remember rejection","kind":"reject_always"}
                    ]
                }}),
            ]);
            create_session(&mut backend, session_id);
            backend
                .execute_command(AgentCommand::StartTurn {
                    turn: turn(session_id, 1),
                    input: UserInput::from("edit"),
                })
                .unwrap();
            let mut file = None;
            let mut request = None;
            let mut profile = None;
            for _ in 0..20 {
                match backend.poll_event().unwrap() {
                    BackendPoll::Event(BackendEvent::ActivityStarted {
                        activity,
                        kind: ActivityKind::FileChange,
                    }) => file = Some(activity.activity_id().get().get()),
                    BackendPoll::Event(BackendEvent::ActivityStarted {
                        activity,
                        kind: ActivityKind::ApprovalRequest { request_id },
                    }) => request = Some(ActivityRequestRef::new(activity, request_id)),
                    BackendPoll::Event(BackendEvent::ActivityUpdated {
                        update: ActivityUpdate::TextSnapshot(text),
                        ..
                    }) => {
                        if let Some(value) = ActivityApproval::from_snapshot(&text) {
                            profile = Some(value);
                            break;
                        }
                    },
                    _ => {},
                }
            }
            let profile = profile.unwrap();
            assert_eq!(profile.related_change, if linked { file } else { None });
            assert_eq!(profile.decline_choice, Some(3));
            assert_eq!(profile.choices.len(), 5);
            assert!(!profile.choices[1].enabled);
            assert!(profile.choices[0].description.contains("remember"));
            assert!(profile.plain_text.contains("settings.rs"));
            let request = request.unwrap();
            for invalid in [0, 2, 6] {
                assert!(
                    backend
                        .execute_command(AgentCommand::RespondToActivity {
                            request,
                            response: ActivityResponse::Approval(ApprovalDecision::Offered(
                                invalid
                            )),
                        })
                        .is_err()
                );
                assert!(backend.approvals.contains_key(&request));
                assert!(
                    !sent
                        .0
                        .borrow()
                        .iter()
                        .any(|message| message["id"] == "permission-a")
                );
            }
            backend
                .execute_command(AgentCommand::RespondToActivity {
                    request,
                    response: ActivityResponse::Approval(ApprovalDecision::Offered(ordinal)),
                })
                .unwrap();
            assert_eq!(
                sent.0.borrow().last().unwrap()["result"]["outcome"]["optionId"],
                selected
            );
            assert!(!backend.approvals.contains_key(&request));
            let mut receipt = None;
            for _ in 0..4 {
                if let BackendPoll::Event(BackendEvent::ActivityUpdated {
                    update: ActivityUpdate::TextSnapshot(text),
                    ..
                }) = backend.poll_event().unwrap()
                {
                    receipt = Some(text);
                }
            }
            let receipt = receipt.expect("sent decision receipt");
            assert!(receipt.contains(&profile.choices[ordinal as usize - 1].label));
            assert!(receipt.contains(&profile.choices[ordinal as usize - 1].description));
            assert!(!receipt.contains("settings.rs"));

            assert!(
                backend
                    .execute_command(AgentCommand::RespondToActivity {
                        request,
                        response: ActivityResponse::Approval(ApprovalDecision::Offered(ordinal)),
                    })
                    .is_err()
            );
        }
    }
}

// 승인 선택지 한도와 중복 ID는 게시 전에 검사하고 표시 한도 초과는 거절 응답으로 닫는다.
#[test]
fn grok_approval_bounds_and_duplicate_ids_fail_before_publication() {
    for mode in ["exact", "excess", "duplicate", "oversized"] {
        let count = if mode == "excess" { 65 } else { 64 };
        let mut options = (0..count).map(|index| json!({
            "optionId":format!("option-{index}"), "name":format!("Choice {index}"),
            "kind":if index == 0 { "allow_once" } else if index == 1 { "reject_once" } else { "allow_always" }
        })).collect::<Vec<_>>();
        if mode == "duplicate" {
            options[2]["optionId"] = options[0]["optionId"].clone();
        }
        let session_id = session(1);
        let (mut backend, sent) = backend([
            response(3, json!({"sessionId":"grok-session-a"})),
            json!({"jsonrpc":"2.0","id":"permission-a","method":"session/request_permission","params":{
                "sessionId":"grok-session-a", "toolCall":{"title":"Review edit", "rawInput":if mode == "oversized" { "x".repeat(ToolOutput::MAX_SNAPSHOT_BYTES) } else { "small".into() }}, "options":options
            }}),
        ]);
        create_session(&mut backend, session_id);
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("edit"),
            })
            .unwrap();
        if mode == "exact" {
            assert!(matches!(
                backend.poll_event().unwrap(),
                BackendPoll::Event(BackendEvent::ActivityStarted {
                    kind: ActivityKind::ApprovalRequest { .. },
                    ..
                })
            ));
            let BackendPoll::Event(BackendEvent::ActivityUpdated {
                update: yo_core::ActivityUpdate::TextSnapshot(text),
                ..
            }) = backend.poll_event().unwrap()
            else {
                panic!("approval profile required")
            };
            assert_eq!(
                ActivityApproval::from_snapshot(&text)
                    .unwrap()
                    .choices
                    .len(),
                64
            );
        } else {
            assert!(backend.poll_event().is_err());
            assert!(backend.approvals.is_empty());
            assert!(backend.pending_events.is_empty());
            if mode == "oversized" {
                assert_eq!(
                    sent.0.borrow().last().unwrap()["result"]["outcome"]["optionId"],
                    "option-1"
                );
            }
        }
    }
}

// 승인 전후 순서가 달라도 정확한 미종료 호출만 연결하고 ID 전용 요청은 관측된 설명을 사용한다.
#[test]
fn grok_permission_links_late_calls_and_resolves_known_details() {
    use yo_core::ActivityUpdate;
    for late in [false, true] {
        let session_id = session(1);
        let call = session_update(
            "tool_call",
            json!({"toolCallId":"target","kind":"edit","title":"Update settings","rawInput":{"path":"settings.rs"}}),
        );
        let permission = json!({"jsonrpc":"2.0","id":"permission-a","method":"session/request_permission","params":{
            "sessionId":"grok-session-a","toolCall":if late { json!({"toolCallId":"target","title":"Review settings"}) } else { json!({"toolCallId":"target"}) },
            "options":[{"optionId":"yes","kind":"allow_once","name":"Allow"},{"optionId":"no","kind":"reject_once","name":"Reject"}]
        }});
        let unrelated = session_update(
            "tool_call",
            json!({"toolCallId":"other","kind":"edit","title":"Other file"}),
        );
        let events = if late {
            vec![permission, unrelated, call]
        } else {
            vec![call, unrelated, permission]
        };
        let (mut backend, _) = backend(
            std::iter::once(response(3, json!({"sessionId":"grok-session-a"}))).chain(events),
        );
        create_session(&mut backend, session_id);
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("edit"),
            })
            .unwrap();
        let mut profiles = Vec::new();
        let mut owner = None;
        for _ in 0..30 {
            if let BackendPoll::Event(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(text),
            }) = backend.poll_event().unwrap()
                && let Some(profile) = ActivityApproval::from_snapshot(&text)
            {
                if let Some(owner) = owner {
                    assert_eq!(activity, owner);
                } else {
                    owner = Some(activity);
                }
                profiles.push(profile);
            }
            if profiles.len() == if late { 2 } else { 1 } {
                break;
            }
        }
        let expected = backend.tools["target"].activity.activity_id().get().get();
        let profile = profiles.last().unwrap();
        assert_eq!(profile.related_change, Some(expected));
        assert_ne!(
            profile.related_change,
            Some(backend.tools["other"].activity.activity_id().get().get())
        );
        if late {
            assert_eq!(profiles[0].related_change, None);
            assert_eq!(profiles[0].choices, profiles[1].choices);
            assert_eq!(profiles[0].plain_text, profiles[1].plain_text);
        } else {
            assert!(profile.plain_text.contains("Update settings"));
            assert!(profile.plain_text.contains("settings.rs"));
        }
    }
}

// 승인 요청의 명시적 새 인수는 이전 호출 인수보다 우선하며 예약 링크 바이트도 한도에 포함한다.
#[test]
fn grok_permission_summary_uses_current_arguments_and_reserves_link_bytes() {
    use yo_core::ActivityUpdate;
    let mut reserve = 0;
    for mode in ["arguments", "measure", "exact", "excess"] {
        let session_id = session(1);
        let mut events = vec![response(3, json!({"sessionId":"grok-session-a"}))];
        if mode == "arguments" {
            events.push(session_update("tool_call",json!({"toolCallId":"target","name":"terminal","rawInput":{"command":"old-command"}})));
        }
        let title_len = match mode {
            "exact" => 1 + reserve,
            "excess" => 2 + reserve,
            _ => 1,
        };
        let tool_call = if mode == "arguments" {
            json!({"toolCallId":"target","rawInput":{"command":"new-command"}})
        } else {
            json!({"toolCallId":"target","title":"x".repeat(title_len)})
        };
        events.push(json!({"jsonrpc":"2.0","id":"permission-a","method":"session/request_permission","params":{
            "sessionId":"grok-session-a","toolCall":tool_call,
            "options":[{"optionId":"yes","kind":"allow_once","name":"Allow"},{"optionId":"no","kind":"reject_once","name":"Reject"}]
        }}));
        let (mut backend, sent) = backend(events);
        create_session(&mut backend, session_id);
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("edit"),
            })
            .unwrap();
        let mut observed = None;
        let mut rejected = false;
        for _ in 0..12 {
            match backend.poll_event() {
                Ok(BackendPoll::Event(BackendEvent::ActivityUpdated {
                    update: ActivityUpdate::TextSnapshot(text),
                    ..
                })) => {
                    if let Some(profile) = ActivityApproval::from_snapshot(&text) {
                        observed = Some(profile);
                        break;
                    }
                },
                Err(_) => {
                    rejected = true;
                    break;
                },
                _ => {},
            }
        }
        if mode == "excess" {
            assert!(rejected);
            assert!(backend.approvals.is_empty());
            assert_eq!(
                sent.0.borrow().last().unwrap()["result"]["outcome"]["optionId"],
                "no"
            );
        } else {
            let mut profile = observed.expect("admitted approval");
            if mode == "arguments" {
                assert!(
                    profile
                        .plain_text
                        .contains("terminal: {\"command\":\"new-command\"}")
                );
                assert!(!profile.plain_text.contains("old-command"));
            } else {
                profile.related_change = Some(u64::MAX);
                let length = profile.to_snapshot().unwrap().len();
                if mode == "measure" {
                    reserve = ToolOutput::MAX_SNAPSHOT_BYTES - length;
                } else {
                    assert_eq!(length, ToolOutput::MAX_SNAPSHOT_BYTES);
                }
            }
        }
    }
}

// 승인 요청의 최신 diff는 승인 게시 전에 반영하지만 요청의 상태·종류로 호출을 끝내거나 바꾸지
// 않는다.
#[test]
fn grok_permission_refreshes_diff_before_approval_without_finishing_call() {
    use yo_core::ActivityUpdate;
    for clear in [false, true] {
        let session_id = session(1);
        let old =
            json!({"type":"diff","path":"settings.rs","oldText":"base\n","newText":"stale\n"});
        let new =
            json!({"type":"diff","path":"settings.rs","oldText":"base\n","newText":"reviewed\n"});
        let (mut backend, _) = backend([
            response(3, json!({"sessionId":"grok-session-a"})),
            session_update(
                "tool_call",
                json!({"toolCallId":"target","kind":"edit","title":"Edit settings","content":[old]}),
            ),
            json!({"jsonrpc":"2.0","id":"permission-a","method":"session/request_permission","params":{
                "sessionId":"grok-session-a","toolCall":{"toolCallId":"target","title":"Review settings","status":"completed","kind":"read","rawInput":{"path":"current.rs"},"content":if clear {json!([])} else {json!([new.clone()])}},
                "options":[{"optionId":"yes","kind":"allow_once","name":"Allow"},{"optionId":"no","kind":"reject_once","name":"Reject"}]
            }}),
        ]);
        create_session(&mut backend, session_id);
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("edit"),
            })
            .unwrap();
        let mut call = None;
        let mut latest = String::new();
        let mut output = None;
        let mut approved = false;
        for _ in 0..30 {
            match backend.poll_event().unwrap() {
                BackendPoll::Event(BackendEvent::ActivityStarted {
                    activity,
                    kind: ActivityKind::FileChange,
                }) => {
                    assert!(call.replace(activity).is_none());
                },
                BackendPoll::Event(BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(text),
                }) => {
                    if Some(activity) == call {
                        latest = text;
                    } else if let Some(value) = ToolOutput::from_snapshot(&text) {
                        output = Some(value);
                    }
                },
                BackendPoll::Event(BackendEvent::ActivityFinished { activity, .. })
                    if Some(activity) == call =>
                {
                    panic!("permission status must not complete call")
                },
                BackendPoll::Event(BackendEvent::ActivityStarted {
                    kind: ActivityKind::ApprovalRequest { .. },
                    ..
                }) => {
                    approved = true;
                    break;
                },
                _ => {},
            }
        }
        assert!(approved);
        assert!(!latest.contains("+stale"));
        assert_eq!(latest.contains("+reviewed"), !clear);
        let output = output.unwrap();
        assert_eq!(output.arguments.unwrap()["path"], "current.rs");
        if clear {
            assert_eq!(output.content_items, Some(json!([])));
        } else {
            assert_eq!(output.content_items.unwrap()[0]["source"], new);
        }
        assert!(!backend.tools["target"].finished);
        assert!(backend.tools["target"].file_change);
    }
}

// 선행 승인 diff는 원문에 남고 실제 호출의 생략 필드만 보충하며 새 값·명시적 빈 값은 덮어쓰지
// 않는다.
#[test]
fn grok_early_permission_retains_diff_without_overwriting_call_content() {
    use yo_core::ActivityUpdate;
    for mode in ["omitted", "newer", "empty"] {
        let session_id = session(1);
        let proposed =
            json!({"type":"diff","path":"a.rs","oldText":"base\n","newText":"proposed\n"});
        let newer = json!({"type":"diff","path":"a.rs","oldText":"base\n","newText":"actual\n"});
        let mut call = json!({"toolCallId":"target","kind":"edit","title":"Edit file","rawInput":{"path":"actual.rs"}});
        if mode == "newer" {
            call["content"] = json!([newer.clone()]);
        }
        if mode == "empty" {
            call["content"] = json!([]);
        }
        let (mut backend, _) = backend([
            response(3, json!({"sessionId":"grok-session-a"})),
            json!({"jsonrpc":"2.0","id":"permission-a","method":"session/request_permission","params":{
                "sessionId":"grok-session-a","toolCall":{"toolCallId":"target","title":"Review file","rawInput":{"path":"proposed.rs"},"content":[proposed.clone()],"locations":[{"path":"a.rs","line":1}]},
                "options":[{"optionId":"yes","kind":"allow_once","name":"Allow"},{"optionId":"no","kind":"reject_once","name":"Reject"}]
            }}),
            session_update(
                "tool_call",
                json!({"toolCallId":"other","kind":"edit","title":"Other file"}),
            ),
            session_update("tool_call", call),
        ]);
        create_session(&mut backend, session_id);
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("edit"),
            })
            .unwrap();
        let mut profile: Option<String> = None;
        let mut output = None;
        let mut linked = false;
        for _ in 0..30 {
            if let BackendPoll::Event(BackendEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(text),
                ..
            }) = backend.poll_event().unwrap()
            {
                if let Some(value) = ActivityApproval::from_snapshot(&text) {
                    if value.related_change.is_some() {
                        assert_eq!(&value.plain_text, profile.as_ref().unwrap());
                        linked = true;
                        break;
                    }
                    assert!(
                        value
                            .plain_text
                            .contains(&format!("{:#}", json!([proposed.clone()])))
                    );
                    profile = Some(value.plain_text);
                } else if let Some(value) = ToolOutput::from_snapshot(&text) {
                    output = Some(value);
                }
            }
        }
        assert!(linked);
        let output = output.unwrap();
        assert_eq!(output.arguments.unwrap()["path"], "actual.rs");
        assert_eq!(output.result.unwrap()["locations"][0]["path"], "a.rs");
        if mode == "empty" {
            assert_eq!(output.content_items, Some(json!([])));
        } else {
            assert_eq!(
                output.content_items.unwrap()[0]["source"],
                if mode == "newer" { newer } else { proposed }
            );
        }
        assert!(
            backend
                .approvals
                .values()
                .all(|binding| binding.pending_display.is_none())
        );
        assert_eq!(backend.tools["other"].output, json!({}));
    }
}

// 텍스트 사이 이미지·리소스·미지 콘텐츠를 버리지 않고 원래 순서의 독립 메시지로 전달한다.
#[test]
fn answer_content_preserves_payload_and_splits_text_stream_in_order() {
    use yo_core::{ActivityUpdate, MessageContent};
    for message_id in [None, Some("answer-a")] {
        for block in [
            json!({"type":"image","mimeType":"image/png","data":"AA==","_meta":{"extra":1}}),
            json!({"type":"resource","resource":{"uri":"file:///remote/main.rs","text":"fn main() {}"}}),
            json!({"type":"future","payload":[1,2,3]}),
        ] {
            let update = |content: Value| {
                let mut fields = json!({"content":content});
                if let Some(id) = message_id {
                    fields["messageId"] = id.into();
                }
                session_update("agent_message_chunk", fields)
            };
            let session_id = session(1);
            let (mut backend, _) = backend([
                response(3, json!({"sessionId":"grok-session-a"})),
                update(json!({"type":"text","text":"before"})),
                update(block.clone()),
                update(json!({"type":"text","text":"after"})),
                response(4, json!({"stopReason":"end_turn"})),
            ]);
            create_session(&mut backend, session_id);
            backend
                .execute_command(AgentCommand::StartTurn {
                    turn: turn(session_id, 1),
                    input: "show content".into(),
                })
                .unwrap();
            let mut opened = None;
            let mut sources = Vec::new();
            let mut completed = false;
            for _ in 0..32 {
                match backend.poll_event().unwrap() {
                    BackendPoll::Event(BackendEvent::ActivityStarted { activity, kind }) => {
                        assert_eq!(kind, ActivityKind::AgentMessage);
                        assert!(opened.replace(activity).is_none());
                    },
                    BackendPoll::Event(BackendEvent::ActivityUpdated { activity, update }) => {
                        assert_eq!(opened, Some(activity));
                        let (ActivityUpdate::TextDelta(text) | ActivityUpdate::TextSnapshot(text)) =
                            update;
                        sources.push(text);
                    },
                    BackendPoll::Event(BackendEvent::ActivityFinished { activity, outcome }) => {
                        assert_eq!(opened.take(), Some(activity));
                        assert_eq!(outcome, ActivityOutcome::Completed);
                    },
                    BackendPoll::Event(BackendEvent::ResumableTurnFinished { .. }) => {
                        completed = true;
                        break;
                    },
                    _ => {},
                }
            }
            assert!(completed);
            assert!(opened.is_none());
            assert_eq!(sources.len(), 3);
            assert_eq!(sources[0], "before");
            assert_eq!(
                MessageContent::from_snapshot(&sources[1]).unwrap().block,
                block
            );
            assert_eq!(sources[2], "after");
        }
    }
}

// 추론 청크는 누적 snapshot을 내보내고 비텍스트 경계 뒤에는 새 Activity로 원문 순서를 보존한다.
#[test]
fn reasoning_stream_preserves_accumulation_and_content_boundaries() {
    use yo_core::{ActivityReasoning, ActivityUpdate};
    for message_id in [None, Some("thought-a")] {
        let block =
            json!({"type":"future", "value":"![literal](data:image/png;base64,AA==)", "extra":7});
        let updates = [
            json!("first "),
            json!("한글"),
            block.clone(),
            json!("after"),
        ]
        .map(|content| {
            let mut update = text_update("agent_thought_chunk", "");
            update["params"]["update"]["content"] = if let Some(text) = content.as_str() {
                json!({"type":"text", "text":text})
            } else {
                content
            };
            if let Some(id) = message_id {
                update["params"]["update"]["messageId"] = json!(id);
            }
            update
        });
        let messages = std::iter::once(response(3, json!({"sessionId":"grok-session-a"})))
            .chain(updates)
            .chain(std::iter::once(response(
                4,
                json!({"stopReason":"end_turn"}),
            )));
        let (mut backend, _) = backend(messages);
        let session_id = session(1);
        create_session(&mut backend, session_id);
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("inspect"),
            })
            .unwrap();
        let mut active = None;
        let mut started = Vec::new();
        let mut snapshots = Vec::new();
        for _ in 0..20 {
            match backend.poll_event().unwrap() {
                BackendPoll::Event(BackendEvent::ActivityStarted { activity, kind }) => {
                    assert_eq!(kind, ActivityKind::ModelWork);
                    assert!(active.replace(activity).is_none());
                    started.push(activity);
                },
                BackendPoll::Event(BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(source),
                }) => {
                    assert_eq!(active, Some(activity));
                    snapshots.push(ActivityReasoning::from_snapshot(&source).unwrap().content);
                },
                BackendPoll::Event(BackendEvent::ActivityFinished { activity, outcome }) => {
                    assert_eq!(active.take(), Some(activity));
                    assert!(matches!(outcome, ActivityOutcome::Completed));
                },
                BackendPoll::Event(BackendEvent::ResumableTurnFinished { .. }) => break,
                other => panic!("unexpected event: {other:?}"),
            }
        }
        assert!(active.is_none());
        assert_eq!(started.len(), 3);
        assert!(started.windows(2).all(|pair| pair[0] != pair[1]));
        assert_eq!(
            snapshots,
            vec![json!("first "), json!("first 한글"), block, json!("after")]
        );
    }
}

// 누적 추론의 크기 초과는 일반 텍스트로 우회하지 않고 이미 전달한 snapshot 다음에 명시적으로
// 실패한다.
#[test]
fn reasoning_stream_rejects_oversized_continuation_without_literal_fallback() {
    let messages = [
        response(3, json!({"sessionId":"grok-session-a"})),
        text_update("agent_thought_chunk", "retained"),
        text_update(
            "agent_thought_chunk",
            &"x".repeat(ToolOutput::MAX_SNAPSHOT_BYTES),
        ),
    ];
    let (mut backend, _) = backend(messages);
    let session_id = session(1);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(session_id, 1),
            input: UserInput::from("inspect"),
        })
        .unwrap();
    let activity = expect_activity_started(&mut backend, ActivityKind::ModelWork);
    expect_activity_update(&mut backend, activity);
    assert!(backend.poll_event().is_err());
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
