use serde_json::json;

use super::support::{activity, backend, session, submission, thread_start_response, turn};
use crate::{
    ActivityKind, ActivityOutcome, AgentCommand, AgentEvent, AgentRuntime, RuntimePoll,
    TurnOutcome, UserInput,
};

// steer와 interrupt는 새 Turn을 만들지 않고 현재 Codex Turn 식별자를 정확히 사용하며,
// steer 응답도 요청한 동일 Turn을 수락했는지 검증하는지 확인한다.
#[test]
fn sends_steer_and_interrupt_to_the_bound_turn() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        thread_start_response(2, "thread-a"),
        json!({ "id": 3, "result": { "turn": { "id": "turn-a" } } }),
        json!({ "id": 4, "result": { "turnId": "turn-a" } }),
        json!({ "id": 5, "result": {} }),
    ];
    let (mut backend, sent) = backend(messages);
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("inspect"),
        })
        .unwrap();

    backend
        .execute_command(AgentCommand::SteerTurn {
            turn: active_turn,
            input: UserInput::from("focus"),
        })
        .unwrap();
    backend
        .execute_command(AgentCommand::InterruptTurn { turn: active_turn })
        .unwrap();

    let sent = sent.0.borrow();
    assert_eq!(sent[4]["method"], "turn/steer");
    assert_eq!(sent[4]["params"]["expectedTurnId"], "turn-a");
    assert_eq!(sent[5]["method"], "turn/interrupt");
    assert_eq!(sent[5]["params"]["turnId"], "turn-a");
}

// app-server가 interrupt 중 Turn을 먼저 닫고 실제 item/completed를 늦게 보내더라도 adapter가
// 해당 Turn의 item을 ID 순서대로 Interrupted 처리하고 후발 알림을 중복 종료로 무시한다.
#[test]
fn interrupted_turn_closes_open_items_before_the_turn() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        thread_start_response(2, "thread-a"),
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
            "method": "item/started",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-a",
                "item": { "id": "item-b", "type": "agentMessage", "status": "inProgress" }
            }
        }),
        json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-a",
                "turn": { "id": "turn-a", "status": "interrupted" }
            }
        }),
        json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-a",
                "item": { "id": "item-a", "type": "commandExecution", "status": "failed" }
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
            submission(8),
        )
        .unwrap();

    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted { activity: observed, .. })
            if observed == activity(active_turn, 1)
    ));
    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted { activity: observed, .. })
            if observed == activity(active_turn, 2)
    ));
    for expected in [activity(active_turn, 1), activity(active_turn, 2)] {
        assert_eq!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::ActivityFinished {
                activity: expected,
                outcome: ActivityOutcome::Interrupted,
            })
        );
    }
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Interrupted,
        })
    );
    assert_eq!(runtime.poll_event().unwrap(), RuntimePoll::Pending);
}

// interrupt 시 일반 item과 승인 대기 Activity를 저장소 경계와 무관하게 ActivityRef 순서로
// 먼저 닫고, 뒤늦은 serverRequest/resolved가 완료 이벤트를 다시 만들지 않는지 확인한다.
#[test]
fn interrupted_turn_sorts_items_and_approvals_before_the_turn() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        thread_start_response(2, "thread-a"),
        json!({ "id": 3, "result": { "turn": { "id": "turn-a" } } }),
        json!({
            "id": "approval-a",
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-a",
                "itemId": "item-a",
                "command": "cargo test"
            }
        }),
        json!({
            "method": "item/started",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-a",
                "item": { "id": "item-b", "type": "commandExecution", "status": "inProgress" }
            }
        }),
        json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-a",
                "turn": { "id": "turn-a", "status": "interrupted" }
            }
        }),
        json!({
            "method": "serverRequest/resolved",
            "params": { "threadId": "thread-a", "requestId": "approval-a" }
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
            submission(9),
        )
        .unwrap();

    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted { activity: observed, .. })
            if observed == activity(active_turn, 1)
    ));
    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityUpdated { activity: observed, .. })
            if observed == activity(active_turn, 1)
    ));
    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted { activity: observed, .. })
            if observed == activity(active_turn, 2)
    ));
    for expected in [activity(active_turn, 1), activity(active_turn, 2)] {
        assert_eq!(
            runtime.poll_event().unwrap(),
            RuntimePoll::Event(AgentEvent::ActivityFinished {
                activity: expected,
                outcome: ActivityOutcome::Interrupted,
            })
        );
    }
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Interrupted,
        })
    );
    assert_eq!(runtime.poll_event().unwrap(), RuntimePoll::Pending);
}

// 이전 interrupted Turn의 후발 완료가 같은 Codex item id를 재사용한 다음 Turn의 binding을
// 제거하거나 완료를 삼키지 않고, 정확한 interrupted Turn 상태로만 분류되는지 확인한다.
#[test]
fn interrupted_turn_state_isolated_from_a_later_item() {
    let session_id = session(1);
    let first_turn = turn(session_id, 1);
    let second_turn = turn(session_id, 2);
    let messages = [
        thread_start_response(2, "thread-a"),
        json!({ "id": 3, "result": { "turn": { "id": "turn-a" } } }),
        json!({
            "method": "item/started",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-a",
                "item": { "id": "item-a", "type": "agentMessage", "status": "inProgress" }
            }
        }),
        json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-a",
                "turn": { "id": "turn-a", "status": "interrupted" }
            }
        }),
        json!({ "id": 4, "result": { "turn": { "id": "turn-b" } } }),
        json!({
            "method": "item/started",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-b",
                "item": { "id": "item-a", "type": "agentMessage", "status": "inProgress" }
            }
        }),
        json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-a",
                "item": { "id": "item-a", "type": "agentMessage", "status": "failed" }
            }
        }),
        json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-b",
                "item": {
                    "id": "item-a",
                    "type": "agentMessage",
                    "status": "completed",
                    "text": "done"
                }
            }
        }),
        json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-a",
                "turn": { "id": "turn-b", "status": "completed" }
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
                turn: first_turn,
                input: UserInput::from("first"),
            },
            submission(6),
        )
        .unwrap();

    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted { .. })
    ));
    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityFinished {
            outcome: ActivityOutcome::Interrupted,
            ..
        })
    ));
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished {
            turn: first_turn,
            outcome: TurnOutcome::Interrupted,
        })
    );

    runtime
        .execute_submission(
            AgentCommand::StartTurn {
                turn: second_turn,
                input: UserInput::from("second"),
            },
            submission(7),
        )
        .unwrap();
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted {
            activity: activity(second_turn, 2),
            kind: ActivityKind::AgentMessage,
        })
    );
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityUpdated {
            activity: activity(second_turn, 2),
            update: crate::ActivityUpdate::TextSnapshot("done".to_owned()),
        })
    );
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityFinished {
            activity: activity(second_turn, 2),
            outcome: ActivityOutcome::Completed,
        })
    );
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished {
            turn: second_turn,
            outcome: TurnOutcome::Completed,
        })
    );
}
