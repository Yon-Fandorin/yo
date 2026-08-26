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
    backend_with_profile(later_messages, false)
}

fn backend_with_profile(
    later_messages: impl IntoIterator<Item = Value>,
    read_only_review: bool,
) -> (Backend<FakePeer>, Sent) {
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
            read_only_review,
        ),
        sent,
    )
}

fn session_update(kind: &str, mut update: Value) -> Value {
    update["sessionUpdate"] = Value::String(kind.to_owned());
    json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": { "sessionId": "grok-session-a", "update": update }
    })
}

fn text_update(kind: &str, text: &str) -> Value {
    session_update(kind, json!({ "content": { "type": "text", "text": text } }))
}

fn tool_call(tool_id: &str, status: &str, title: Option<&str>) -> Value {
    let mut update = json!({ "toolCallId": tool_id, "status": status });
    if let Some(title) = title {
        update["title"] = Value::String(title.to_owned());
    }
    session_update("tool_call", update)
}

fn tool_result(tool_id: &str, text: &str) -> Value {
    session_update(
        "tool_call_update",
        json!({
            "toolCallId": tool_id,
            "name": "terminal",
            "content": [{
                "type": "content",
                "content": { "type": "text", "text": text }
            }],
            "status": "completed"
        }),
    )
}

fn tool_raw_input_update(tool_id: &str, raw_input: Value) -> Value {
    session_update(
        "tool_call_update",
        json!({
            "toolCallId": tool_id,
            "rawInput": raw_input,
            "status": "in_progress"
        }),
    )
}

fn permission_request(id: &str, title: Option<&str>) -> Value {
    let mut params = json!({
        "sessionId": "grok-session-a",
        "options": [
            { "optionId": "once", "kind": "allow_once" },
            { "optionId": "reject", "kind": "reject_once" }
        ]
    });
    if let Some(title) = title {
        params["toolCall"] = json!({ "title": title });
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "session/request_permission",
        "params": params
    })
}

fn expect_activity_started(
    backend: &mut Backend<FakePeer>,
    expected: ActivityKind,
) -> yo_core::ActivityRef {
    match backend.poll_event().unwrap() {
        BackendPoll::Event(BackendEvent::ActivityStarted { activity, kind }) => {
            assert_eq!(kind, expected);
            activity
        },
        other => panic!("expected {expected:?} ActivityStarted, got {other:?}"),
    }
}

fn expect_activity_update(backend: &mut Backend<FakePeer>, activity: yo_core::ActivityRef) {
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityUpdated { activity: observed, .. })
            if observed == activity
    ));
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

fn read_only_resume_binding(grok_session: &str) -> BackendBindingEvidence {
    BackendBindingEvidence::new(
        "grok-build-acp",
        "grok/recorded",
        BackendIdentity::new(
            "grok.acp/session-binding/v1alpha1",
            json!({
                "executionProfile": "yo.delegated-review-execution/v1alpha1",
                "sessionId": grok_session,
            })
            .to_string(),
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

// 읽기 전용 Grok Session은 일반 ACP payload를 바꾸지 않되 durable binding에 제한
// execution profile을 기록해 이후 resume이 같은 process 정책을 복원할 수 있게 합니다.
#[test]
fn read_only_review_records_a_distinct_durable_binding() {
    let (mut backend, sent) = backend_with_profile(
        [response(3, json!({ "sessionId": "grok-session-a" }))],
        true,
    );

    let evidence = backend
        .execute_command(AgentCommand::CreateSession {
            session_id: session(1),
        })
        .unwrap();
    let BackendCommandEvidence::BindingOpened(binding) = evidence else {
        panic!("session/new must return the restricted binding");
    };
    assert_eq!(
        binding.binding_identity().schema(),
        "grok.acp/session-binding/v1alpha1"
    );
    assert!(
        binding
            .binding_identity()
            .value()
            .contains("yo.delegated-review-execution/v1alpha1")
    );
    assert_eq!(sent.0.borrow()[2]["method"], "session/new");
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
            false,
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
        false,
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
            "schema": "grok.acp-prompt-usage-receipt/v1",
            "source_profile": "grok.acp.prompt-response.meta-usage/v1",
            "prompt_request_id": 4,
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
        }) if observed == activity && summary == "terminal: {\"command\":\"cargo test\"}"
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
    assert_eq!(
        serde_json::from_str::<Value>(&result).unwrap(),
        json!({ "call_id": "tool-a", "output": "all tests passed" })
    );
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
            && serde_json::from_str::<Value>(&result).unwrap()
                == json!({ "call_id": "tool-a", "output": "" })
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

// 제한 binding은 같은 read-only backend에서만 session/load로 이어지고, 일반 backend가
// 받으면 process 초기화 전 거절되어 resume이 권한을 넓히지 않습니다.
#[test]
fn read_only_resume_restores_profile_and_rejects_downgrade() {
    let binding = read_only_resume_binding("grok-session-a");
    let (mut restricted, restricted_sent) = backend_with_profile([response(3, json!({}))], true);
    let evidence = restricted.resume_binding(session(1), &binding).unwrap();
    assert_eq!(
        evidence.binding_identity().schema(),
        "grok.acp/session-binding/v1alpha1"
    );
    assert_eq!(restricted_sent.0.borrow()[2]["method"], "session/load");

    let (mut standard, standard_sent) = backend([]);
    let failure = standard.resume_binding(session(1), &binding).unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::Session);
    assert!(standard_sent.0.borrow().is_empty());
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
