use super::*;

// skills watcher ACK는 session/new의 실제 숫자 response를 대신하지 않고, 0건 reload도
// 성공한 Session binding을 그대로 보존합니다.
#[test]
fn consumes_skills_reload_ack_while_waiting_for_new_session() {
    let session_id = session(1);
    let (mut backend, sent) = backend([
        skills_reload_ack(0),
        response(3, json!({ "sessionId": "grok-session-a" })),
    ]);

    let evidence = backend
        .execute_command(AgentCommand::CreateSession { session_id })
        .unwrap();

    assert!(matches!(
        evidence,
        BackendCommandEvidence::BindingOpened(evidence)
            if evidence.session_locator().value() == "grok-session-a"
    ));
    assert_eq!(sent.0.borrow()[2]["method"], "session/new");
}

// prompt acceptance 전에 maintenance ACK만 오면 active Turn을 만들지 않고, 실제 수락
// signal이 없었던 종료를 그대로 호출자에게 전달합니다.
#[test]
fn skills_reload_ack_is_not_prompt_acceptance() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let (mut backend, _) = backend([
        response(3, json!({ "sessionId": "grok-session-a" })),
        skills_reload_ack(0),
    ]);
    create_session(&mut backend, session_id);

    let failure = backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("marker"),
        })
        .unwrap_err();

    assert_eq!(failure.kind(), BackendFailureKind::ProcessExit);
    assert!(failure.message().contains("session/prompt acceptance"));
    assert!(backend.prompt.is_none());
}

// 수락된 prompt 중 maintenance ACK는 Pending으로만 소진하고, 뒤따른 숫자 ID 4의
// stopReason 완료를 한 번만 runtime Turn 종료로 투영하며 marker text를 보존합니다.
#[test]
fn consumes_skills_reload_ack_during_prompt_without_finishing_early() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let messages = [
        response(3, json!({ "sessionId": "grok-session-a" })),
        skills_reload_ack(0),
        text_update("agent_message_chunk", "marker"),
        skills_reload_ack(1),
        response(4, json!({ "stopReason": "end_turn" })),
    ];
    let (mut backend, _) = backend(messages);
    create_session(&mut backend, session_id);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("prompt"),
        })
        .unwrap();

    let activity = expect_activity_started(&mut backend, ActivityKind::AgentMessage);
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityUpdated {
            activity: observed,
            update: yo_core::ActivityUpdate::TextDelta(text),
        }) if observed == activity && text == "marker"
    ));
    assert_eq!(backend.poll_event().unwrap(), BackendPoll::Pending);

    let mut completions = 0;
    for _ in 0..8 {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ResumableTurnFinished { turn, .. }) => {
                assert_eq!(turn, active_turn);
                completions += 1;
                break;
            },
            BackendPoll::Pending => {},
            BackendPoll::Event(_) => {},
            BackendPoll::Closed => panic!("fixture peer closed before prompt completion"),
        }
    }
    assert_eq!(completions, 1);
}

// maintenance ACK를 소진해도 active prompt와 무관한 숫자 response를 정상 완료로
// 오인하지 않고 기존 foreign-response Protocol 실패를 유지합니다.
#[test]
fn skills_reload_ack_does_not_hide_foreign_prompt_response() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let (mut backend, _) = backend([
        response(3, json!({ "sessionId": "grok-session-a" })),
        skills_reload_ack(0),
        response(99, json!({ "stopReason": "end_turn" })),
    ]);
    create_session(&mut backend, session_id);

    let failure = backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("prompt"),
        })
        .unwrap_err();

    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
    assert!(
        failure
            .message()
            .contains("unexpected Grok ACP response id")
    );
    assert!(backend.prompt.is_none());
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
    let (mut backend, sent) = backend([history, skills_reload_ack(0), response(3, json!({}))]);

    let evidence = backend
        .resume_binding(session_id, &resume_binding("grok-session-a"))
        .unwrap();

    assert_eq!(evidence.session_locator().value(), "grok-session-a");
    assert_eq!(sent.0.borrow()[2]["method"], "session/load");
    assert_eq!(backend.poll_event().unwrap(), BackendPoll::Pending);
}

// 1,025개의 과거 update는 기존 mailbox 상한을 넘지만 재개를 막거나 새 응답으로
// 나타나면 안 됩니다. load 응답 이후 같은 Session의 새 prompt 응답은 그대로 전달합니다.
#[test]
fn long_resume_drains_history_before_accepting_the_next_turn() {
    let session_id = session(1);
    let old = session_update(
        "agent_message_chunk",
        json!({ "content": { "type": "text", "text": "historical" } }),
    );
    let fresh = session_update(
        "agent_message_chunk",
        json!({ "content": { "type": "text", "text": "fresh" } }),
    );
    let messages = std::iter::repeat_n(old, 1025).chain([
        response(3, json!({})),
        fresh,
        response(4, json!({ "stopReason": "end_turn" })),
    ]);
    let (mut backend, sent) = backend(messages);
    backend
        .resume_binding(session_id, &resume_binding("grok-session-a"))
        .unwrap();
    let active_turn = turn(session_id, 2);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::from("continue"),
        })
        .unwrap();
    assert_eq!(sent.0.borrow()[2]["method"], "session/load");
    assert_eq!(sent.0.borrow()[3]["method"], "session/prompt");
    let mut updates = Vec::new();
    let mut completed = false;
    for _ in 0..8 {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ActivityUpdated { update, .. }) => {
                updates.push(update)
            },
            BackendPoll::Event(BackendEvent::ResumableTurnFinished { turn, .. }) => {
                assert_eq!(turn, active_turn);
                completed = true;
                break;
            },
            _ => {},
        }
    }
    assert!(completed);
    assert_eq!(
        updates,
        vec![yo_core::ActivityUpdate::TextDelta("fresh".to_owned())]
    );
}

// history를 걸러도 다른 Session 알림, 다른 메서드, 서버 요청의 대기열 상한은 유지하며
// 오류 응답과 다른 request ID는 성공으로 숨기지 않습니다.
#[test]
fn resume_history_filter_preserves_unrelated_bounds_and_response_failures() {
    let history = session_update(
        "agent_message_chunk",
        json!({ "content": { "type": "text", "text": "old" } }),
    );
    let mut other_session = history.clone();
    other_session["params"]["sessionId"] = json!("different-session");
    let mut other_method = history.clone();
    other_method["method"] = json!("other/notification");
    for unrelated in [
        other_session,
        other_method,
        permission_request("permission", None),
    ] {
        let messages = [history.clone()]
            .into_iter()
            .chain(std::iter::repeat_n(unrelated, 1025));
        let (mut backend, _) = backend(messages);
        let failure = backend
            .resume_binding(session(1), &resume_binding("grok-session-a"))
            .unwrap_err();
        assert!(failure.message().contains("event backlog filled"));
        assert!(backend.session.is_none());
    }
    for (reply, expected) in [
        (
            error_response(3, -32000, "fixture load failed"),
            "fixture load failed",
        ),
        (response(4, json!({})), "unexpected Grok ACP response id"),
    ] {
        let (mut backend, _) = backend([history.clone(), reply]);
        let failure = backend
            .resume_binding(session(1), &resume_binding("grok-session-a"))
            .unwrap_err();
        assert!(failure.message().contains(expected));
        assert!(backend.session.is_none());
    }
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
