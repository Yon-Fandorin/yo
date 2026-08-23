use std::{cell::RefCell, collections::VecDeque, rc::Rc, time::Duration};

use serde_json::{Value, json};
use yo_backend::transport::JsonMessagePeer;
use yo_core::{
    ActivityKind, ActivityOutcome, ActivityResponse, AgentCommand, ApprovalDecision,
    BackendBindingEvidence, BackendCommandEvidence, BackendEvent, BackendFailure,
    BackendFailureKind, BackendIdentity, BackendPoll, ContinuationStrategy, SessionId, TurnOutcome,
    TurnRef, UserInput,
};

use super::{Backend, GrokBackend, GrokBackendConfig, client::AcpClient, transport::PeerPoll};

#[derive(Clone)]
struct Sent(Rc<RefCell<Vec<Value>>>);

struct FakePeer {
    incoming: VecDeque<Result<PeerPoll, BackendFailure>>,
    sent: Sent,
}

impl FakePeer {
    fn new(messages: impl IntoIterator<Item = Value>) -> (Self, Sent) {
        let sent = Sent(Rc::new(RefCell::new(Vec::new())));
        (
            Self {
                incoming: messages
                    .into_iter()
                    .map(|message| Ok(PeerPoll::Message(message)))
                    .collect(),
                sent: sent.clone(),
            },
            sent,
        )
    }
}

impl JsonMessagePeer for FakePeer {
    fn stop_handle(&self) -> yo_core::BackendStopHandle {
        yo_core::BackendStopHandle::no_op()
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

fn initialize_response(id: u64, auth_methods: &[&str], load_session: bool) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "protocolVersion": 1,
            "agentCapabilities": { "loadSession": load_session },
            "authMethods": auth_methods
                .iter()
                .map(|method| json!({ "id": method, "name": method }))
                .collect::<Vec<_>>(),
            "agentInfo": { "name": "grok", "version": "1.0.5" }
        }
    })
}

fn response(id: u64, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: u64, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

fn session(value: u64) -> SessionId {
    let uuid = uuid::Uuid::from_u128(0x0189_0f00_0000_7000_8000_0000_0000_0000 | u128::from(value));
    SessionId::from_uuid(uuid).expect("the test Session fixture is a UUIDv7")
}

fn turn(session_id: SessionId, value: u64) -> TurnRef {
    TurnRef::new(
        session_id,
        yo_core::TurnId::new(std::num::NonZeroU64::new(value).unwrap()),
    )
}

fn backend(later_messages: impl IntoIterator<Item = Value>) -> (Backend<FakePeer>, Sent) {
    let messages = [
        vec![
            initialize_response(1, &["cached_token", "grok.com"], true),
            response(2, json!({})),
        ],
        later_messages.into_iter().collect(),
    ]
    .concat();
    let (peer, sent) = FakePeer::new(messages);
    (
        Backend::new_uninitialized(
            AcpClient::new(peer, Duration::from_secs(1)),
            "/workspace".to_owned(),
        ),
        sent,
    )
}

fn create_session(backend: &mut Backend<FakePeer>, session_id: SessionId) {
    backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();
}

fn resume_binding(grok_session: &str) -> BackendBindingEvidence {
    BackendBindingEvidence::new(
        "grok-build-acp",
        "grok/recorded",
        BackendIdentity::new(
            "grok.acp/session-binding/v1",
            json!({ "sessionId": grok_session }).to_string(),
        ),
        BackendIdentity::new("grok.build/model-selection/v1", "backend-managed"),
        BackendIdentity::new("grok.acp/session-locator/v1", grok_session),
        ContinuationStrategy::BackendManagedState,
    )
}

// 새 Session은 ACP v1 초기화 뒤 cached_token만 인증하고 session/new를 호출하며,
// 반환된 Grok Session ID를 backend-managed Continuation 신원으로 보존합니다.
#[test]
fn initializes_with_cached_login_and_opens_a_durable_session() {
    let session_id = session(1);
    let (mut backend, sent) = backend([response(3, json!({ "sessionId": "grok-session-a" }))]);

    let evidence = backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();

    let BackendCommandEvidence::BindingOpened(evidence) = evidence else {
        panic!("session/new must return binding evidence");
    };
    assert_eq!(evidence.backend_kind(), "grok-build-acp");
    assert_eq!(evidence.backend_version(), "grok/1.0.5");
    assert_eq!(evidence.session_locator().value(), "grok-session-a");
    assert_eq!(
        evidence.continuation_strategy(),
        ContinuationStrategy::BackendManagedState
    );

    let sent = sent.0.borrow();
    assert_eq!(sent[0]["method"], "initialize");
    assert_eq!(sent[0]["params"]["clientCapabilities"], json!({}));
    assert_eq!(sent[1]["method"], "authenticate");
    assert_eq!(sent[1]["params"]["methodId"], "cached_token");
    assert_eq!(sent[2]["method"], "session/new");
    assert_eq!(sent[2]["params"]["cwd"], "/workspace");
}

// Grok이 cached_token을 광고하지 않으면 API key나 브라우저 flow로 자동 전환하지 않고,
// 별도 과금 가능성을 피하기 위해 grok login 안내가 있는 Initialization 실패로 닫습니다.
#[test]
fn refuses_to_fall_back_when_cached_login_is_unavailable() {
    for methods in [&[][..], &["grok.com"][..]] {
        let (peer, sent) = FakePeer::new([initialize_response(1, methods, true)]);
        let mut backend = Backend::new_uninitialized(
            AcpClient::new(peer, Duration::from_secs(1)),
            "/workspace".to_owned(),
        );

        let failure = backend
            .execute_command(AgentCommand::CreateSession {
                session_id: session(1),
            })
            .unwrap_err();

        assert_eq!(failure.kind(), BackendFailureKind::Initialization);
        assert!(failure.message().contains("grok login"));
        assert_eq!(sent.0.borrow().len(), 1);
    }
}

// cached_token이 광고됐지만 저장 login이 거절되면 원래 인증 실패를 보존하면서도
// 다른 credential flow로 전환하지 않고 grok login 재실행 경로를 명시합니다.
#[test]
fn rejected_cached_login_has_actionable_login_guidance() {
    let (peer, sent) = FakePeer::new([
        initialize_response(1, &["cached_token", "grok.com"], true),
        error_response(2, -32000, "token expired"),
    ]);
    let mut backend = Backend::new_uninitialized(
        AcpClient::new(peer, Duration::from_secs(1)),
        "/workspace".to_owned(),
    );

    let failure = backend
        .execute_command(AgentCommand::CreateSession {
            session_id: session(1),
        })
        .unwrap_err();

    assert_eq!(failure.kind(), BackendFailureKind::Initialization);
    assert!(failure.message().contains("run `grok login`"));
    assert!(failure.message().contains("token expired"));
    assert_eq!(sent.0.borrow().len(), 2);
}

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

// permission request는 allow_once와 reject_once를 우선하는 이진 yo 승인으로 변환되고,
// 승인 응답은 ACP selected outcome에 원래 optionId를 넣어 동일 request id에 돌려줍니다.
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
                "toolCall": { "toolCallId": "tool-a", "title": "Run cargo test" },
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

// 한 Session에서 완료된 ACP ToolCallId는 다음 Turn에서도 correlation tombstone으로
// 남아야 하며, 재사용을 새 Activity로 잘못 열어서는 안 됩니다.
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

    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityStarted { .. })
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished { .. })
    ));
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
#[test]
fn bounds_active_activity_state_before_allocating_another_message() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let mut messages = vec![response(3, json!({ "sessionId": "grok-session-a" }))];
    messages.extend(
        (0..=Backend::<FakePeer>::MAX_ACTIVE_ACTIVITIES).map(|index| {
            json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": "grok-session-a",
                    "update": {
                        "sessionUpdate": "agent_message_chunk",
                        "messageId": format!("message-{index}"),
                        "content": { "type": "text", "text": "x" }
                    }
                }
            })
        }),
    );
    let (mut backend, _) = backend(messages);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("stream"),
        })
        .unwrap();

    for _ in 0..Backend::<FakePeer>::MAX_ACTIVE_ACTIVITIES {
        assert!(matches!(
            backend.poll_event().unwrap(),
            BackendPoll::Event(BackendEvent::ActivityStarted { .. })
        ));
        assert!(matches!(
            backend.poll_event().unwrap(),
            BackendPoll::Event(BackendEvent::ActivityUpdated { .. })
        ));
    }
    let failure = backend.poll_event().unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
    assert!(failure.message().contains("active activity limit"));
    assert_eq!(
        backend.messages.len(),
        Backend::<FakePeer>::MAX_ACTIVE_ACTIVITIES
    );
}

// 완료 tombstone 집합 자체도 Session 수명 동안 유한해야 하며, 상한 뒤의 새 ToolCallId는
// Activity나 추가 문자열을 보존하기 전에 거절합니다.
#[test]
fn bounds_the_session_tool_call_tombstone_set() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let tool = json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": "grok-session-a",
            "update": {
                "sessionUpdate": "tool_call",
                "toolCallId": "overflow-tool"
            }
        }
    });
    let (mut backend, _) = backend([response(3, json!({ "sessionId": "grok-session-a" })), tool]);
    create_session(&mut backend, session_id);
    backend.seen_tool_ids.extend(
        (0..Backend::<FakePeer>::MAX_SESSION_TOOL_IDS).map(|index| format!("tool-{index}")),
    );
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("tool"),
        })
        .unwrap();

    let failure = backend.poll_event().unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
    assert!(failure.message().contains("ToolCallId limit"));
    assert!(!backend.seen_tool_ids.contains("overflow-tool"));
}

// interrupt는 active prompt의 Session ID로 session/cancel 알림을 보내고, Grok의
// cancelled 응답 뒤 열린 Activity를 먼저 Interrupted 처리한 후 Turn을 닫습니다.
#[test]
fn cancels_the_active_session_and_waits_for_cancelled_completion() {
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
                    "sessionUpdate": "agent_thought_chunk",
                    "content": { "type": "text", "text": "thinking" }
                }
            }
        }),
        response(4, json!({ "stopReason": "cancelled" })),
    ];
    let (mut backend, sent) = backend(messages);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("inspect"),
        })
        .unwrap();
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityStarted { .. })
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityUpdated { .. })
    ));

    backend
        .execute_command(AgentCommand::InterruptTurn { turn: active_turn })
        .unwrap();
    let cancel = sent.0.borrow().last().cloned().unwrap();
    assert_eq!(cancel["method"], "session/cancel");
    assert_eq!(cancel["params"]["sessionId"], "grok-session-a");
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished {
            outcome: ActivityOutcome::Interrupted,
            ..
        })
    ));
    assert_eq!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Interrupted,
        })
    );
}

// session/load가 과거 transcript update를 응답 전에 다시 보내도 그것을 새 Turn으로
// 투영하지 않고 버린 뒤, durable locator와 같은 Session 신원만 재개합니다.
#[test]
fn resumes_with_session_load_without_replaying_history_as_new_activity() {
    let session_id = session(1);
    let history = json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": "grok-session-a",
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "old" }
            }
        }
    });
    let (mut backend, sent) = backend([history, response(3, json!({}))]);

    let evidence = backend
        .resume_binding(session_id, &resume_binding("grok-session-a"))
        .unwrap();

    assert_eq!(evidence.session_locator().value(), "grok-session-a");
    assert_eq!(sent.0.borrow()[2]["method"], "session/load");
    assert_eq!(backend.poll_event().unwrap(), BackendPoll::Pending);
}

// 설치되지 않은 실행 파일은 protocol이나 Session 오류로 오인하지 않고 시작 경계의
// Unavailable 실패로 분류해 사용자가 Grok CLI 설치 문제를 바로 구분할 수 있게 합니다.
#[test]
fn classifies_a_missing_grok_executable_as_unavailable() {
    let config = GrokBackendConfig::new(std::env::temp_dir())
        .with_executable("yo-definitely-missing-grok-executable");

    let failure = match GrokBackend::spawn(config) {
        Ok(_) => panic!("missing Grok executable must not spawn"),
        Err(failure) => failure,
    };

    assert_eq!(failure.kind(), BackendFailureKind::Unavailable);
}

// 로컬에 호환되는 Grok CLI와 cached login이 있으면 실제 ACP v1 초기화·인증을 수행한 뒤
// Session을 만들지 않고 process를 정상 종료하는 smoke 경계를 확인합니다.
#[test]
#[ignore = "requires a compatible installed and logged-in Grok CLI"]
fn local_grok_authenticates_and_shuts_down_without_a_session() {
    let cwd = std::env::current_dir().unwrap();

    GrokBackend::verify(GrokBackendConfig::new(cwd)).unwrap();
}
