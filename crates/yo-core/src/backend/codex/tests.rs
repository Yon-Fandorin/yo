use std::{cell::RefCell, collections::VecDeque, num::NonZeroU64, rc::Rc, time::Duration};

use serde_json::{Value, json};

use super::{
    Backend,
    client::AppServerClient,
    transport::{JsonPeer, PeerPoll},
};
use crate::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef, ActivityResponse,
    AgentBackend, AgentCommand, AgentEvent, AgentRuntime, ApprovalDecision, BackendFailure,
    BackendFailureKind, RequestId, RuntimePoll, SessionId, TurnId, TurnOutcome, TurnRef, UserInput,
};

#[derive(Clone)]
struct Sent(Rc<RefCell<Vec<Value>>>);

struct FakePeer {
    incoming: VecDeque<Result<PeerPoll, BackendFailure>>,
    sent: Sent,
}

impl FakePeer {
    fn new(incoming: impl IntoIterator<Item = Value>) -> (Self, Sent) {
        let sent = Sent(Rc::new(RefCell::new(Vec::new())));
        (
            Self {
                incoming: incoming
                    .into_iter()
                    .map(|value| Ok(PeerPoll::Message(value)))
                    .collect(),
                sent: sent.clone(),
            },
            sent,
        )
    }
}

impl JsonPeer for FakePeer {
    fn stop_handle(&self) -> crate::BackendStopHandle {
        crate::BackendStopHandle::no_op()
    }

    fn send(&mut self, message: &Value) -> Result<(), BackendFailure> {
        self.sent.0.borrow_mut().push(message.clone());
        Ok(())
    }

    fn receive(&mut self, _timeout: Duration) -> Result<PeerPoll, BackendFailure> {
        self.incoming.pop_front().unwrap_or(Ok(PeerPoll::Closed))
    }

    fn try_receive(&mut self) -> Result<PeerPoll, BackendFailure> {
        self.incoming.pop_front().unwrap_or(Ok(PeerPoll::Pending))
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        Ok(())
    }
}

fn initialize_response(id: u64, version: &str) -> Value {
    json!({
        "id": id,
        "result": {
            "userAgent": format!("codex_cli_rs/{version} (test)"),
            "platformFamily": "unix",
            "platformOs": "linux",
            "codexHome": "/tmp/codex-test"
        }
    })
}

fn id(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

fn session(value: u64) -> SessionId {
    crate::fixture_session(value)
}

fn turn(session_id: SessionId, value: u64) -> TurnRef {
    TurnRef::new(session_id, TurnId::new(id(value)))
}

fn activity(turn: TurnRef, value: u64) -> ActivityRef {
    ActivityRef::new(turn, ActivityId::new(id(value)))
}

fn backend(later_messages: impl IntoIterator<Item = Value>) -> (Backend<FakePeer>, Sent) {
    let messages = [
        vec![initialize_response(1, "0.146.0")],
        later_messages.into_iter().collect(),
    ]
    .concat();
    let (peer, sent) = FakePeer::new(messages);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));
    client.initialize().unwrap();
    let mut backend = Backend::new_uninitialized(client, "/workspace".into());
    backend.initialized = true;
    (backend, sent)
}

// 초기화가 성공하면 initialize 요청 다음에 initialized 알림을 정확한 순서로 보내고,
// 이후 첫 Session RPC가 다음 request id와 설정된 작업 디렉토리를 사용하는지 확인한다.
#[test]
fn initializes_before_starting_a_thread() {
    let (mut backend, sent) = backend([json!({
        "id": 2,
        "result": { "thread": { "id": "thread-a" } }
    })]);

    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: session(1),
        })
        .unwrap();

    let sent = sent.0.borrow();
    assert_eq!(sent[0]["method"], "initialize");
    assert_eq!(sent[1], json!({ "method": "initialized" }));
    assert_eq!(sent[2]["id"], 2);
    assert_eq!(sent[2]["method"], "thread/start");
    assert_eq!(sent[2]["params"]["cwd"], "/workspace");
    assert_eq!(sent[2]["params"]["ephemeral"], true);
}

// 검증하지 않은 Codex minor 버전이면 initialized 알림이나 Session 명령을 보내기 전에
// Initialization 실패로 연결을 중단하고, 새로 검증한 0.146은 이 경로에 들지 않게 한다.
#[test]
fn incompatible_version_fails_during_initialization() {
    let (peer, sent) = FakePeer::new([initialize_response(1, "0.147.0")]);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));

    let failure = match client.initialize() {
        Ok(()) => panic!("an unverified Codex version must not initialize"),
        Err(failure) => failure,
    };

    assert_eq!(failure.kind(), BackendFailureKind::Initialization);
    assert_eq!(sent.0.borrow().len(), 1);
}

// Codex thread/turn/item 식별자가 yo 식별자로 변환되고 streaming tool Activity와
// 완료된 Turn이 같은 의미 순서로 runtime에서 관찰되는지 확인한다.
#[test]
fn maps_a_coding_turn_into_semantic_events() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        json!({ "id": 2, "result": { "thread": { "id": "thread-a" } } }),
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
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("run tests"),
        })
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

// server approval request를 상관관계 Activity로 노출하고 사용자 결정을 원래 JSON-RPC
// id에 응답한 뒤 request와 response Activity를 각각 완료하는지 확인한다.
#[test]
fn correlates_an_approval_round_trip() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        json!({ "id": 2, "result": { "thread": { "id": "thread-a" } } }),
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
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("inspect"),
        })
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
            update: crate::ActivityUpdate::TextSnapshot(
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
        json!({ "id": 2, "result": { "thread": { "id": "thread-a" } } }),
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
        json!({ "id": 2, "result": { "thread": { "id": "thread-a" } } }),
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
        json!({ "id": 2, "result": { "thread": { "id": "thread-a" } } }),
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
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("inspect"),
        })
        .unwrap();

    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Failed(crate::Failure::new("model stream failed")),
        })
    );
}

// steer와 interrupt는 새 Turn을 만들지 않고 현재 Codex Turn 식별자를 정확히 사용하며,
// steer 응답도 요청한 동일 Turn을 수락했는지 검증하는지 확인한다.
#[test]
fn sends_steer_and_interrupt_to_the_bound_turn() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        json!({ "id": 2, "result": { "thread": { "id": "thread-a" } } }),
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
        json!({ "id": 2, "result": { "thread": { "id": "thread-a" } } }),
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
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("inspect"),
        })
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
        json!({ "id": 2, "result": { "thread": { "id": "thread-a" } } }),
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
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("inspect"),
        })
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
        json!({ "id": 2, "result": { "thread": { "id": "thread-a" } } }),
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
        .execute_command(AgentCommand::StartTurn {
            turn: first_turn,
            input: UserInput::from("first"),
        })
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
        .execute_command(AgentCommand::StartTurn {
            turn: second_turn,
            input: UserInput::from("second"),
        })
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

// Codex 실행 파일이 없으면 일반 protocol 오류가 아니라 설치 또는 PATH 문제로 대응할 수
// 있도록 Unavailable 실패를 즉시 반환하는지 확인한다.
#[test]
fn missing_codex_binary_is_reported_as_unavailable() {
    let config =
        super::CodexBackendConfig::new("/tmp").with_executable("/definitely/missing/yo-codex");

    let failure = match super::CodexBackend::spawn(config) {
        Ok(_) => panic!("a missing executable must not initialize"),
        Err(failure) => failure,
    };

    assert_eq!(failure.kind(), BackendFailureKind::Unavailable);
}

// 상대 경로나 존재하지 않는 작업 디렉토리는 child process에 넘겨 모호한 spawn 오류로
// 바꾸지 않고 adapter 설정의 Initialization 실패로 먼저 설명하는지 확인한다.
#[test]
fn invalid_working_directory_is_rejected_before_spawn() {
    let config = super::CodexBackendConfig::new("relative/path");

    let failure = match super::CodexBackend::spawn(config) {
        Ok(_) => panic!("an invalid working directory must not initialize"),
        Err(failure) => failure,
    };

    assert_eq!(failure.kind(), BackendFailureKind::Initialization);
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

// 로컬에 호환되는 Codex가 설치된 환경에서는 실제 stdio app-server와 initialize를
// 완료하고 명시적 shutdown으로 자식 프로세스를 회수할 수 있는지 수동 검증한다.
#[test]
#[ignore = "requires a compatible local Codex installation and writable Codex state"]
fn local_codex_initializes_and_shuts_down() {
    let cwd = std::env::current_dir().unwrap();
    let mut backend = super::CodexBackend::spawn(super::CodexBackendConfig::new(cwd)).unwrap();

    backend.shutdown().unwrap();
}
