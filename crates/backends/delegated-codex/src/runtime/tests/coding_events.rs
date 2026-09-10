use serde_json::json;
use yo_core::{
    ActivityKind, ActivityOutcome, AgentCommand, AgentEvent, AgentRuntime, BackendFailureKind,
    ImageInputCapability, InputImage, InputImageSnapshot, ModelInputPart, RuntimePoll, ToolOutput,
    TurnOutcome, UserInput,
};

use super::support::{activity, backend, session, submission, thread_start_response, turn};

// Codex thread/turn/item 식별자가 yo 식별자로 변환되고 streaming tool Activity와
// 완료된 Turn이 같은 의미 순서로 runtime에서 관찰되는지 확인한다.
#[test]
fn maps_a_coding_turn_into_semantic_events() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        thread_start_response(2, "thread-a"),
        json!({
            "method": "turn/started",
            "params": { "threadId": "thread-a", "turn": { "id": "turn-a" } }
        }),
        json!({ "id": 3, "result": { "turn": { "id": "turn-a" } } }),
        json!({
            "method": "item/started",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-a",
                "item": { "id": "item-a", "type": "commandExecution", "status": "inProgress" }
            }
        }),
        json!({
            "method": "item/commandExecution/outputDelta",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-a",
                "itemId": "item-a",
                "delta": "cargo test"
            }
        }),
        json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-a",
                "item": {
                    "id": "item-a",
                    "type": "commandExecution",
                    "status": "completed",
                    "command": "cargo test",
                    "aggregatedOutput": "cargo test\nok"
                }
            }
        }),
        json!({
            "method": "item/started",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-a",
                "item": { "id": "item-b", "type": "fileChange", "status": "inProgress" }
            }
        }),
        json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-a",
                "item": {
                    "id": "item-b",
                    "type": "fileChange",
                    "status": "completed",
                    "changes": [
                        { "path": "src/lib.rs", "kind": "update", "diff": "@@" }
                    ]
                }
            }
        }),
        json!({
            "method": "thread/tokenUsage/updated",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-a",
                "tokenUsage": {
                    "last": {
                        "inputTokens": 120,
                        "cachedInputTokens": 80,
                        "outputTokens": 30,
                        "reasoningOutputTokens": 12,
                        "totalTokens": 150
                    },
                    "total": {
                        "inputTokens": 300,
                        "cachedInputTokens": 180,
                        "cacheWriteInputTokens": 9,
                        "outputTokens": 70,
                        "reasoningOutputTokens": 22,
                        "totalTokens": 370
                    },
                    "modelContextWindow": 200000
                }
            }
        }),
        json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-a",
                "turn": { "id": "turn-a", "status": "completed" }
            }
        }),
    ];
    let (backend, _) = backend(messages);
    let mut runtime = AgentRuntime::new(backend);
    runtime
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    runtime
        .execute_submission(
            AgentCommand::StartTurn {
                turn: active_turn,
                input: UserInput::from("run tests"),
            },
            submission(1),
        )
        .unwrap();

    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted {
            activity: activity(active_turn, 1),
            kind: ActivityKind::ToolCall,
        })
    );
    for expected in ["cargo test", "$ cargo test\ncargo test\nok"] {
        let RuntimePoll::Event(AgentEvent::ActivityUpdated {
            update: yo_core::ActivityUpdate::TextSnapshot(snapshot),
            ..
        }) = runtime.poll_event().unwrap()
        else {
            panic!("command must publish a structured snapshot");
        };
        let output = ToolOutput::from_snapshot(&snapshot).unwrap();
        assert_eq!(output.tool, "commandExecution");
        assert_eq!(output.plain_text, expected);
    }
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityFinished {
            activity: activity(active_turn, 1),
            outcome: ActivityOutcome::Completed,
        })
    );
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted {
            activity: activity(active_turn, 2),
            kind: ActivityKind::FileChange,
        })
    );
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityUpdated {
            activity: activity(active_turn, 2),
            update: yo_core::ActivityUpdate::TextSnapshot("update: src/lib.rs\n@@".to_owned()),
        })
    );
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityFinished {
            activity: activity(active_turn, 2),
            outcome: ActivityOutcome::Completed,
        })
    );
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted {
            activity: activity(active_turn, 3),
            kind: ActivityKind::ModelWork,
        })
    );
    let RuntimePoll::Event(AgentEvent::ActivityUpdated {
        activity: usage_activity,
        update: yo_core::ActivityUpdate::TextSnapshot(receipt),
    }) = runtime.poll_event().unwrap()
    else {
        panic!("Codex usage must be emitted as one durable text snapshot");
    };
    assert_eq!(usage_activity, activity(active_turn, 3));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&receipt).unwrap(),
        json!({
            "schema": "codex.app-server-token-usage-receipt/v1",
            "source_profile": "codex.app-server.thread-token-usage-updated/v1",
            "turn_id": "turn-a",
            "usage": {
                "input_tokens": 120,
                "output_tokens": 30,
                "total_tokens": 150,
                "reasoning_tokens": 12,
                "cache_read_input_tokens": 80,
                "cache_write_input_tokens": 0
            },
            "thread_total": {
                "input_tokens": 300,
                "output_tokens": 70,
                "total_tokens": 370,
                "reasoning_tokens": 22,
                "cache_read_input_tokens": 180,
                "cache_write_input_tokens": 9
            },
            "model_context_window": 200000
        })
    );
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityFinished {
            activity: activity(active_turn, 3),
            outcome: ActivityOutcome::Completed,
        })
    );
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Completed,
        })
    );
}

// 존재하지 않는 Codex Turn을 가리키는 item event는 현재 Turn에 임의로 붙이지 않고
// Protocol 실패로 반환해 상관관계 손상을 격리하는지 확인한다.
#[test]
fn rejects_an_item_for_an_unknown_turn() {
    let (mut backend, _) = backend([json!({
        "method": "item/started",
        "params": {
            "threadId": "thread-a",
            "turnId": "unknown",
            "item": { "id": "item-a", "type": "agentMessage" }
        }
    })]);

    let failure = backend.poll_event().unwrap_err();

    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
}

// 이미 Turn A에 연결된 item의 delta가 Turn B를 주장하면 itemId만 믿어 A의 Activity에
// 잘못 붙이지 않고 교차 Turn 상관관계 위반으로 거절하는지 확인한다.
#[test]
fn rejects_a_delta_that_changes_the_items_turn() {
    let session_id = session(1);
    let first_turn = turn(session_id, 1);
    let second_turn = turn(session_id, 2);
    let messages = [
        thread_start_response(2, "thread-a"),
        json!({ "id": 3, "result": { "turn": { "id": "turn-a" } } }),
        json!({ "id": 4, "result": { "turn": { "id": "turn-b" } } }),
        json!({
            "method": "item/started",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-a",
                "item": { "id": "item-a", "type": "agentMessage", "text": "" }
            }
        }),
        json!({
            "method": "item/agentMessage/delta",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-b",
                "itemId": "item-a",
                "delta": "wrong turn"
            }
        }),
    ];
    let (mut backend, _) = backend(messages);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: first_turn,
            input: UserInput::from("first"),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: second_turn,
            input: UserInput::from("second"),
        })
        .unwrap();
    backend.poll_event().unwrap();

    let failure = backend.poll_event().unwrap_err();

    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
}

// 지원하는 item 종류의 completed 알림이 start 없이 도착하면 조용히 무시해 증거를
// 잃지 않고 malformed lifecycle을 Protocol 실패로 드러내는지 확인한다.
#[test]
fn rejects_a_supported_item_completion_without_start() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        thread_start_response(2, "thread-a"),
        json!({ "id": 3, "result": { "turn": { "id": "turn-a" } } }),
        json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-a",
                "item": {
                    "id": "item-a",
                    "type": "fileChange",
                    "status": "completed"
                }
            }
        }),
    ];
    let (mut backend, _) = backend(messages);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("inspect"),
        })
        .unwrap();

    let failure = backend.poll_event().unwrap_err();

    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
}

// Codex token usage의 필수 cachedInputTokens가 음수이면 영수증을 생성하지 않고
// Protocol 실패로 닫아 provider가 보고하지 않은 유효값을 추측하지 않습니다.
#[test]
fn rejects_malformed_codex_token_usage() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        thread_start_response(2, "thread-a"),
        json!({ "id": 3, "result": { "turn": { "id": "turn-a" } } }),
        json!({
            "method": "thread/tokenUsage/updated",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-a",
                "tokenUsage": {
                    "last": {
                        "inputTokens": 10,
                        "cachedInputTokens": -1,
                        "outputTokens": 2,
                        "reasoningOutputTokens": 1,
                        "totalTokens": 12
                    },
                    "total": {
                        "inputTokens": 10,
                        "cachedInputTokens": 0,
                        "outputTokens": 2,
                        "reasoningOutputTokens": 1,
                        "totalTokens": 12
                    }
                }
            }
        }),
    ];
    let (mut backend, _) = backend(messages);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("inspect"),
        })
        .unwrap();

    let failure = backend.poll_event().unwrap_err();

    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
    assert!(failure.message().contains("cached input tokens"));
}

// Codex의 error 알림은 곧바로 transport 실패로 중복 보고하지 않고 뒤따르는 failed
// turn/completed의 정확한 실패 메시지로 합쳐지는지 확인한다.
#[test]
fn folds_an_error_notification_into_the_failed_turn() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        thread_start_response(2, "thread-a"),
        json!({ "id": 3, "result": { "turn": { "id": "turn-a" } } }),
        json!({
            "method": "error",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-a",
                "error": { "message": "model stream failed" }
            }
        }),
        json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-a",
                "turn": {
                    "id": "turn-a",
                    "status": "failed",
                    "error": { "message": "less specific" }
                }
            }
        }),
    ];
    let (backend, _) = backend(messages);
    let mut runtime = AgentRuntime::new(backend);
    runtime
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    runtime
        .execute_submission(
            AgentCommand::StartTurn {
                turn: active_turn,
                input: UserInput::from("inspect"),
            },
            submission(4),
        )
        .unwrap();

    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Failed(yo_core::Failure::new("model stream failed")),
        })
    );
}

// 실행 시작부터 명령·디렉터리를 보여주고 delta 뒤의 최종 snapshot으로 출력과 종료 정보를 교체한다.
#[test]
fn publishes_command_before_output_and_retains_exit_details() {
    use yo_core::{ActivityUpdate, BackendEvent, BackendPoll};

    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let (mut backend, _) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id": 3, "result": {"turn": {"id": "turn-a"}}}),
        json!({"method": "item/started", "params": {
            "threadId": "thread-a", "turnId": "turn-a",
            "item": {"id": "shell", "type": "commandExecution", "command": "cargo test", "cwd": "/workspace", "status": "inProgress"}
        }}),
        json!({"method": "item/commandExecution/outputDelta", "params": {
            "threadId": "thread-a", "turnId": "turn-a", "itemId": "shell", "delta": "partial output"
        }}),
        json!({"method": "item/completed", "params": {
            "threadId": "thread-a", "turnId": "turn-a",
            "item": {"id": "shell", "type": "commandExecution", "status": "failed", "aggregatedOutput": "actual failure", "exitCode": 101, "durationMs": 1240}
        }}),
    ]);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("check"),
        })
        .unwrap();
    let mut events = Vec::new();
    while let BackendPoll::Event(event) = backend.poll_event().unwrap() {
        events.push(event);
    }
    assert!(matches!(
        events[0],
        BackendEvent::ActivityStarted {
            kind: ActivityKind::ToolCall,
            ..
        }
    ));
    for (index, expected) in [
        "$ cargo test\nDirectory: /workspace",
        "$ cargo test\nDirectory: /workspace\npartial output",
        "$ cargo test\nDirectory: /workspace\nactual failure\nExit: 101 · Duration: 1240 ms",
    ]
    .into_iter()
    .enumerate()
    {
        let BackendEvent::ActivityUpdated {
            activity: observed,
            update: ActivityUpdate::TextSnapshot(snapshot),
        } = &events[index + 1]
        else {
            panic!("command must remain a structured snapshot");
        };
        assert_eq!(*observed, activity(active_turn, 1));
        let output = ToolOutput::from_snapshot(snapshot).unwrap();
        assert_eq!(output.tool, "commandExecution");
        assert_eq!(
            output.arguments,
            Some(json!({"command": "cargo test", "cwd": "/workspace"}))
        );
        assert_eq!(output.plain_text, expected);
        if index == 2 {
            assert_eq!(
                output.result.unwrap(),
                json!({"content":[{"type":"text","text":"actual failure"}],"exitCode":101,"durationMs":1240,"status":"failed"})
            );
        }
    }
    assert!(matches!(
        events[4],
        BackendEvent::ActivityFinished {
            outcome: ActivityOutcome::Failed(_),
            ..
        }
    ));
}

// 시작 시 제안된 diff와 완료 시 실제 diff를 각각 보존하고 구조화된 rename kind도 잃지 않는다.
#[test]
fn retains_proposed_and_final_file_changes_including_renames() {
    use yo_core::{ActivityUpdate, BackendEvent, BackendPoll};

    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let change = |diff: &str| {
        json!({"id": "edit", "type": "fileChange", "status": "completed", "changes": [
            {"path": "src/old.rs", "kind": {"type": "update", "move_path": "src/new.rs"}, "diff": diff},
            {"path": "empty.txt", "kind": {"type": "add"}, "diff": ""}
        ]})
    };
    let (mut backend, _) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id": 3, "result": {"turn": {"id": "turn-a"}}}),
        json!({"method": "item/started", "params": {"threadId": "thread-a", "turnId": "turn-a", "item": change("@@ -1 +1 @@\n-old\n+proposed\n")}}),
        json!({"method": "item/completed", "params": {"threadId": "thread-a", "turnId": "turn-a", "item": change("@@ -1 +1 @@\n-old\n+actual\n")}}),
    ]);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("edit"),
        })
        .unwrap();
    let mut snapshots = Vec::new();
    while let BackendPoll::Event(event) = backend.poll_event().unwrap() {
        if let BackendEvent::ActivityUpdated {
            update: ActivityUpdate::TextSnapshot(text),
            ..
        } = event
        {
            snapshots.push(text);
        }
    }
    assert_eq!(
        snapshots,
        [
            "update: src/old.rs -> src/new.rs\n@@ -1 +1 @@\n-old\n+proposed\n\nadd: empty.txt",
            "update: src/old.rs -> src/new.rs\n@@ -1 +1 @@\n-old\n+actual\n\nadd: empty.txt",
        ]
    );
}

// MCP 도구는 빈 완료 label 대신 서버·도구·인수·실제 오류를 남긴다.
#[test]
fn retains_mcp_tool_identity_arguments_and_error() {
    use yo_core::{ActivityUpdate, BackendEvent, BackendPoll};

    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let (mut backend, _) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id": 3, "result": {"turn": {"id": "turn-a"}}}),
        json!({"method": "item/started", "params": {"threadId": "thread-a", "turnId": "turn-a", "item": {
            "id": "mcp", "type": "mcpToolCall", "server": "docs", "tool": "search", "arguments": {"query": "render"}, "status": "inProgress"
        }}}),
        json!({"method": "item/completed", "params": {"threadId": "thread-a", "turnId": "turn-a", "item": {
            "id": "mcp", "type": "mcpToolCall", "server": "docs", "tool": "search", "arguments": {"query": "render"}, "status": "failed", "error": {"message": "No index found"}
        }}}),
    ]);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("search"),
        })
        .unwrap();
    let mut snapshots = Vec::new();
    while let BackendPoll::Event(event) = backend.poll_event().unwrap() {
        if let BackendEvent::ActivityUpdated {
            update: ActivityUpdate::TextSnapshot(text),
            ..
        } = event
        {
            snapshots.push(
                ToolOutput::from_snapshot(&text)
                    .expect("typed tool output")
                    .plain_text,
            );
        }
    }
    assert_eq!(snapshots.len(), 2);
    assert!(snapshots[0].starts_with("docs.search\nArguments:\n"));
    assert!(snapshots[0].contains("render"));
    assert!(snapshots[1].contains("No index found"));
}

// 계획 갱신은 같은 Activity를 교체하고 Turn 완료 전에 종료하며 미완료 단계를 임의로 완료하지
// 않는다.
#[test]
fn updates_one_plan_activity_and_closes_it_before_turn_completion() {
    use yo_core::{ActivityPlan, ActivityUpdate, BackendEvent, BackendPoll, PlanStepStatus};
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let plan = |status| json!({"method":"turn/plan/updated","params":{"threadId":"thread-a","turnId":"turn-a","plan":[{"step":"Inspect","status":"completed"},{"step":"Verify","status":status}]}});
    let (mut backend, _) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        plan("pending"),
        plan("inProgress"),
        json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}}),
        plan("completed"),
    ]);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("plan"),
        })
        .unwrap();
    let mut events = Vec::new();
    while let BackendPoll::Event(event) = backend.poll_event().unwrap() {
        events.push(event);
    }
    assert_eq!(events.len(), 5);
    for (index, status) in [
        (1, PlanStepStatus::Pending),
        (2, PlanStepStatus::InProgress),
    ] {
        let BackendEvent::ActivityUpdated {
            activity: owner,
            update: ActivityUpdate::TextSnapshot(text),
        } = &events[index]
        else {
            panic!("missing plan snapshot");
        };
        assert_eq!(*owner, activity(active_turn, 1));
        let plan = ActivityPlan::from_snapshot(text).unwrap();
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].text, "Inspect");
        assert_eq!(plan.steps[0].status, PlanStepStatus::Completed);
        assert_eq!(plan.steps[1].text, "Verify");
        assert_eq!(plan.steps[1].status, status);
    }
    assert!(matches!(events[3], BackendEvent::ActivityFinished { .. }));
    assert!(matches!(
        events[4],
        BackendEvent::ResumableTurnFinished { .. }
    ));
}

// 추가·삭제의 diff 필드는 파일 원문이므로 줄 표식을 붙이고 개행 유무와 diff처럼 보이는
// 원문도 보존한다. 수정의 unified diff는 다시 변환하지 않는다.
#[test]
fn renders_added_and_deleted_content_as_literal_change_lines() {
    use yo_core::{ActivityUpdate, BackendEvent, BackendPoll};

    let item = json!({"id":"patch","type":"fileChange","changes":[
        {"path":"new.rs","kind":{"type":"add"},"diff":"+literal\n\n한글\n"},
        {"path":"old.rs","kind":{"type":"delete"},"diff":"-literal"},
        {"path":"existing.rs","kind":{"type":"update","move_path":null},"diff":"@@ -1 +1 @@\n-old\n+new\n"}
    ]});
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let (mut backend, _) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":item}}),
    ]);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("patch"),
        })
        .unwrap();
    backend.poll_event().unwrap();
    let BackendPoll::Event(BackendEvent::ActivityUpdated {
        update: ActivityUpdate::TextSnapshot(snapshot),
        ..
    }) = backend.poll_event().unwrap()
    else {
        panic!("file snapshot")
    };
    assert_eq!(
        snapshot,
        "add: new.rs\n++literal\n+\n+한글\n\ndelete: old.rs\n--literal\n\\ No newline at end of file\nupdate: existing.rs\n@@ -1 +1 @@\n-old\n+new\n"
    );
}

// 검색·페이지 열기·본문 찾기와 공개 추론 요약은 완료 snapshot으로 보존한다.
// 공개 요약이 없다고 raw reasoning content를 대신 노출하거나 검색 성공을 추측하지 않는다.
#[test]
fn retains_web_actions_and_public_reasoning_summaries() {
    use yo_core::{ActivitySummary, ActivityUpdate, BackendEvent, BackendPoll, SummaryKind};
    for (item, expected, kind) in [
        (
            json!({"type":"webSearch","action":{"type":"search","query":"Rust","queries":["Rust","한글"]}}),
            "Web search\nQuery: Rust\nQuery: 한글",
            ActivityKind::ToolCall,
        ),
        (
            json!({"type":"webSearch","action":{"type":"openPage","url":"https://example.com/docs"}}),
            "Open web page\nURL: https://example.com/docs",
            ActivityKind::ToolCall,
        ),
        (
            json!({"type":"webSearch","action":{"type":"findInPage","url":"https://example.com/docs","pattern":"render"}}),
            "Find in web page\nURL: https://example.com/docs\nFind: render",
            ActivityKind::ToolCall,
        ),
        (
            json!({"type":"webSearch","action":null}),
            "Web search\nDetails not reported",
            ActivityKind::ToolCall,
        ),
        (
            json!({"type":"webSearch","query":"fallback query","action":null}),
            "Web search\nQuery: fallback query",
            ActivityKind::ToolCall,
        ),
        (
            json!({"type":"webSearch","query":"fallback query","action":{"type":"search","query":"","queries":[""]}}),
            "Web search\nQuery: fallback query",
            ActivityKind::ToolCall,
        ),
        (
            json!({"type":"webSearch","query":"fallback query","action":{"type":"openPage","url":null}}),
            "Open web page\nQuery: fallback query",
            ActivityKind::ToolCall,
        ),
        (
            json!({"type":"webSearch","query":"fallback query","action":{"type":"findInPage","url":null,"pattern":"needle"}}),
            "Find in web page\nFind: needle",
            ActivityKind::ToolCall,
        ),
        (
            json!({"type":"webSearch","query":"fallback","action":{"type":"future","extension":17}}),
            "Web search · other action\nQuery: fallback\n{\n  \"extension\": 17,\n  \"type\": \"future\"\n}",
            ActivityKind::ToolCall,
        ),
        (
            json!({"type":"webSearch","query":"fallback","action":{"type":"search","query":17,"queries":["valid",false],"extension":"retained"}}),
            "Web search\nQuery: valid\n{\n  \"extension\": \"retained\",\n  \"queries\": [\n    \"valid\",\n    false\n  ],\n  \"query\": 17\n}",
            ActivityKind::ToolCall,
        ),
        (
            json!({"type":"reasoning","summary":["Inspecting layout","Checking widths"],"content":["NOT PUBLIC"]}),
            "Inspecting layout\n\nChecking widths",
            ActivityKind::ModelWork,
        ),
        (
            json!({"type":"reasoning","summary":[],"content":["NOT PUBLIC"]}),
            "",
            ActivityKind::ModelWork,
        ),
    ] {
        let mut item = item;
        item["id"] = json!("output");
        let session_id = session(1);
        let active_turn = turn(session_id, 1);
        let (mut backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":item.clone()}}),
            json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":item}}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: active_turn,
                input: UserInput::from("inspect"),
            })
            .unwrap();
        assert!(
            matches!(backend.poll_event().unwrap(), BackendPoll::Event(BackendEvent::ActivityStarted { kind: actual, .. }) if actual == kind)
        );
        let mut snapshots = Vec::new();
        while let BackendPoll::Event(event) = backend.poll_event().unwrap() {
            if let BackendEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(text),
                ..
            } = event
            {
                assert!(!text.contains("NOT PUBLIC"));
                snapshots.push(text);
            }
        }
        if kind == ActivityKind::ModelWork {
            let summary = ActivitySummary::from_snapshot(snapshots.last().unwrap()).unwrap();
            assert_eq!(summary.kind, SummaryKind::Reasoning);
            assert_eq!(summary.summary, expected);
            assert!(summary.tokens_before.is_none());
        } else {
            for snapshot in snapshots {
                let output = ToolOutput::from_snapshot(&snapshot).unwrap();
                assert_eq!(output.tool, "webSearch");
                assert_eq!(output.plain_text, expected);
                let arguments = output.arguments.as_ref().unwrap();
                assert_eq!(arguments.get("query"), item.get("query"));
                assert_eq!(arguments.get("action"), item.get("action"));
                assert!(output.result.is_none());
                assert!(output.error.is_none());
            }
        }
    }
}

// 공개 요약의 문단별 delta는 순서대로 재구성되고 완료 snapshot이 최종 내용을 교체한다.
// 원시 추론 delta는 출력하지 않으며 완료 전에도 각 갱신을 소비자가 받는다.
#[test]
fn streams_public_reasoning_parts_before_completion() {
    use yo_core::{ActivitySummary, ActivityUpdate, BackendEvent, BackendPoll, SummaryKind};
    let session_id = session(1);
    let mut messages = vec![
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"reason","type":"reasoning","summary":["Initial "],"content":[]}}}),
        json!({"method":"item/reasoning/textDelta","params":{"threadId":"thread-a","turnId":"turn-a","itemId":"reason","contentIndex":0,"delta":"NOT PUBLIC"}}),
    ];
    for (index, delta) in [(1, "Second"), (0, "첫"), (0, " 문단"), (1, " part")] {
        messages.push(json!({"method":"item/reasoning/summaryTextDelta","params":{
            "threadId":"thread-a","turnId":"turn-a","itemId":"reason","summaryIndex":index,"delta":delta
        }}));
    }
    messages.push(json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"reason","type":"reasoning","summary":["Final public summary"],"content":["NOT PUBLIC"]}}}));
    let (mut backend, _) = backend(messages);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(session_id, 1),
            input: UserInput::from("inspect"),
        })
        .unwrap();
    let BackendPoll::Event(BackendEvent::ActivityStarted {
        activity,
        kind: ActivityKind::ModelWork,
    }) = backend.poll_event().unwrap()
    else {
        panic!("reasoning must start")
    };
    for expected in [
        "Initial ",
        "Initial \n\nSecond",
        "Initial 첫\n\nSecond",
        "Initial 첫 문단\n\nSecond",
        "Initial 첫 문단\n\nSecond part",
        "Final public summary",
    ] {
        let BackendPoll::Event(BackendEvent::ActivityUpdated {
            activity: actual,
            update: ActivityUpdate::TextSnapshot(text),
        }) = backend.poll_event().unwrap()
        else {
            panic!("missing public summary snapshot");
        };
        assert_eq!(actual, activity);
        let summary = ActivitySummary::from_snapshot(&text).unwrap();
        assert_eq!(summary.kind, SummaryKind::Reasoning);
        assert_eq!(summary.summary, expected);
        assert!(summary.tokens_before.is_none());
    }
    assert_eq!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed
        })
    );
}

// 요약 delta도 기존 thread/turn/item 경계를 따르고 잘못된 문단 번호나 다른 item 종류를 거절한다.
#[test]
fn rejects_invalid_public_summary_targets_and_indices() {
    for (item_type, patch) in [
        ("reasoning", json!({"threadId":"other"})),
        ("reasoning", json!({"turnId":"other"})),
        ("reasoning", json!({"itemId":"other"})),
        ("reasoning", json!({"summaryIndex":-1})),
        ("reasoning", json!({"summaryIndex":"0"})),
        ("agentMessage", json!({})),
    ] {
        let session_id = session(1);
        let mut params = json!({"threadId":"thread-a","turnId":"turn-a","itemId":"reason","summaryIndex":0,"delta":"text"});
        params
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let (mut backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"reason","type":item_type}}}),
            json!({"method":"item/reasoning/summaryTextDelta","params":params}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("inspect"),
            })
            .unwrap();
        backend.poll_event().unwrap();
        assert!(backend.poll_event().is_err(), "{item_type}: {patch}");
    }
}

// MCP·동적 도구의 텍스트와 리소스는 실제 줄바꿈으로 읽히며 구조화 결과와 미지의 필드는 유실되지
// 않는다.
#[test]
fn projects_structured_tool_content_without_json_escaped_text() {
    use yo_core::{ActivityUpdate, BackendEvent, BackendPoll};
    for (kind, field, result, expected) in [
        (
            "mcpToolCall",
            "result",
            json!({"content":[
            {"type":"text","text":"첫 줄\nsecond line","annotations":{"audience":["user"]}},
            {"type":"resource","resource":{"uri":"memory://notes","mimeType":"text/plain","text":"alpha\nbeta"}},
            {"type":"resource_link","uri":"https://example.com/report","name":"Report","description":"read this"},
            {"type":"futureBlock","payload":{"keep":"unknown"}},
            {"type":"image","mimeType":"image/png","data":"RETAIN_BYTES"}
        ],"structuredContent":{"count":2},"_meta":{"trace":"retained"}}),
            vec![
                "첫 줄\nsecond line",
                "Resource\nURI: memory://notes\nalpha\nbeta",
                "Resource · Report\nURI: https://example.com/report",
                "text/plain",
                "read this",
                "futureBlock",
                "unknown",
                "encoded bytes",
                "structuredContent:",
                "count",
                "trace",
                "audience",
            ],
        ),
        (
            "dynamicToolCall",
            "contentItems",
            json!([
                {"type":"inputText","text":"dynamic\noutput"},
                {"type":"inputImage","imageUrl":"data:image/png;base64,RETAIN_BYTES"}
            ]),
            vec!["dynamic\noutput", "encoded bytes"],
        ),
        (
            "mcpToolCall",
            "result",
            json!({"content":[],"structuredContent":{"empty":true}}),
            vec!["(empty content)", "structuredContent:", "empty"],
        ),
        (
            "mcpToolCall",
            "result",
            json!({"unexpected":"retain malformed result"}),
            vec!["retain malformed result"],
        ),
    ] {
        let session_id = session(1);
        let mut item =
            json!({"id":"tool","type":kind,"tool":"read","server":"docs","status":"completed"});
        item[field] = result.clone();
        let (mut backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"tool","type":kind,"tool":"read"}}}),
            json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":item}}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("read"),
            })
            .unwrap();
        let mut snapshots = Vec::new();
        let mut outputs = Vec::new();
        while let BackendPoll::Event(event) = backend.poll_event().unwrap() {
            if let BackendEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(text),
                ..
            } = event
            {
                let output = ToolOutput::from_snapshot(&text).expect("typed tool output");
                snapshots.push(output.plain_text.clone());
                outputs.push(output);
            }
        }
        let output = outputs.last().unwrap();
        if field == "result" {
            assert_eq!(output.result.as_ref(), Some(&result));
        } else {
            assert_eq!(output.content_items.as_ref(), Some(&result));
        }
        let text = snapshots.last().unwrap();
        assert!(text.starts_with("docs.read\nResult:\n"), "{text}");
        for expected in expected {
            assert!(text.contains(expected), "missing {expected}: {text}");
        }
        assert!(!text.contains("첫 줄\\nsecond line"), "{text}");
    }
}

// 재시도 가능 오류는 즉시 별도 안내로 전달하고 이후 실제 완료·실패 사유를 오염시키지 않는다.
#[test]
fn retry_notice_does_not_replace_the_terminal_failure() {
    use yo_core::{ActivityNotice, ActivityUpdate, BackendEvent, BackendPoll, NoticeLevel};

    for status in ["completed", "failed"] {
        let session_id = session(1);
        let (mut backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"error","params":{"threadId":"thread-a","turnId":"turn-a","willRetry":true,"error":{"message":"temporary disconnect"}}}),
            json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":status,"error":{"message":"terminal cause"}}}}),
            json!({"method":"error","params":{"threadId":"thread-a","turnId":"turn-a","willRetry":true,"error":{"message":"late retry"}}}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("inspect"),
            })
            .unwrap();
        let BackendPoll::Event(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ModelWork,
        }) = backend.poll_event().unwrap()
        else {
            panic!("retry notice must start")
        };
        let BackendPoll::Event(BackendEvent::ActivityUpdated {
            activity: updated,
            update: ActivityUpdate::TextSnapshot(text),
        }) = backend.poll_event().unwrap()
        else {
            panic!("retry text must arrive before completion")
        };
        assert_eq!(activity, updated);
        let notice = ActivityNotice::from_snapshot(&text).unwrap();
        assert_eq!(notice.title, "Retry announced");
        assert_eq!(notice.level, NoticeLevel::Warning);
        assert!(notice.message.contains("temporary disconnect"));
        assert_eq!(
            backend.poll_event().unwrap(),
            BackendPoll::Event(BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed
            })
        );
        match (status, backend.poll_event().unwrap()) {
            ("completed", BackendPoll::Event(BackendEvent::ResumableTurnFinished { .. })) => {},
            (
                "failed",
                BackendPoll::Event(BackendEvent::TurnFinished {
                    outcome: TurnOutcome::Failed(failure),
                    ..
                }),
            ) => assert_eq!(failure.message(), "terminal cause"),
            other => panic!("unexpected terminal event: {other:?}"),
        }
        assert_eq!(backend.poll_event().unwrap(), BackendPoll::Pending);
    }
}

// status 없는 압축 항목은 알림 종류로 시작·완료를 구분하고 같은 Activity와 실제 Turn을 유지한다.
#[test]
fn context_compaction_reports_progress_and_completion_without_invented_metrics() {
    use yo_core::{ActivityNotice, ActivityUpdate, BackendEvent, BackendPoll, NoticeLevel};

    let session_id = session(1);
    let (mut backend, _) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"compact-a","type":"contextCompaction"}}}),
        json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"compact-a","type":"contextCompaction"}}}),
        json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}}),
    ]);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    let active_turn = turn(session_id, 1);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("continue"),
        })
        .unwrap();
    let BackendPoll::Event(BackendEvent::ActivityStarted {
        activity,
        kind: ActivityKind::ModelWork,
    }) = backend.poll_event().unwrap()
    else {
        panic!("compaction must start a visible activity");
    };
    assert_eq!(activity.turn(), active_turn);
    for title in ["Compacting context", "Context compacted"] {
        let BackendPoll::Event(BackendEvent::ActivityUpdated {
            activity: updated,
            update: ActivityUpdate::TextSnapshot(text),
        }) = backend.poll_event().unwrap()
        else {
            panic!("missing compaction snapshot");
        };
        assert_eq!(activity, updated);
        let notice = ActivityNotice::from_snapshot(&text).unwrap();
        assert_eq!(notice.title, title);
        assert_eq!(notice.level, NoticeLevel::Info);
        assert!(!notice.message.contains("tokens"));
        assert!(!notice.message.contains("summary"));
    }
    assert_eq!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed
        })
    );
    assert!(
        matches!(backend.poll_event().unwrap(), BackendPoll::Event(BackendEvent::ResumableTurnFinished { turn, .. }) if turn == active_turn)
    );
}

// 압축 도중 중단되면 완료 안내를 합성하지 않고 뒤늦은 item 완료도 다시 표시하지 않는다.
#[test]
fn context_compaction_interruption_does_not_claim_completion() {
    use yo_core::{ActivityUpdate, BackendEvent, BackendPoll};

    let session_id = session(1);
    let (mut backend, _) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"compact-a","type":"contextCompaction"}}}),
        json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"interrupted"}}}),
        json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"compact-a","type":"contextCompaction"}}}),
    ]);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(session_id, 1),
            input: UserInput::from("continue"),
        })
        .unwrap();
    let BackendPoll::Event(BackendEvent::ActivityStarted { activity, .. }) =
        backend.poll_event().unwrap()
    else {
        panic!("missing compaction activity");
    };
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityUpdated {
            update: ActivityUpdate::TextSnapshot(_),
            ..
        })
    ));
    assert_eq!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Interrupted
        })
    );
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::TurnFinished {
            outcome: TurnOutcome::Interrupted,
            ..
        })
    ));
    assert_eq!(backend.poll_event().unwrap(), BackendPoll::Pending);
}

// 제안 계획은 시작 원문과 delta를 순서대로 합치고 최종 snapshot으로 같은 활동의 본문을 교체한다.
#[test]
fn proposed_plan_streams_documents_and_replaces_with_final_source() {
    use yo_core::{ActivityDocument, ActivityUpdate, BackendEvent, BackendPoll};
    let session_id = session(1);
    let (mut backend, _) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"plan-a","type":"plan","text":"## Draft\n"}}}),
        json!({"method":"item/plan/delta","params":{"threadId":"thread-a","turnId":"turn-a","itemId":"plan-a","delta":"- Inspect"}}),
        json!({"method":"item/plan/delta","params":{"threadId":"thread-a","turnId":"turn-a","itemId":"plan-a","delta":"\n- Verify"}}),
        json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"plan-a","type":"plan","text":"## Final\n- Review first"}}}),
    ]);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(session_id, 1),
            input: UserInput::from("plan"),
        })
        .unwrap();
    let BackendPoll::Event(BackendEvent::ActivityStarted {
        activity,
        kind: ActivityKind::ModelWork,
    }) = backend.poll_event().unwrap()
    else {
        panic!("missing proposed plan");
    };
    for expected in [
        "## Draft\n",
        "## Draft\n- Inspect",
        "## Draft\n- Inspect\n- Verify",
        "## Final\n- Review first",
    ] {
        let BackendPoll::Event(BackendEvent::ActivityUpdated {
            activity: owner,
            update: ActivityUpdate::TextSnapshot(text),
        }) = backend.poll_event().unwrap()
        else {
            panic!("missing document");
        };
        assert_eq!(owner, activity);
        assert_eq!(
            ActivityDocument::from_snapshot(&text),
            Some(ActivityDocument {
                title: "Proposed plan".to_owned(),
                markdown: expected.to_owned()
            })
        );
    }
    assert_eq!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed
        })
    );
}

// 제안 계획 delta가 다른 종류의 항목을 바꾸지 않으며 원래 thread 경계를 유지한다.
#[test]
fn proposed_plan_rejects_wrong_item_kind_and_thread() {
    for (kind, thread) in [("agentMessage", "thread-a"), ("plan", "other")] {
        let session_id = session(1);
        let (mut backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"plan-a","type":kind,"text":""}}}),
            json!({"method":"item/plan/delta","params":{"threadId":thread,"turnId":"turn-a","itemId":"plan-a","delta":"content"}}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("plan"),
            })
            .unwrap();
        backend.poll_event().unwrap();
        if kind == "plan" {
            backend.poll_event().unwrap();
        }
        assert!(backend.poll_event().is_err());
    }
}

// 백그라운드 터미널 대기·입력은 명령 완료 뒤에도 별도 안내로 남고 stdout이나 명령 실행을 합성하지
// 않는다.
#[test]
fn terminal_interactions_preserve_literal_input_after_command_completion() {
    use yo_core::{ActivityDocument, ActivityUpdate, BackendEvent, BackendPoll};
    let session_id = session(1);
    let input = "```\n![literal](data:image/png;base64,AAAA)\n\u{3}";
    let interaction = |stdin| json!({"method":"item/commandExecution/terminalInteraction","params":{"threadId":"thread-a","turnId":"turn-a","itemId":"command-a","processId":"process-7","stdin":stdin}});
    let (mut backend, _) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"command-a","type":"commandExecution","command":"cargo test","cwd":"/workspace"}}}),
        json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"command-a","type":"commandExecution","command":"cargo test","aggregatedOutput":"actual stdout","status":"completed"}}}),
        interaction(""),
        interaction(input),
        json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}}),
        interaction("late input"),
    ]);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(session_id, 1),
            input: UserInput::from("inspect"),
        })
        .unwrap();
    let mut documents = Vec::new();
    let mut tool_starts = 0;
    let mut output = Vec::new();
    while let BackendPoll::Event(event) = backend.poll_event().unwrap() {
        match event {
            BackendEvent::ActivityStarted {
                kind: ActivityKind::ToolCall,
                ..
            } => tool_starts += 1,
            BackendEvent::ActivityUpdated {
                update: ActivityUpdate::TextSnapshot(text),
                ..
            } => {
                if let Some(document) = ActivityDocument::from_snapshot(&text) {
                    documents.push(document);
                } else {
                    output.push(text);
                }
            },
            _ => {},
        }
    }
    assert_eq!(tool_starts, 1);
    assert_eq!(documents.len(), 2);
    assert_eq!(documents[0].title, "Waited for background terminal");
    assert!(!documents[0].markdown.contains("Input:"));
    assert_eq!(documents[1].title, "Terminal input sent");
    assert!(documents[1].markdown.starts_with("````text\n"));
    assert!(documents[1].markdown.contains(input));
    assert!(documents[1].markdown.contains("Process: process-7"));
    assert!(documents[1].markdown.contains("Command: cargo test"));
    assert!(
        ToolOutput::from_snapshot(output.last().unwrap())
            .unwrap()
            .plain_text
            .ends_with("actual stdout")
    );
    assert!(output.iter().all(|text| !text.contains("Input:")));
    assert!(backend.terminal_commands.is_empty());
}

// 알 수 없는 명령·스레드나 문자열이 아닌 입력은 터미널 안내로 위장하지 못한다.
#[test]
fn terminal_interactions_reject_invalid_targets_and_input() {
    for patch in [
        json!({"itemId":"other"}),
        json!({"threadId":"other"}),
        json!({"stdin":12}),
    ] {
        let session_id = session(1);
        let mut params = json!({"threadId":"thread-a","turnId":"turn-a","itemId":"command-a","processId":"7","stdin":""});
        params
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let (mut backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"command-a","type":"commandExecution","command":"test"}}}),
            json!({"method":"item/commandExecution/terminalInteraction","params":params}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("inspect"),
            })
            .unwrap();
        backend.poll_event().unwrap();
        backend.poll_event().unwrap();
        assert!(backend.poll_event().is_err());
    }
}

// 서버가 보고한 시간만 결과별로 보존하고 계획 종료·시간 안내·턴 종료 순서를 유지한다.
#[test]
fn turn_duration_notice_precedes_terminal_outcome_once() {
    use yo_core::{ActivityNotice, ActivityUpdate, BackendEvent, BackendPoll, NoticeLevel};

    for status in ["completed", "failed", "interrupted"] {
        for duration in [None, Some(json!(null)), Some(json!(0)), Some(json!(62345))] {
            let session_id = session(1);
            let mut completed = json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":status,"error":{"message":"actual failure"}}}});
            if let Some(duration) = &duration {
                completed["params"]["turn"]["durationMs"] = duration.clone();
            }
            let (mut backend, _) = backend([
                thread_start_response(2, "thread-a"),
                json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
                json!({"method":"turn/plan/updated","params":{"threadId":"thread-a","turnId":"turn-a","plan":[{"step":"Verify","status":"inProgress"}]}}),
                completed.clone(),
                completed,
            ]);
            backend
                .execute_command(AgentCommand::CreateSession { session_id })
                .unwrap();
            backend
                .execute_command(AgentCommand::StartTurn {
                    turn: turn(session_id, 1),
                    input: UserInput::from("inspect"),
                })
                .unwrap();
            let mut events = Vec::new();
            while let BackendPoll::Event(event) = backend.poll_event().unwrap() {
                events.push(event);
            }
            let notices = events
                .iter()
                .enumerate()
                .filter_map(|(index, event)| {
                    if let BackendEvent::ActivityUpdated {
                        activity,
                        update: ActivityUpdate::TextSnapshot(text),
                    } = event
                    {
                        ActivityNotice::from_snapshot(text).map(|notice| (index, *activity, notice))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            let expected = duration.as_ref().and_then(|value| value.as_u64());
            assert_eq!(notices.len(), usize::from(expected.is_some()));
            if let Some(ms) = expected {
                let (index, activity, notice) = &notices[0];
                assert_eq!(notice.title, format!("Turn {status}"));
                assert_eq!(
                    notice.level,
                    if status == "failed" {
                        NoticeLevel::Warning
                    } else {
                        NoticeLevel::Info
                    }
                );
                assert!(
                    notice
                        .message
                        .contains(&format!("({ms} ms, reported by Codex)"))
                );
                assert!(
                    notice
                        .message
                        .contains(if ms == 0 { "0.000s" } else { "1m 2.345s" })
                );
                assert!(
                    events[..index - 1]
                        .iter()
                        .any(|event| matches!(event, BackendEvent::ActivityFinished { .. }))
                );
                assert!(
                    matches!(&events[index + 1], BackendEvent::ActivityFinished { activity: finished, outcome: ActivityOutcome::Completed } if finished == activity)
                );
            }
            match (status, events.last().unwrap()) {
                ("completed", BackendEvent::ResumableTurnFinished { .. }) => {},
                (
                    "failed",
                    BackendEvent::TurnFinished {
                        outcome: TurnOutcome::Failed(failure),
                        ..
                    },
                ) => assert_eq!(failure.message(), "actual failure"),
                (
                    "interrupted",
                    BackendEvent::TurnFinished {
                        outcome: TurnOutcome::Interrupted,
                        ..
                    },
                ) => {},
                other => panic!("wrong terminal outcome: {other:?}"),
            }
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(
                        event,
                        BackendEvent::ResumableTurnFinished { .. }
                            | BackendEvent::TurnFinished { .. }
                    ))
                    .count(),
                1
            );
        }
    }
}

// 음수·문자열·소수·int64 첫 초과 시간은 종료 상태를 바꾸기 전에 거절한다.
#[test]
fn turn_duration_rejects_invalid_protocol_values() {
    for duration in [
        json!(-1),
        json!("12"),
        json!(1.5),
        json!(9223372036854775808u64),
    ] {
        let session_id = session(1);
        let (mut backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed","durationMs":duration}}}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("inspect"),
            })
            .unwrap();
        assert_eq!(
            backend.poll_event().unwrap_err().kind(),
            BackendFailureKind::Protocol
        );
        assert!(!backend.wire_turns["turn-a"].finished);
    }
}

// 여러 출력 조각은 누적되고 최종 null 출력은 이미 받은 출력을 지우지 않는다.
#[test]
fn command_stream_accumulates_and_sparse_completion_retains_output() {
    use yo_core::{ActivityUpdate, BackendEvent, BackendPoll};
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let (mut backend, _) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"shell","type":"commandExecution","command":"echo test","cwd":"/workspace","aggregatedOutput":"first"}}}),
        json!({"method":"item/commandExecution/outputDelta","params":{"threadId":"thread-a","turnId":"turn-a","itemId":"shell","delta":"\nsecond"}}),
        json!({"method":"item/commandExecution/outputDelta","params":{"threadId":"thread-a","turnId":"turn-a","itemId":"shell","delta":"\n![literal](file.png)"}}),
        json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"shell","type":"commandExecution","status":"completed","aggregatedOutput":null,"exitCode":0,"durationMs":0}}}),
    ]);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("check"),
        })
        .unwrap();
    let mut outputs = Vec::new();
    while let BackendPoll::Event(event) = backend.poll_event().unwrap() {
        if let BackendEvent::ActivityUpdated {
            update: ActivityUpdate::TextSnapshot(snapshot),
            ..
        } = event
        {
            outputs.push(ToolOutput::from_snapshot(&snapshot).unwrap());
        }
    }
    assert_eq!(outputs.len(), 4);
    for (output, expected) in outputs.iter().zip([
        "first",
        "first\nsecond",
        "first\nsecond\n![literal](file.png)",
        "first\nsecond\n![literal](file.png)",
    ]) {
        assert_eq!(
            output.arguments,
            Some(json!({"command":"echo test","cwd":"/workspace"}))
        );
        assert_eq!(
            output.result.as_ref().unwrap()["content"][0]["text"],
            expected
        );
    }
    assert_eq!(outputs[3].result.as_ref().unwrap()["exitCode"], 0);
    assert_eq!(outputs[3].result.as_ref().unwrap()["durationMs"], 0);
}

// 구조화 표현 한도를 넘는 출력은 마지막 유효 상태와 명령 식별자를 훼손하지 않고 거절한다.
#[test]
fn oversized_command_delta_preserves_last_valid_snapshot() {
    use yo_core::BackendPoll;
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let (mut backend, _) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"shell","type":"commandExecution","command":"echo test","aggregatedOutput":"x"}}}),
        json!({"method":"item/commandExecution/outputDelta","params":{"threadId":"thread-a","turnId":"turn-a","itemId":"shell","delta":"x".repeat(16*1024*1024)}}),
    ]);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("check"),
        })
        .unwrap();
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(_)
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(_)
    ));
    assert!(backend.poll_event().is_err());
    assert_eq!(
        backend.items["shell"].command.as_ref().unwrap()["aggregatedOutput"],
        "x"
    );
    assert_eq!(
        backend.terminal_commands["shell"].1.as_deref(),
        Some("echo test")
    );
}

// 제공자가 보고한 모델 변경은 순서 있는 안내로 보존하며 완료·실패 결과를 바꾸지 않고 늦은 알림은
// 무시한다.
#[test]
fn model_reroute_preserves_reported_details_and_terminal_outcome() {
    use yo_core::{ActivityNotice, ActivityUpdate, BackendEvent, BackendPoll, NoticeLevel};
    for status in ["completed", "failed"] {
        let session_id = session(1);
        let reroute = json!({"method":"model/rerouted","params":{"threadId":"thread-a","turnId":"turn-a","fromModel":"requested","toModel":"actual","reason":"highRiskCyberActivity"}});
        let (mut backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            reroute.clone(),
            json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":status,"error":{"message":"terminal cause"}}}}),
            reroute,
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("inspect"),
            })
            .unwrap();
        let BackendPoll::Event(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ModelWork,
        }) = backend.poll_event().unwrap()
        else {
            panic!("missing reroute start")
        };
        let BackendPoll::Event(BackendEvent::ActivityUpdated {
            activity: updated,
            update: ActivityUpdate::TextSnapshot(text),
        }) = backend.poll_event().unwrap()
        else {
            panic!("missing reroute content")
        };
        assert_eq!(updated, activity);
        let notice = ActivityNotice::from_snapshot(&text).unwrap();
        assert_eq!(notice.title, "Model rerouted");
        assert_eq!(notice.level, NoticeLevel::Warning);
        assert_eq!(
            notice.message,
            "Codex reported a model change.\nFrom: requested\nTo: actual\nReason: highRiskCyberActivity"
        );
        assert_eq!(
            backend.poll_event().unwrap(),
            BackendPoll::Event(BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed
            })
        );
        match (status, backend.poll_event().unwrap()) {
            ("completed", BackendPoll::Event(BackendEvent::ResumableTurnFinished { .. })) => {},
            (
                "failed",
                BackendPoll::Event(BackendEvent::TurnFinished {
                    outcome: TurnOutcome::Failed(failure),
                    ..
                }),
            ) => assert_eq!(failure.message(), "terminal cause"),
            other => panic!("unexpected terminal: {other:?}"),
        }
        assert_eq!(backend.poll_event().unwrap(), BackendPoll::Pending);
    }
}

// 잘못된 대상·필드·크기 알림은 안내 activity를 일부 발행하기 전에 거부한다.
#[test]
fn malformed_model_reroute_is_rejected_before_notice_publication() {
    for (field, value) in [
        ("threadId", json!("other-thread")),
        ("turnId", json!("unknown-turn")),
        ("fromModel", json!(null)),
        ("toModel", json!(42)),
        ("reason", json!(null)),
        ("reason", json!("x".repeat(ToolOutput::MAX_SNAPSHOT_BYTES))),
    ] {
        let session_id = session(1);
        let mut params = json!({"threadId":"thread-a","turnId":"turn-a","fromModel":"requested","toModel":"actual","reason":"highRiskCyberActivity"});
        params[field] = value;
        let (mut backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"model/rerouted","params":params}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("inspect"),
            })
            .unwrap();
        assert_eq!(
            backend.poll_event().unwrap_err().kind(),
            BackendFailureKind::Protocol
        );
        assert!(backend.pending_events.is_empty());
    }
}

// 안내 스냅샷의 정확한 한도는 발행하고 첫 초과 바이트는 activity를 만들기 전에 거부한다.
#[test]
fn model_reroute_notice_accepts_exact_snapshot_limit_only() {
    use yo_core::{ActivityNotice, BackendEvent, BackendPoll, NoticeLevel};
    let overhead = ActivityNotice {
        title: "Model rerouted".to_owned(),
        message: "Codex reported a model change.\nFrom: requested\nTo: actual\nReason: ".to_owned(),
        level: NoticeLevel::Warning,
    }
    .to_snapshot()
    .unwrap()
    .len();
    for extra in [0, 1] {
        let session_id = session(1);
        let (mut backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"model/rerouted","params":{"threadId":"thread-a","turnId":"turn-a","fromModel":"requested","toModel":"actual","reason":"x".repeat(ToolOutput::MAX_SNAPSHOT_BYTES - overhead + extra)}}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("inspect"),
            })
            .unwrap();
        if extra == 0 {
            assert!(matches!(
                backend.poll_event().unwrap(),
                BackendPoll::Event(BackendEvent::ActivityStarted { .. })
            ));
        } else {
            assert_eq!(
                backend.poll_event().unwrap_err().kind(),
                BackendFailureKind::Protocol
            );
            assert!(backend.pending_events.is_empty());
        }
    }
}

// 턴 누적 diff는 같은 activity를 교체하고 빈 diff로 이전 변경을 지우며 모든 종료에서 한 번 닫는다.
#[test]
fn aggregate_turn_diff_replaces_content_and_closes_with_the_turn() {
    use yo_core::{ActivityUpdate, BackendEvent, BackendPoll};
    for status in ["completed", "failed", "interrupted"] {
        let session_id = session(1);
        let update = |diff: &str| json!({"method":"turn/diff/updated","params":{"threadId":"thread-a","turnId":"turn-a","diff":diff}});
        let (mut backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            update("diff --git a/a.rs b/a.rs\n-old\n+new\n"),
            update(""),
            json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":status,"error":{"message":"failed turn"}}}}),
            update("late"),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("edit"),
            })
            .unwrap();
        let BackendPoll::Event(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::FileChange,
        }) = backend.poll_event().unwrap()
        else {
            panic!("missing aggregate start")
        };
        for expected in [
            "Turn aggregate diff\ndiff --git a/a.rs b/a.rs\n-old\n+new\n",
            "Turn aggregate diff\nNo remaining changes reported for this turn.",
        ] {
            assert_eq!(
                backend.poll_event().unwrap(),
                BackendPoll::Event(BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(expected.to_owned())
                })
            );
        }
        let BackendPoll::Event(BackendEvent::ActivityFinished {
            activity: finished,
            outcome,
        }) = backend.poll_event().unwrap()
        else {
            panic!("missing aggregate completion")
        };
        assert_eq!(finished, activity);
        assert!(match status {
            "completed" => outcome == ActivityOutcome::Completed,
            "interrupted" => outcome == ActivityOutcome::Interrupted,
            _ => matches!(outcome, ActivityOutcome::Failed(_)),
        });
        assert!(matches!(
            backend.poll_event().unwrap(),
            BackendPoll::Event(
                BackendEvent::TurnFinished { .. } | BackendEvent::ResumableTurnFinished { .. }
            )
        ));
        assert!(backend.turn_diffs.is_empty());
        assert_eq!(backend.poll_event().unwrap(), BackendPoll::Pending);
    }
}

// 누적 diff의 대상·타입과 정확한 크기 한도를 검증하고 거부 시 부분 activity를 남기지 않는다.
#[test]
fn aggregate_turn_diff_validates_targets_and_exact_size_before_publication() {
    use yo_core::{BackendEvent, BackendPoll};
    let capacity = ToolOutput::MAX_SNAPSHOT_BYTES - "Turn aggregate diff\n".len();
    for (field, value, accepted) in [
        ("threadId", json!("other"), false),
        ("turnId", json!("other"), false),
        ("diff", json!(null), false),
        ("diff", json!("x".repeat(capacity)), true),
        ("diff", json!("x".repeat(capacity + 1)), false),
    ] {
        let session_id = session(1);
        let mut params = json!({"threadId":"thread-a","turnId":"turn-a","diff":""});
        params[field] = value;
        let (mut backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"turn/diff/updated","params":params}),
        ]);
        backend
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        backend
            .execute_command(AgentCommand::StartTurn {
                turn: turn(session_id, 1),
                input: UserInput::from("edit"),
            })
            .unwrap();
        if accepted {
            assert!(matches!(
                backend.poll_event().unwrap(),
                BackendPoll::Event(BackendEvent::ActivityStarted {
                    kind: ActivityKind::FileChange,
                    ..
                })
            ));
        } else {
            assert_eq!(
                backend.poll_event().unwrap_err().kind(),
                BackendFailureKind::Protocol
            );
            assert!(backend.pending_events.is_empty());
            assert!(backend.turn_diffs.is_empty());
        }
    }
}

// 이미지 생성의 시작·완료·실패를 같은 도구 Activity에 연결하고 원본 항목과 PNG를 보존한다.
#[test]
fn image_generation_preserves_lifecycle_payload_and_failure() {
    use yo_core::ActivityUpdate;
    for (status, data) in [
        ("completed", "cG5n"),
        ("completed", ""),
        ("failed", ""),
        ("declined", ""),
    ] {
        let session_id = session(1);
        let active_turn = turn(session_id, 1);
        let start =
            json!({"id":"image-a","type":"imageGeneration","status":"in_progress","result":""});
        let failure = (status == "failed").then(
            || json!({"type":"usageLimitExceeded","limitId":"image_gen","resetsAt":1786150800_u64}),
        );
        let completed = json!({"id":"image-a","type":"imageGeneration","status":status,"result":data,"revisedPrompt":"A blue square","savedPath":"/remote/generated.png","transparentBackground":true,"failure":failure,"future":"retained"});
        let (backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"method":"turn/started","params":{"threadId":"thread-a","turn":{"id":"turn-a"}}}),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":start}}),
            json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":completed}}),
            json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}}),
        ]);
        let mut runtime = AgentRuntime::new(backend);
        runtime
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        runtime
            .execute_submission(
                AgentCommand::StartTurn {
                    turn: active_turn,
                    input: "generate image".into(),
                },
                submission(1),
            )
            .unwrap();
        assert_eq!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::ActivityStarted {
                activity: activity(active_turn, 1),
                kind: ActivityKind::ToolCall,
            })
        );
        for item in [&start, &completed] {
            let RuntimePoll::Event(AgentEvent::ActivityUpdated {
                activity: observed,
                update: ActivityUpdate::TextSnapshot(snapshot),
            }) = runtime.poll_event().unwrap()
            else {
                panic!("image snapshot missing");
            };
            assert_eq!(observed, activity(active_turn, 1));
            let output = ToolOutput::from_snapshot(&snapshot).unwrap();
            assert_eq!(output.tool, "image_generation");
            if item.get("future").is_some() {
                assert!(output.plain_text.contains("retained"));
                assert!(output.plain_text.contains("transparentBackground"));
            }
            let blocks = output.content_blocks().collect::<Vec<_>>();
            assert_eq!(blocks[0]["source"], *item);
            let expected_data = item["result"].as_str().unwrap();
            assert_eq!(blocks.len(), if expected_data.is_empty() { 1 } else { 2 });
            if !expected_data.is_empty() {
                assert_eq!(
                    blocks[1],
                    &json!({"type":"image","mimeType":"image/png","data":expected_data})
                );
            } else {
                assert!(output.plain_text.contains("No image payload received"));
            }
            assert_eq!(
                output.error.as_ref(),
                item.get("failure").filter(|value| !value.is_null())
            );
        }
        let RuntimePoll::Event(AgentEvent::ActivityFinished {
            activity: observed,
            outcome,
        }) = runtime.poll_event().unwrap()
        else {
            panic!("image terminal outcome missing");
        };
        assert_eq!(observed, activity(active_turn, 1));
        match status {
            "completed" => assert_eq!(outcome, ActivityOutcome::Completed),
            "declined" => assert_eq!(outcome, ActivityOutcome::Interrupted),
            "failed" => assert!(matches!(outcome, ActivityOutcome::Failed(_))),
            _ => unreachable!(),
        }
        assert!(matches!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::TurnFinished { .. })
        ));
    }
}

// 이미지 조회는 시작·완료 경로를 보존하되 경로 문자열을 이미지 바이트로 취급하지 않는다.
#[test]
fn image_view_preserves_remote_paths_without_fabricating_image_content() {
    use yo_core::ActivityUpdate;
    for path in [
        "/remote/그림.png",
        "file:///remote/a%20b.png",
        "![image](data:image/png;base64,cG5n)",
    ] {
        let session_id = session(1);
        let active_turn = turn(session_id, 1);
        let item = json!({"id":"view-a","type":"imageView","path":path,"future":"retained"});
        let (backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"method":"turn/started","params":{"threadId":"thread-a","turn":{"id":"turn-a"}}}),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":item}}),
            json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":item}}),
            json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}}),
        ]);
        let mut runtime = AgentRuntime::new(backend);
        runtime
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        runtime
            .execute_submission(
                AgentCommand::StartTurn {
                    turn: active_turn,
                    input: "inspect image".into(),
                },
                submission(1),
            )
            .unwrap();
        assert_eq!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::ActivityStarted {
                activity: activity(active_turn, 1),
                kind: ActivityKind::ToolCall,
            })
        );
        for _ in 0..2 {
            let RuntimePoll::Event(AgentEvent::ActivityUpdated {
                activity: observed,
                update: ActivityUpdate::TextSnapshot(snapshot),
            }) = runtime.poll_event().unwrap()
            else {
                panic!("image view snapshot missing");
            };
            assert_eq!(observed, activity(active_turn, 1));
            let output = ToolOutput::from_snapshot(&snapshot).unwrap();
            assert_eq!(output.tool, "view_image");
            assert_eq!(output.arguments, Some(json!({"path":path})));
            let blocks = output.content_blocks().collect::<Vec<_>>();
            assert_eq!(blocks.len(), 1);
            assert_eq!(blocks[0]["type"], "text");
            assert_eq!(blocks[0]["source"], item);
            assert!(output.plain_text.contains(path));
            assert!(output.plain_text.contains("retained"));
            assert!(output.plain_text.contains("Image bytes are not included"));
        }
        assert_eq!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::ActivityFinished {
                activity: activity(active_turn, 1),
                outcome: ActivityOutcome::Completed,
            })
        );
        assert!(matches!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::TurnFinished { .. })
        ));
    }
}

// 위임 도구 종료와 자식 에이전트 상태를 구분하고 모든 협업 연산의 원본 정보를 보존한다.
#[test]
fn collab_tools_preserve_parent_lifecycle_and_reported_child_states() {
    use yo_core::ActivityUpdate;
    for tool in [
        "spawnAgent",
        "sendInput",
        "resumeAgent",
        "wait",
        "closeAgent",
        "sendMessage",
        "followupTask",
        "interruptAgent",
        "listAgents",
    ] {
        for status in ["completed", "failed", "interrupted"] {
            let session_id = session(1);
            let active_turn = turn(session_id, 1);
            let states = json!({"child-running":{"status":"running","message":null},"child-failed":{"status":"errored","message":"literal **failure**"},"child-done":{"status":"completed","message":"done"}});
            let start = json!({"id":"collab-a","type":"collabAgentToolCall","tool":tool,"status":"inProgress","senderThreadId":"thread-a","receiverThreadIds":[],"agentsStates":{}});
            let completed = json!({"id":"collab-a","type":"collabAgentToolCall","tool":tool,"status":status,"senderThreadId":"thread-a","receiverThreadIds":["child-running","child-failed","child-done"],"agentsStates":states,"model":"reported-model","reasoningEffort":"high","prompt":"Inspect code","future":"retained"});
            let (backend, _) = backend([
                thread_start_response(2, "thread-a"),
                json!({"method":"turn/started","params":{"threadId":"thread-a","turn":{"id":"turn-a"}}}),
                json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
                json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":start}}),
                json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":completed}}),
                json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}}),
            ]);
            let mut runtime = AgentRuntime::new(backend);
            runtime
                .execute_command(AgentCommand::CreateSession { session_id })
                .unwrap();
            runtime
                .execute_submission(
                    AgentCommand::StartTurn {
                        turn: active_turn,
                        input: "inspect agent activity".into(),
                    },
                    submission(1),
                )
                .unwrap();
            assert_eq!(
                runtime.poll_event().unwrap(),
                RuntimePoll::Event(AgentEvent::ActivityStarted {
                    activity: activity(active_turn, 1),
                    kind: ActivityKind::ToolCall,
                })
            );
            for item in [&start, &completed] {
                let RuntimePoll::Event(AgentEvent::ActivityUpdated {
                    activity: observed,
                    update: ActivityUpdate::TextSnapshot(snapshot),
                }) = runtime.poll_event().unwrap()
                else {
                    panic!("agent task snapshot missing");
                };
                assert_eq!(observed, activity(active_turn, 1));
                let output = ToolOutput::from_snapshot(&snapshot).unwrap();
                assert_eq!(output.tool, tool);
                let blocks = output.content_blocks().collect::<Vec<_>>();
                assert_eq!(blocks.len(), 1);
                assert_eq!(blocks[0]["type"], "text");
                assert_eq!(blocks[0]["source"], *item);
                assert!(output.plain_text.contains("Tool status:"));
                assert!(output.plain_text.contains("Reported agent states:"));
                if item.get("future").is_some() {
                    for text in [
                        "Agent child-running · running",
                        "Agent child-failed · errored",
                        "Agent child-done · completed",
                        "literal **failure**",
                        "reported-model",
                        "Inspect code",
                        "retained",
                    ] {
                        assert!(
                            output.plain_text.contains(text),
                            "{tool}: {}",
                            output.plain_text
                        );
                    }
                }
            }
            let RuntimePoll::Event(AgentEvent::ActivityFinished {
                activity: observed,
                outcome,
            }) = runtime.poll_event().unwrap()
            else {
                panic!("agent task terminal outcome missing");
            };
            assert_eq!(observed, activity(active_turn, 1));
            match status {
                "completed" => assert_eq!(outcome, ActivityOutcome::Completed),
                "interrupted" => assert_eq!(outcome, ActivityOutcome::Interrupted),
                "failed" => assert!(matches!(outcome, ActivityOutcome::Failed(_))),
                _ => unreachable!(),
            }
            assert!(matches!(
                runtime.poll_event().unwrap(),
                RuntimePoll::Event(AgentEvent::TurnFinished {
                    outcome: TurnOutcome::Completed,
                    ..
                })
            ));
        }
    }
}

// 검색 결과의 새 형식·필드와 명시적인 빈 배열은 보존하고 생략·null에는 결과를 만들지 않는다.
#[test]
fn web_search_preserves_opaque_results_and_explicit_empty_results() {
    use yo_core::ActivityUpdate;
    for results in [
        None,
        Some(json!(null)),
        Some(json!([])),
        Some(json!([
            {"type":"page","url":"https://example.com/a","title":"Literal **title**","future":{"score":3}},
            {"type":"future-result","payload":[1,"kept"]},
        ])),
        Some(json!({"futureShape":"retained"})),
    ] {
        let session_id = session(1);
        let active_turn = turn(session_id, 1);
        let start = json!({"id":"search-a","type":"webSearch","query":"output elements","action":{"type":"search","query":"output elements"}});
        let mut completed = start.clone();
        if let Some(results) = &results {
            completed["results"] = results.clone();
        }
        let (backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"method":"turn/started","params":{"threadId":"thread-a","turn":{"id":"turn-a"}}}),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":start}}),
            json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":completed}}),
            json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}}),
        ]);
        let mut runtime = AgentRuntime::new(backend);
        runtime
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        runtime
            .execute_submission(
                AgentCommand::StartTurn {
                    turn: active_turn,
                    input: "search".into(),
                },
                submission(1),
            )
            .unwrap();
        assert_eq!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::ActivityStarted {
                activity: activity(active_turn, 1),
                kind: ActivityKind::ToolCall,
            })
        );
        for item in [&start, &completed] {
            let RuntimePoll::Event(AgentEvent::ActivityUpdated {
                activity: observed,
                update: ActivityUpdate::TextSnapshot(snapshot),
            }) = runtime.poll_event().unwrap()
            else {
                panic!("search snapshot missing");
            };
            assert_eq!(observed, activity(active_turn, 1));
            let output = ToolOutput::from_snapshot(&snapshot).unwrap();
            assert_eq!(output.tool, "webSearch");
            assert_eq!(
                output.arguments.as_ref().unwrap()["query"],
                "output elements"
            );
            let expected = item.get("results").filter(|value| !value.is_null());
            assert_eq!(
                output.result,
                expected.map(|value| json!({"results":value}))
            );
            if let Some(expected) = expected {
                assert!(
                    output
                        .plain_text
                        .contains(&format!("Results:\n{expected:#}"))
                );
            } else {
                assert!(!output.plain_text.contains("Results:"));
            }
        }
        assert_eq!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::ActivityFinished {
                activity: activity(active_turn, 1),
                outcome: ActivityOutcome::Completed,
            })
        );
        assert!(matches!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::TurnFinished { .. })
        ));
    }
}

// 리뷰 모드의 두 경계를 독립 문서로 보존하며 빈 종료 본문을 통과 판정으로 바꾸지 않는다.
#[test]
fn review_boundaries_preserve_markdown_and_empty_or_unknown_content() {
    use yo_core::{ActivityDocument, ActivityUpdate};
    for (kind, title) in [
        ("enteredReviewMode", "Review started"),
        ("exitedReviewMode", "Review ended"),
    ] {
        for review in [
            json!(
                "Review **working tree**\n\n[P1] Preserve errors\n\n```rust\nreturn Err(error);\n```"
            ),
            json!(""),
            json!({"future":"retained"}),
        ] {
            let session_id = session(1);
            let active_turn = turn(session_id, 1);
            let item = json!({"id":"review-a","type":kind,"review":review});
            let (backend, _) = backend([
                thread_start_response(2, "thread-a"),
                json!({"method":"turn/started","params":{"threadId":"thread-a","turn":{"id":"turn-a"}}}),
                json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
                json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":item}}),
                json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":item}}),
                json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}}),
            ]);
            let mut runtime = AgentRuntime::new(backend);
            runtime
                .execute_command(AgentCommand::CreateSession { session_id })
                .unwrap();
            runtime
                .execute_submission(
                    AgentCommand::StartTurn {
                        turn: active_turn,
                        input: "review".into(),
                    },
                    submission(1),
                )
                .unwrap();
            assert_eq!(
                runtime.poll_event().unwrap(),
                RuntimePoll::Event(AgentEvent::ActivityStarted {
                    activity: activity(active_turn, 1),
                    kind: ActivityKind::ModelWork
                })
            );
            for _ in 0..2 {
                let RuntimePoll::Event(AgentEvent::ActivityUpdated {
                    activity: observed,
                    update: ActivityUpdate::TextSnapshot(snapshot),
                }) = runtime.poll_event().unwrap()
                else {
                    panic!("review document missing");
                };
                assert_eq!(observed, activity(active_turn, 1));
                if let Some(markdown) = review.as_str() {
                    let document = ActivityDocument::from_snapshot(&snapshot).unwrap();
                    assert_eq!(document.title, title);
                    assert_eq!(document.markdown, markdown);
                } else {
                    assert!(ActivityDocument::from_snapshot(&snapshot).is_none());
                    assert!(snapshot.contains("retained"));
                }
            }
            assert_eq!(
                runtime.poll_event().unwrap(),
                RuntimePoll::Event(AgentEvent::ActivityFinished {
                    activity: activity(active_turn, 1),
                    outcome: ActivityOutcome::Completed
                })
            );
            assert!(matches!(
                runtime.poll_event().unwrap(),
                RuntimePoll::Event(AgentEvent::TurnFinished { .. })
            ));
        }
    }
}

// 에이전트 알림과 대기는 동일 ID의 시작·완료를 한 Activity로 묶고 요청 시간의 정밀도를 지킨다.
#[test]
fn agent_notifications_and_sleep_preserve_identity_and_reported_values() {
    use yo_core::{ActivityNotice, ActivityUpdate, NoticeLevel};
    let mut cases = Vec::new();
    for kind in ["started", "interacted", "interrupted", "completed"] {
        cases.push((json!({"id":"item-a","type":"subAgentActivity","kind":kind,"agentPath":"/root/검토","agentThreadId":"child-a","future":"retained"}), ActivityKind::ModelWork));
    }
    for duration in [json!(0), json!(1500), json!(u64::MAX), json!(-1)] {
        cases.push((
            json!({"id":"item-a","type":"sleep","durationMs":duration,"future":"retained"}),
            ActivityKind::ToolCall,
        ));
    }
    for (item, kind) in cases {
        let session_id = session(1);
        let active_turn = turn(session_id, 1);
        let (backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"method":"turn/started","params":{"threadId":"thread-a","turn":{"id":"turn-a"}}}),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":item}}),
            json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":item}}),
            json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}}),
        ]);
        let mut runtime = AgentRuntime::new(backend);
        runtime
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        runtime
            .execute_submission(
                AgentCommand::StartTurn {
                    turn: active_turn,
                    input: "observe".into(),
                },
                submission(1),
            )
            .unwrap();
        assert_eq!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::ActivityStarted {
                activity: activity(active_turn, 1),
                kind
            })
        );
        let mut previous = None;
        for _ in 0..2 {
            let RuntimePoll::Event(AgentEvent::ActivityUpdated {
                activity: observed,
                update: ActivityUpdate::TextSnapshot(snapshot),
            }) = runtime.poll_event().unwrap()
            else {
                panic!("snapshot missing or duplicate start");
            };
            assert_eq!(observed, activity(active_turn, 1));
            if let Some(previous) = &previous {
                assert_eq!(&snapshot, previous);
            }
            if item["type"] == "subAgentActivity" {
                let notice = ActivityNotice::from_snapshot(&snapshot).unwrap();
                assert_eq!(notice.level, NoticeLevel::Info);
                assert_eq!(
                    notice.title,
                    format!("Agent activity · {}", item["kind"].as_str().unwrap())
                );
                for text in ["/root/검토", "child-a", "retained"] {
                    assert!(notice.message.contains(text));
                }
            } else if let Some(duration) = item["durationMs"].as_u64() {
                let output = ToolOutput::from_snapshot(&snapshot).unwrap();
                assert_eq!(output.tool, "clock.sleep");
                assert_eq!(output.arguments, Some(json!({"durationMs":duration})));
                assert!(
                    output
                        .plain_text
                        .contains(&format!("Requested duration: {duration} ms"))
                );
                assert!(output.plain_text.contains("retained"));
                assert_eq!(output.content_blocks().next().unwrap()["source"], item);
            } else {
                assert!(ToolOutput::from_snapshot(&snapshot).is_none());
                assert!(snapshot.contains("-1") && snapshot.contains("retained"));
            }
            previous = Some(snapshot);
        }
        assert_eq!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::ActivityFinished {
                activity: activity(active_turn, 1),
                outcome: ActivityOutcome::Completed
            })
        );
        assert!(matches!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::TurnFinished {
                outcome: TurnOutcome::Completed,
                ..
            })
        ));
    }
}

// 함수 결과는 실행 호출을 만들지 않고 원본·이름공간과 공통 미디어 변환을 보존한다.
#[test]
fn function_outputs_preserve_original_fields_and_normalized_content() {
    use yo_core::ActivityUpdate;
    let blocks = json!([
        {"type":"input_text","text":"literal **text**","future":1},
        {"type":"input_image","image_url":"data:image/png;base64,AAAA","detail":"original"},
        {"type":"input_audio","audio_url":"https://example.com/audio"},
        {"type":"future","payload":"retained"},
        {"type":"input_image","image_url":"original","imageUrl":"collision"},
    ]);
    for body in [
        json!("literal output"),
        json!(""),
        json!([]),
        blocks.clone(),
        json!({"type":"image","data":"unknown shape"}),
    ] {
        for namespace in [json!(null), json!("functions")] {
            let session_id = session(1);
            let active_turn = turn(session_id, 1);
            let item = json!({"id":"output-a","type":"functionCallOutput","name":"exec","namespace":namespace,"output":body,"future":"kept"});
            let (backend, _) = backend([
                thread_start_response(2, "thread-a"),
                json!({"method":"turn/started","params":{"threadId":"thread-a","turn":{"id":"turn-a"}}}),
                json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
                json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":item}}),
                json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":item}}),
                json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}}),
            ]);
            let mut runtime = AgentRuntime::new(backend);
            runtime
                .execute_command(AgentCommand::CreateSession { session_id })
                .unwrap();
            runtime
                .execute_submission(
                    AgentCommand::StartTurn {
                        turn: active_turn,
                        input: "show output".into(),
                    },
                    submission(1),
                )
                .unwrap();
            assert_eq!(
                runtime.poll_event().unwrap(),
                RuntimePoll::Event(AgentEvent::ActivityStarted {
                    activity: activity(active_turn, 1),
                    kind: ActivityKind::ToolResult
                })
            );
            for _ in 0..2 {
                let RuntimePoll::Event(AgentEvent::ActivityUpdated {
                    activity: observed,
                    update: ActivityUpdate::TextSnapshot(snapshot),
                }) = runtime.poll_event().unwrap()
                else {
                    panic!("result snapshot missing");
                };
                assert_eq!(observed, activity(active_turn, 1));
                let output = ToolOutput::from_snapshot(&snapshot).unwrap();
                assert_eq!(
                    output.tool,
                    if namespace.is_null() {
                        "exec"
                    } else {
                        "functions.exec"
                    }
                );
                assert!(
                    output.server.is_none() && output.arguments.is_none() && output.error.is_none()
                );
                let content = output.content_blocks().collect::<Vec<_>>();
                assert_eq!(content[0]["source"], item);
                assert!(output.plain_text.contains("kept"));
                if body == blocks {
                    assert_eq!(content.len(), 6);
                    assert_eq!(
                        content[1],
                        &json!({"type":"text","text":"literal **text**","future":1})
                    );
                    assert_eq!(
                        content[2],
                        &json!({"type":"inputImage","imageUrl":"data:image/png;base64,AAAA","detail":"original"})
                    );
                    assert_eq!(
                        content[3],
                        &json!({"type":"inputAudio","audioUrl":"https://example.com/audio"})
                    );
                    assert_eq!(content[4], &blocks[3]);
                    assert_eq!(content[5], &blocks[4]);
                } else {
                    assert_eq!(content.len(), 2);
                    assert_eq!(content[1]["type"], "text");
                    let expected = if body == json!("") || body == json!([]) {
                        "(empty output)".to_owned()
                    } else {
                        body.as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| format!("{body:#}"))
                    };
                    assert_eq!(content[1]["text"], expected);
                }
            }
            assert_eq!(
                runtime.poll_event().unwrap(),
                RuntimePoll::Event(AgentEvent::ActivityFinished {
                    activity: activity(active_turn, 1),
                    outcome: ActivityOutcome::Completed
                })
            );
            assert!(matches!(
                runtime.poll_event().unwrap(),
                RuntimePoll::Event(AgentEvent::TurnFinished { .. })
            ));
        }
    }
}

// 훅 문맥을 사용자 제출과 구분하고 여러 fragment의 순서·원문·fence 및 미지 필드를 보존한다.
#[test]
fn hook_context_is_one_literal_document_with_original_fragment_order() {
    use yo_core::{ActivityDocument, ActivityUpdate};
    for fragments in [
        json!([{"hookRunId":"hook-a","text":"First **context**\n````\n![literal](data:image/png;base64,AAAA)","future":"fragment-kept"},{"hookRunId":"hook-b","text":"Second context"}]),
        json!([]),
        json!([{"hookRunId":"hook-a","text":42}]),
    ] {
        let session_id = session(1);
        let active_turn = turn(session_id, 1);
        let item = json!({"id":"hook-item","type":"hookPrompt","fragments":fragments,"future":"item-kept"});
        let (backend, _) = backend([
            thread_start_response(2, "thread-a"),
            json!({"method":"turn/started","params":{"threadId":"thread-a","turn":{"id":"turn-a"}}}),
            json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
            json!({"method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":item}}),
            json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":item}}),
            json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}}),
        ]);
        let mut runtime = AgentRuntime::new(backend);
        runtime
            .execute_command(AgentCommand::CreateSession { session_id })
            .unwrap();
        runtime
            .execute_submission(
                AgentCommand::StartTurn {
                    turn: active_turn,
                    input: "inspect context".into(),
                },
                submission(1),
            )
            .unwrap();
        assert_eq!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::ActivityStarted {
                activity: activity(active_turn, 1),
                kind: ActivityKind::ModelWork
            })
        );
        for _ in 0..2 {
            let RuntimePoll::Event(AgentEvent::ActivityUpdated {
                activity: observed,
                update: ActivityUpdate::TextSnapshot(snapshot),
            }) = runtime.poll_event().unwrap()
            else {
                panic!("hook document missing or input was duplicated");
            };
            assert_eq!(observed, activity(active_turn, 1));
            if fragments[0]
                .get("text")
                .is_some_and(|text| !text.is_string())
            {
                assert!(ActivityDocument::from_snapshot(&snapshot).is_none());
                assert!(snapshot.contains("42") && snapshot.contains("item-kept"));
            } else {
                let doc = ActivityDocument::from_snapshot(&snapshot).unwrap();
                assert_eq!(doc.title, "Hook context");
                assert!(doc.markdown.contains("item-kept"));
                if fragments.as_array().unwrap().is_empty() {
                    assert!(doc.markdown.contains("No hook context fragments reported"));
                } else {
                    assert!(doc.markdown.starts_with("`````text\n"));
                    assert!(doc.markdown.ends_with("\n`````"));
                    for fragment in fragments.as_array().unwrap() {
                        assert!(doc.markdown.contains(fragment["text"].as_str().unwrap()));
                    }
                    assert!(
                        doc.markdown.find("Hook run: hook-a").unwrap()
                            < doc.markdown.find("Hook run: hook-b").unwrap()
                    );
                    assert!(doc.markdown.contains("fragment-kept"));
                }
            }
        }
        assert_eq!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::ActivityFinished {
                activity: activity(active_turn, 1),
                outcome: ActivityOutcome::Completed
            })
        );
        assert!(matches!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::TurnFinished { .. })
        ));
    }
}

// Codex native skill path 재로딩 없이 공통 host가 검증한 전체 지침을 text 입력으로 전달한다.
// 시작과 steer 모두 같은 snapshot을 쓰며 원래 `$name`만 보내지 않는다.
#[test]
fn resolved_skill_snapshot_reaches_codex_start_and_steer_requests() {
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
    let active = turn(session_id, 1);
    let (mut backend, sent) = backend([
        thread_start_response(2, "thread-a"),
        json!({"id":3,"result":{"turn":{"id":"turn-a"}}}),
        json!({"id":4,"result":{"turnId":"turn-a"}}),
    ]);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active,
            input: input.clone(),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::SteerTurn {
            turn: active,
            input,
        })
        .unwrap();
    let sent = sent.0.borrow();
    for method in ["turn/start", "turn/steer"] {
        let request = sent
            .iter()
            .find(|request| request["method"] == method)
            .unwrap();
        assert_eq!(
            request["params"]["input"],
            json!([{"type":"text", "text": expected}])
        );
    }
}

// immutable PNG occurrence와 host-resolved skill trailer가 start와 steer 모두에서 정확한
// text/image 순서와 마지막 지침 위치를 유지하며, 표시 metadata나 별도 caption을 만들지 않습니다.
#[test]
fn immutable_png_projection_preserves_repeated_occurrences_on_start_and_steer() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use yo_core::{InputReference, ResolvedSkill, SkillReference, SkillReferenceScope};

    let png = STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==")
        .unwrap();
    let snapshot = InputImageSnapshot::new(1, 1, png.clone()).unwrap();
    let text = "before [image] middle [image] use $review after";
    let reference = SkillReference::new(
        "review",
        "host",
        "/skills/review/SKILL.md",
        "review",
        SkillReferenceScope::User,
        1,
        "revision",
    );
    let input =
        UserInput::with_references(text, vec![InputReference::skill(34..41, reference.clone())])
            .unwrap()
            .with_images(vec![
                InputImage::new(7..14, 70, snapshot.clone()).unwrap(),
                InputImage::new(22..29, 70, snapshot).unwrap(),
            ])
            .unwrap()
            .with_resolved_skill(
                ResolvedSkill::new(reference, "# Review\nVerify the exact implementation.")
                    .unwrap(),
            )
            .unwrap();
    let expected_url = format!("data:image/png;base64,{}", STANDARD.encode(png));
    let parts = super::super::project_input(&input).unwrap();
    let skill_snapshot = json!({
        "name": "review",
        "source": "/skills/review/SKILL.md",
        "instructions": "# Review\nVerify the exact implementation."
    });
    let expected_tail = format!(
        " use $review after\n\nExplicit skill instructions (yo.skill-instructions/v1):\n{}",
        skill_snapshot
    );
    assert_eq!(
        parts,
        vec![
            json!({"type":"text", "text":"before "}),
            json!({"type":"image", "url": expected_url.clone()}),
            json!({"type":"text", "text":" middle "}),
            json!({"type":"image", "url": expected_url}),
            json!({"type":"text", "text": expected_tail}),
        ]
    );
    assert!(matches!(
        input.model_parts()[1],
        ModelInputPart::Image { .. }
    ));

    let session_id = session(1);
    let active = turn(session_id, 1);
    let (mut backend, sent) = backend([
        thread_start_response(2, "thread-a"),
        json!({
            "id": 3,
            "result": {
                "data": [{"model": "gpt-test", "inputModalities": ["text", "image"]}],
                "nextCursor": null
            }
        }),
        json!({"id":4,"result":{"turn":{"id":"turn-a"}}}),
        json!({"id":5,"result":{"turnId":"turn-a"}}),
    ]);
    backend.image_wire_supported = true;
    backend.selected_model = Some("gpt-test".to_owned());
    backend.image_capability = ImageInputCapability::Supported {
        maximum_occurrences: 16,
        maximum_image_bytes: InputImageSnapshot::MAX_BYTES as u64,
        maximum_input_bytes: InputImageSnapshot::MAX_BYTES as u64,
    };
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active,
            input: input.clone(),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::SteerTurn {
            turn: active,
            input,
        })
        .unwrap();
    let sent = sent.0.borrow();
    for method in ["turn/start", "turn/steer"] {
        let request = sent
            .iter()
            .find(|request| request["method"] == method)
            .unwrap();
        assert_eq!(request["params"]["input"], json!(parts.clone()));
    }
}
