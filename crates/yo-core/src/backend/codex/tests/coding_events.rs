use serde_json::json;

use super::support::{activity, backend, session, submission, thread_start_response, turn};
use crate::{
    ActivityKind, ActivityOutcome, AgentCommand, AgentEvent, AgentRuntime, BackendFailureKind,
    RuntimePoll, TurnOutcome, UserInput,
};

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
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityUpdated {
            activity: activity(active_turn, 1),
            update: crate::ActivityUpdate::TextDelta("cargo test".to_owned()),
        })
    );
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityUpdated {
            activity: activity(active_turn, 1),
            update: crate::ActivityUpdate::TextSnapshot("$ cargo test\ncargo test\nok".to_owned()),
        })
    );
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
            update: crate::ActivityUpdate::TextSnapshot("update: src/lib.rs".to_owned()),
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
            outcome: TurnOutcome::Failed(crate::Failure::new("model stream failed")),
        })
    );
}
