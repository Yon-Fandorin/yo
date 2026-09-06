use serde_json::json;
use yo_core::{
    ActivityKind, ActivityOutcome, ActivityRequestRef, ActivityResponse, AgentCommand, AgentEvent,
    AgentRuntime, ApprovalDecision, BackendFailureKind, RequestId, RuntimePoll, UserInput,
};

use super::support::{activity, backend, id, session, submission, thread_start_response, turn};

// server approval request를 상관관계 Activity로 노출하고 사용자 결정을 원래 JSON-RPC
// id에 응답한 뒤 request와 response Activity를 각각 완료하는지 확인한다.
#[test]
fn correlates_an_approval_round_trip() {
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
                "command": "cargo test",
                "reason": "requires workspace execution"
            }
        }),
        json!({
            "method": "serverRequest/resolved",
            "params": { "threadId": "thread-a", "requestId": "approval-a" }
        }),
    ];
    let (backend, sent) = backend(messages);
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
            submission(2),
        )
        .unwrap();
    let request = ActivityRequestRef::new(activity(active_turn, 1), RequestId::new(id(1)));

    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted {
            activity: request.activity(),
            kind: ActivityKind::ApprovalRequest {
                request_id: request.request_id(),
            },
        })
    );
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityUpdated {
            activity: request.activity(),
            update: yo_core::ActivityUpdate::TextSnapshot(
                "$ cargo test\nReason: requires workspace execution".to_owned()
            ),
        })
    );
    runtime
        .execute_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::Approval(ApprovalDecision::Approved),
        })
        .unwrap();

    assert_eq!(
        sent.0.borrow().last().unwrap(),
        &json!({ "id": "approval-a", "result": { "decision": "accept" } })
    );
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted {
            activity: activity(active_turn, 2),
            kind: ActivityKind::ApprovalResponse {
                request_id: request.request_id(),
            },
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
        RuntimePoll::Event(AgentEvent::ActivityFinished {
            activity: request.activity(),
            outcome: ActivityOutcome::Completed,
        })
    );
}

// 지원하지 않는 server request를 받으면 조용히 버려 Codex를 영원히 기다리게 하지 않고,
// 같은 JSON-RPC id에 Method not found 오류를 답한 뒤 Unsupported로 보고하는지 확인한다.
#[test]
fn unsupported_server_request_is_rejected_on_the_wire() {
    let (mut backend, sent) = backend([json!({
        "id": "request-a",
        "method": "item/permissions/requestApproval",
        "params": {}
    })]);

    let failure = backend.poll_event().unwrap_err();

    assert_eq!(failure.kind(), BackendFailureKind::Unsupported);
    assert_eq!(
        sent.0.borrow().last().unwrap(),
        &json!({
            "id": "request-a",
            "error": {
                "code": -32601,
                "message": "server request is unsupported by yo"
            }
        })
    );
}
