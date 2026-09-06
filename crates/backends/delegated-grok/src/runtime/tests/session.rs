use super::*;

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

// Yo outer sandbox smoke는 실제 mount/write attestation 뒤 native sandbox 대신 exact
// no-tools ACP argv로 인증까지만 수행하고 Agent Session이나 inference Turn을 만들지 않습니다.
#[test]
#[ignore = "requires the Yo bwrap profile plus a compatible installed and logged-in Grok CLI"]
fn local_outer_sandbox_grok_authenticates_without_a_session() {
    let cwd = std::env::current_dir().unwrap();
    let config = GrokBackendConfig::new(cwd)
        .with_read_only_review(true)
        .with_outer_sandboxed_review(true);

    GrokBackend::verify(config).unwrap();
}
