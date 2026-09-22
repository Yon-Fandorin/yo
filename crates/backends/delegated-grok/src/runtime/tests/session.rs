#[cfg(test)]
use std::env;
#[cfg(test)]
use std::iter;

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
    let messages = iter::repeat_n(old, 1025).chain([
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
            .chain(iter::repeat_n(unrelated, 1025));
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
    let config = GrokBackendConfig::new(env::temp_dir())
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
    let cwd = env::current_dir().unwrap();

    GrokBackend::verify(GrokBackendConfig::new(cwd)).unwrap();
}

// 실제 Grok에서 모의 비밀 입력 MCP를 붙인 Session을 열되 모델 Turn은 만들지 않는다.
#[test]
#[ignore = "requires a logged-in Grok CLI and YO_GROK_SECRET_ENTRY_PROBE=1"]
fn local_grok_probe_opens_session_without_model_turn() {
    assert_eq!(env::var("YO_GROK_SECRET_ENTRY_PROBE").as_deref(), Ok("1"));
    let cwd = env::current_dir().unwrap();
    let mut backend = GrokBackend::spawn(GrokBackendConfig::new(cwd)).unwrap();
    let evidence = yo_backend::BackendAdapter::execute_command(
        &mut backend,
        AgentCommand::CreateSession {
            session_id: session(99),
        },
    )
    .unwrap();
    assert!(matches!(evidence, BackendCommandEvidence::BindingOpened(_)));
    yo_backend::BackendAdapter::shutdown(&mut backend).unwrap();
}

const LIVE_PROBE_TOOL: &str = "yo_secret_entry_probe__yo_secret_entry_probe";
const LIVE_PROBE_SERVER: &str = "yo_secret_entry_probe";
const LIVE_PROBE_UNQUALIFIED_TOOL: &str = "yo_secret_entry_probe";
const LIVE_PROBE_RESULT: &str = "Sample secret entry verified locally and discarded by Yo. No value was sent to Grok or the model.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LiveToolResult {
    Search,
    Probe,
}

fn contains_probe_tool(value: &Value) -> bool {
    match value {
        Value::String(text) => text.match_indices(LIVE_PROBE_TOOL).any(|(start, _)| {
            let end = start + LIVE_PROBE_TOOL.len();
            let is_identifier = |character: char| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
            };
            text[..start]
                .chars()
                .next_back()
                .is_none_or(|c| !is_identifier(c))
                && text[end..].chars().next().is_none_or(|c| !is_identifier(c))
        }),
        Value::Array(items) => items.iter().any(contains_probe_tool),
        Value::Object(fields) => fields.values().any(contains_probe_tool),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

fn contains_exact_text(value: &Value, expected: &str) -> bool {
    match value {
        Value::String(text) => text == expected,
        Value::Array(items) => items.iter().any(|item| contains_exact_text(item, expected)),
        Value::Object(fields) => fields
            .values()
            .any(|field| contains_exact_text(field, expected)),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

fn exact_probe_raw_output() -> Value {
    json!({
        "type":"MCP",
        "server_name":LIVE_PROBE_SERVER,
        "tool_name":LIVE_PROBE_UNQUALIFIED_TOOL,
        "output":{
            "content":[{"type":"text", "text":LIVE_PROBE_RESULT}],
            "isError":false
        }
    })
}

fn classify_live_tool_result(snapshot: &str) -> Option<LiveToolResult> {
    let output = yo_core::ToolOutput::from_snapshot(snapshot)?;
    if output.server.is_some() || output.error.is_some() {
        return None;
    }
    let result = output.result.as_ref()?;
    let tool_name = result
        .get("_meta")?
        .get("x.ai/tool")?
        .get("name")?
        .as_str()?;
    match tool_name {
        "search_tool" => {
            let arguments = output.arguments.as_ref()?.as_object()?;
            let query = arguments.get("query")?.as_str()?;
            let exact_shape = arguments
                .keys()
                .all(|field| matches!(field.as_str(), "query" | "limit" | "variant"));
            let catalog_contains_probe = output
                .content_items
                .as_ref()
                .is_some_and(contains_probe_tool)
                || result.get("rawOutput").is_some_and(contains_probe_tool);
            (exact_shape && query.contains("yo_secret_entry_probe") && catalog_contains_probe)
                .then_some(LiveToolResult::Search)
        },
        "use_tool" => {
            let arguments = output.arguments.as_ref()?.as_object()?;
            let exact_shape = arguments
                .keys()
                .all(|field| matches!(field.as_str(), "tool_name" | "tool_input" | "variant"));
            let exact_target = arguments.get("tool_name")?.as_str()? == LIVE_PROBE_TOOL;
            let empty_input = arguments.get("tool_input")? == &json!({});
            let valid_variant = arguments.get("variant").is_none_or(Value::is_string);
            let exact_content = json!([{"type":"text", "text":LIVE_PROBE_RESULT}]);
            let fixed_result = output.content_items.as_ref() == Some(&exact_content)
                || result.get("rawOutput") == Some(&exact_probe_raw_output());
            (exact_shape && exact_target && empty_input && valid_variant && fixed_result)
                .then_some(LiveToolResult::Probe)
        },
        _ => None,
    }
}

fn describe_live_tool_result(snapshot: &str) -> String {
    let Some(output) = yo_core::ToolOutput::from_snapshot(snapshot) else {
        return "parsed=false".into();
    };
    let arguments = output.arguments.as_ref().and_then(Value::as_object);
    let result = output.result.as_ref().and_then(Value::as_object);
    let tool_name = result
        .and_then(|result| result.get("_meta"))
        .and_then(|meta| meta.get("x.ai/tool"))
        .and_then(|tool| tool.get("name"))
        .and_then(Value::as_str);
    let name_class = match tool_name {
        Some("search_tool") => "search_tool",
        Some("use_tool") => "use_tool",
        Some(_) => "other",
        None => "absent",
    };
    let argument_keys = arguments
        .map(|arguments| arguments.keys().map(String::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    let result_keys = result
        .map(|result| result.keys().map(String::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    let location_count = result
        .and_then(|result| result.get("locations"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let content_count = output
        .content_items
        .as_ref()
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let fixed_result_matches =
        output.content_items.as_ref() == Some(&json!([{"type":"text", "text":LIVE_PROBE_RESULT}]));
    let exact_content = json!([{"type":"text", "text":LIVE_PROBE_RESULT}]);
    let raw_output = result.and_then(|result| result.get("rawOutput"));
    let raw_output_keys = raw_output
        .and_then(Value::as_object)
        .map(|output| output.keys().map(String::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    let raw_content = raw_output.and_then(|output| output.get("content"));
    let raw_payload = raw_output.and_then(|output| output.get("output"));
    let raw_payload_keys = raw_payload
        .and_then(Value::as_object)
        .map(|payload| payload.keys().map(String::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    let raw_okay_output = raw_payload.and_then(|payload| payload.get("OkayOutput"));
    let raw_okay_output_keys = raw_okay_output
        .and_then(Value::as_object)
        .map(|output| output.keys().map(String::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    let fixed_raw_output_matches = raw_output == Some(&exact_probe_raw_output());
    let fixed_raw_content_matches = raw_content == Some(&exact_content);
    let fixed_raw_payload_matches = raw_payload == Some(&json!({"content":exact_content.clone()}));
    let fixed_normalized_payload_matches =
        raw_payload == Some(&json!({"content":exact_content.clone(), "isError":false}));
    let fixed_raw_payload_content_matches =
        raw_payload.and_then(|payload| payload.get("content")) == Some(&exact_content);
    let fixed_raw_okay_matches = raw_okay_output == Some(&exact_content)
        || raw_okay_output == Some(&json!({"content":exact_content.clone()}))
        || raw_okay_output == Some(&json!({"content":exact_content.clone(), "isError":false}));
    let fixed_raw_okay_content_matches =
        raw_okay_output.and_then(|output| output.get("content")) == Some(&exact_content);
    let fixed_raw_text_present = raw_output.is_some_and(|output| {
        contains_exact_text(output, LIVE_PROBE_RESULT)
            || output
                .as_str()
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
                .is_some_and(|parsed| contains_exact_text(&parsed, LIVE_PROBE_RESULT))
    });
    let raw_error_class = match raw_output.and_then(|output| output.get("isError")) {
        Some(Value::Bool(false)) => "false",
        Some(Value::Bool(true)) => "true",
        Some(_) => "other",
        None => "absent",
    };
    let raw_payload_error_class = match raw_payload.and_then(|payload| payload.get("isError")) {
        Some(Value::Bool(false)) => "false",
        Some(Value::Bool(true)) => "true",
        Some(_) => "other",
        None => "absent",
    };
    let raw_okay_error_class = match raw_okay_output.and_then(|output| output.get("isError")) {
        Some(Value::Bool(false)) => "false",
        Some(Value::Bool(true)) => "true",
        Some(_) => "other",
        None => "absent",
    };
    let catalog_contains_probe = output
        .content_items
        .as_ref()
        .is_some_and(contains_probe_tool)
        || result
            .and_then(|result| result.get("rawOutput"))
            .is_some_and(contains_probe_tool);
    format!(
        "parsed=true name={name_class} argument_keys={argument_keys:?} result_keys={result_keys:?} locations={location_count} content_items={content_count} catalog_contains_probe={catalog_contains_probe} target_matches={} input_empty={} query_matches={} fixed_result_matches={fixed_result_matches} raw_output_type={} raw_output_keys={raw_output_keys:?} raw_type_matches={} raw_server_matches={} raw_tool_matches={} raw_content_type={} raw_content_count={} raw_payload_type={} raw_payload_keys={raw_payload_keys:?} raw_okay_type={} raw_okay_keys={raw_okay_output_keys:?} raw_okay_count={} fixed_raw_output_matches={fixed_raw_output_matches} fixed_raw_content_matches={fixed_raw_content_matches} fixed_raw_payload_matches={fixed_raw_payload_matches} fixed_normalized_payload_matches={fixed_normalized_payload_matches} fixed_raw_payload_content_matches={fixed_raw_payload_content_matches} fixed_raw_okay_matches={fixed_raw_okay_matches} fixed_raw_okay_content_matches={fixed_raw_okay_content_matches} fixed_raw_text_present={fixed_raw_text_present} raw_error={raw_error_class} raw_payload_error={raw_payload_error_class} raw_okay_error={raw_okay_error_class} server_present={} error_present={}",
        arguments
            .and_then(|arguments| arguments.get("tool_name"))
            .and_then(Value::as_str)
            == Some(LIVE_PROBE_TOOL),
        arguments.and_then(|arguments| arguments.get("tool_input")) == Some(&json!({})),
        arguments
            .and_then(|arguments| arguments.get("query"))
            .and_then(Value::as_str)
            .is_some_and(|query| query.contains("yo_secret_entry_probe")),
        raw_output.map_or("absent", json_type_name),
        raw_output
            .and_then(|output| output.get("type"))
            .and_then(Value::as_str)
            == Some("MCP"),
        raw_output
            .and_then(|output| output.get("server_name"))
            .and_then(Value::as_str)
            == Some(LIVE_PROBE_SERVER),
        raw_output
            .and_then(|output| output.get("tool_name"))
            .and_then(Value::as_str)
            == Some(LIVE_PROBE_UNQUALIFIED_TOOL),
        raw_content.map_or("absent", json_type_name),
        raw_content.and_then(Value::as_array).map_or(0, Vec::len),
        raw_payload.map_or("absent", json_type_name),
        raw_okay_output.map_or("absent", json_type_name),
        raw_okay_output
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        output.server.is_some(),
        output.error.is_some(),
    )
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

// Grok의 display title이 아니라 보존된 ToolOutput의 실제 wrapper·target·결과를 판별합니다.
#[test]
fn live_probe_result_classifier_requires_exact_structured_output() {
    let probe = yo_core::ToolOutput {
        tool: "opaque-call-id".into(),
        server: None,
        arguments: Some(json!({
            "tool_name": LIVE_PROBE_TOOL,
            "tool_input": {}
        })),
        result: Some(json!({"_meta":{"x.ai/tool":{"name":"use_tool"}}})),
        content_items: Some(json!([{"type":"text", "text":LIVE_PROBE_RESULT}])),
        error: None,
        plain_text: "presentation is not identity".into(),
    };
    assert_eq!(
        classify_live_tool_result(&probe.to_snapshot().unwrap()),
        Some(LiveToolResult::Probe)
    );
    let raw_output_probe = yo_core::ToolOutput {
        arguments: Some(json!({
            "tool_name": LIVE_PROBE_TOOL,
            "tool_input": {},
            "variant":"full"
        })),
        result: Some(json!({
            "locations":[],
            "rawOutput":exact_probe_raw_output(),
            "_meta":{"x.ai/tool":{"name":"use_tool"}}
        })),
        content_items: None,
        ..probe.clone()
    };
    assert_eq!(
        classify_live_tool_result(&raw_output_probe.to_snapshot().unwrap()),
        Some(LiveToolResult::Probe)
    );

    let search = yo_core::ToolOutput {
        tool: "another-opaque-call-id".into(),
        arguments: Some(json!({
            "query":"yo_secret_entry_probe",
            "limit":5,
            "variant":"full"
        })),
        result: Some(json!({
            "locations":[],
            "_meta":{"x.ai/tool":{"name":"search_tool"}}
        })),
        content_items: Some(json!([{"type":"text","text":LIVE_PROBE_TOOL}])),
        plain_text: "presentation remains unrelated".into(),
        ..probe.clone()
    };
    assert_eq!(
        classify_live_tool_result(&search.to_snapshot().unwrap()),
        Some(LiveToolResult::Search)
    );
    let raw_output_search = yo_core::ToolOutput {
        result: Some(json!({
            "locations":[],
            "rawOutput":{"matches":[LIVE_PROBE_TOOL]},
            "_meta":{"x.ai/tool":{"name":"search_tool"}}
        })),
        content_items: None,
        ..search.clone()
    };
    assert_eq!(
        classify_live_tool_result(&raw_output_search.to_snapshot().unwrap()),
        Some(LiveToolResult::Search)
    );

    let unrelated_target = yo_core::ToolOutput {
        arguments: Some(json!({
            "tool_name": "other__tool",
            "tool_input": {"note": LIVE_PROBE_TOOL}
        })),
        plain_text: LIVE_PROBE_TOOL.into(),
        ..probe.clone()
    };
    assert_eq!(
        classify_live_tool_result(&unrelated_target.to_snapshot().unwrap()),
        None
    );

    for invalid_probe in [
        yo_core::ToolOutput {
            arguments: Some(json!({
                "tool_name": LIVE_PROBE_TOOL,
                "tool_input": {},
                "unexpected":true
            })),
            ..probe.clone()
        },
        yo_core::ToolOutput {
            result: Some(json!({
                "rawOutput":{
                    "type":"MCP",
                    "server_name":LIVE_PROBE_SERVER,
                    "tool_name":LIVE_PROBE_UNQUALIFIED_TOOL,
                    "output":{
                        "content":[{
                            "type":"text",
                            "text":format!("{LIVE_PROBE_RESULT} extra")
                        }],
                        "isError":false
                    }
                },
                "_meta":{"x.ai/tool":{"name":"use_tool"}}
            })),
            content_items: None,
            ..probe.clone()
        },
        yo_core::ToolOutput {
            result: Some(json!({
                "rawOutput":{
                    "type":"MCP",
                    "server_name":LIVE_PROBE_SERVER,
                    "tool_name":LIVE_PROBE_UNQUALIFIED_TOOL,
                    "output":{
                        "isError":true,
                        "content":[{"type":"text","text":LIVE_PROBE_RESULT}]
                    }
                },
                "_meta":{"x.ai/tool":{"name":"use_tool"}}
            })),
            content_items: None,
            ..probe.clone()
        },
        yo_core::ToolOutput {
            result: Some(json!({
                "rawOutput":{
                    "type":"MCP",
                    "server_name":LIVE_PROBE_SERVER,
                    "tool_name":LIVE_PROBE_UNQUALIFIED_TOOL,
                    "output":{
                        "content":[{"type":"text","text":LIVE_PROBE_RESULT}],
                        "isError":false
                    },
                    "metadata":LIVE_PROBE_RESULT
                },
                "_meta":{"x.ai/tool":{"name":"use_tool"}}
            })),
            content_items: None,
            ..probe.clone()
        },
        yo_core::ToolOutput {
            result: Some(json!({
                "rawOutput":{
                    "type":"MCP",
                    "server_name":"other_server",
                    "tool_name":LIVE_PROBE_UNQUALIFIED_TOOL,
                    "output":{
                        "content":[{"type":"text","text":LIVE_PROBE_RESULT}],
                        "isError":false
                    }
                },
                "_meta":{"x.ai/tool":{"name":"use_tool"}}
            })),
            content_items: None,
            ..probe.clone()
        },
    ] {
        assert_eq!(
            classify_live_tool_result(&invalid_probe.to_snapshot().unwrap()),
            None
        );
    }

    for invalid in [
        yo_core::ToolOutput {
            result: Some(json!({})),
            ..probe.clone()
        },
        yo_core::ToolOutput {
            result: Some(json!({
                "locations":[],
                "rawOutput":{"matches":[]},
                "_meta":{"x.ai/tool":{"name":"search_tool"}}
            })),
            content_items: Some(json!([])),
            ..search.clone()
        },
        yo_core::ToolOutput {
            content_items: Some(json!([{
                "type":"text",
                "text":format!("{LIVE_PROBE_TOOL}_other")
            }])),
            ..search.clone()
        },
        yo_core::ToolOutput {
            result: Some(json!({
                "locations":[{"path":"catalog-entry"}],
                "rawOutput":{"matches":[LIVE_PROBE_TOOL]},
                "_meta":{"x.ai/tool":{"name":"other_tool"}}
            })),
            ..search.clone()
        },
    ] {
        assert_eq!(
            classify_live_tool_result(&invalid.to_snapshot().unwrap()),
            None
        );
    }
}

// 유료 Grok 실서비스를 명시적으로 선택한 경우에만 모델의 MCP 도구 선택부터 숨김 입력,
// 고정 결과 수신과 Turn 완료까지 검증합니다. 테스트 문자열은 실제 인증 정보가 아닙니다.
#[test]
#[ignore = "uses a paid Grok model Turn; requires cached login and YO_GROK_SECRET_ENTRY_PROBE=1"]
fn local_grok_probe_completes_model_turn_with_discarded_sample() {
    use std::{
        collections::{HashMap, HashSet},
        thread,
        time::{Duration, Instant},
    };

    use yo_core::{ActivityOutcome, ActivityQuestion, ActivityResponse, SecretInput, TurnOutcome};

    const RESULT_MARKER: &str = "YO_SECRET_PROBE_RESULT_ACK";
    assert_eq!(env::var("YO_GROK_SECRET_ENTRY_PROBE").as_deref(), Ok("1"));
    let cwd = env::current_dir().unwrap();
    let session_id = session(100);
    let active_turn = turn(session_id, 1);
    let mut backend = GrokBackend::spawn(GrokBackendConfig::new(cwd)).unwrap();
    yo_backend::BackendAdapter::execute_command(
        &mut backend,
        AgentCommand::CreateSession { session_id },
    )
    .unwrap();
    yo_backend::BackendAdapter::execute_command(
        &mut backend,
        AgentCommand::StartTurn {
            turn: active_turn,
            input: UserInput::new("Diagnostic only: first use search_tool to find the fully qualified MCP tool name for yo_secret_entry_probe, then call that tool once with empty arguments. After the tool result, reply with exactly YO_SECRET_PROBE_RESULT_ACK and no other text. Do not use unrelated tools, inspect files, or ask another question."),
        },
    )
    .unwrap();

    let deadline = Instant::now() + Duration::from_secs(120);
    let mut secret_request = None;
    let mut secret_submitted = false;
    let mut activity_kinds = HashMap::new();
    let mut activity_text = HashMap::new();
    let mut tool_calls = HashSet::new();
    let mut completed_tool_calls = HashSet::new();
    let mut tool_results = HashSet::new();
    let mut classified_results = HashMap::new();
    let mut probe_result_activity = None;
    let mut probe_tool_completed = false;
    let mut marker_activities = HashSet::new();
    let mut final_marker_seen = false;
    let mut finished = false;
    while Instant::now() < deadline {
        match yo_backend::BackendAdapter::poll_event(&mut backend).unwrap() {
            BackendPoll::Event(BackendEvent::ActivityStarted { activity, kind }) => {
                assert!(activity_kinds.insert(activity, kind).is_none());
                match kind {
                    ActivityKind::UserInputRequest { request_id } => {
                        assert!(secret_request.is_none(), "unexpected second user question");
                        assert!(
                            !secret_submitted,
                            "unexpected question after secret submission"
                        );
                        secret_request =
                            Some(yo_core::ActivityRequestRef::new(activity, request_id));
                    },
                    ActivityKind::ToolCall => {
                        tool_calls.insert(activity);
                    },
                    ActivityKind::ToolResult => {
                        tool_results.insert(activity);
                    },
                    ActivityKind::FileChange => {
                        panic!("the diagnostic Turn must not change files");
                    },
                    ActivityKind::ApprovalRequest { .. }
                    | ActivityKind::ApprovalResponse { .. } => {
                        panic!("the diagnostic Turn must not enter a permission flow");
                    },
                    ActivityKind::ModelWork
                    | ActivityKind::AgentMessage
                    | ActivityKind::UserInputResponse { .. } => {},
                }
            },
            BackendPoll::Event(BackendEvent::ActivityUpdated { activity, update }) => {
                let text = activity_text.entry(activity).or_insert_with(String::new);
                match &update {
                    yo_core::ActivityUpdate::TextDelta(delta) => text.push_str(delta),
                    yo_core::ActivityUpdate::TextSnapshot(snapshot) => text.clone_from(snapshot),
                }
                assert!(
                    !text.contains("yo-mock-only-4382"),
                    "sample leaked into an Activity event stream"
                );
                if activity_kinds.get(&activity) == Some(&ActivityKind::ToolResult) {
                    classified_results.remove(&activity);
                    if probe_result_activity == Some(activity) {
                        probe_result_activity = None;
                    }
                    if let Some(classification) = classify_live_tool_result(text) {
                        classified_results.insert(activity, classification);
                        if classification == LiveToolResult::Probe {
                            assert!(
                                probe_result_activity.is_none(),
                                "probe result was published by multiple Activities"
                            );
                            probe_result_activity = Some(activity);
                        }
                    }
                    if probe_result_activity == Some(activity) {
                        assert!(secret_submitted, "probe result preceded hidden submission");
                    }
                }
                if activity_kinds.get(&activity) == Some(&ActivityKind::AgentMessage) {
                    marker_activities.remove(&activity);
                    if text.contains(RESULT_MARKER) {
                        assert!(
                            probe_result_activity.is_some(),
                            "the final marker arrived before the verified probe result"
                        );
                        if text.trim() == RESULT_MARKER {
                            marker_activities.insert(activity);
                        }
                    }
                }
                let yo_core::ActivityUpdate::TextSnapshot(snapshot) = &update else {
                    continue;
                };
                if secret_request.is_some_and(|request| request.activity() == activity) {
                    let request = secret_request.take().expect("matched secret request");
                    let question =
                        ActivityQuestion::from_snapshot(snapshot).expect("probe question");
                    assert_eq!(
                        question,
                        ActivityQuestion {
                            plain_text: "Sample input test\n\nEnter a made-up sample value only. Never enter a real password, token, or credential. Yo discards this value and sends only a fixed completion status to Grok.\nEsc interrupts the turn.".into(),
                            choices: Vec::new(),
                            allow_notes: false,
                            is_secret: true,
                            storage_offer: None,
                            previous_question: false,
                            draft: None,
                            draft_choice: None,
                        }
                    );
                    yo_backend::BackendAdapter::execute_command(
                        &mut backend,
                        AgentCommand::RespondToActivity {
                            request,
                            response: ActivityResponse::SecretInput(
                                SecretInput::new("yo-mock-only-4382").unwrap(),
                            ),
                        },
                    )
                    .unwrap();
                    secret_submitted = true;
                }
            },
            BackendPoll::Event(BackendEvent::ActivityFinished { activity, outcome }) => {
                let kind = activity_kinds
                    .get(&activity)
                    .copied()
                    .expect("finished Activity must have started");
                let text = activity_text
                    .get(&activity)
                    .map(String::as_str)
                    .unwrap_or_default();
                match kind {
                    ActivityKind::ToolCall => {
                        assert_eq!(outcome, ActivityOutcome::Completed);
                        assert!(completed_tool_calls.insert(activity));
                    },
                    ActivityKind::ToolResult => {
                        assert_eq!(outcome, ActivityOutcome::Completed);
                        let classification = classify_live_tool_result(text).unwrap_or_else(|| {
                            panic!(
                                "unexpected or incomplete Grok ToolOutput: {}",
                                describe_live_tool_result(text)
                            )
                        });
                        assert_eq!(classified_results.get(&activity), Some(&classification));
                        if classification == LiveToolResult::Probe {
                            assert!(!probe_tool_completed, "probe tool ran more than once");
                            probe_tool_completed = true;
                        }
                    },
                    ActivityKind::AgentMessage => {
                        final_marker_seen = marker_activities.contains(&activity)
                            && outcome == ActivityOutcome::Completed
                            && text.trim() == RESULT_MARKER;
                    },
                    _ => {},
                }
            },
            BackendPoll::Event(BackendEvent::ResumableTurnFinished { turn, .. }) => {
                assert_eq!(turn, active_turn);
                finished = true;
                break;
            },
            BackendPoll::Event(BackendEvent::TurnFinished { turn, outcome }) => {
                assert_eq!(turn, active_turn);
                assert_eq!(outcome, TurnOutcome::Completed);
                finished = true;
                break;
            },
            BackendPoll::Pending => thread::sleep(Duration::from_millis(20)),
            BackendPoll::Closed => panic!("Grok ACP closed before Turn completion"),
            BackendPoll::Event(_) => {},
        }
    }
    yo_backend::BackendAdapter::shutdown(&mut backend).unwrap();
    assert!(
        secret_submitted,
        "model did not invoke the secret-entry probe"
    );
    assert!(
        probe_tool_completed,
        "Grok did not complete the exact MCP probe invocation"
    );
    assert_eq!(tool_calls.len(), 2, "unexpected number of Grok tool calls");
    assert_eq!(completed_tool_calls, tool_calls);
    assert_eq!(
        tool_results.len(),
        2,
        "unexpected number of Grok tool results"
    );
    assert_eq!(classified_results.len(), tool_results.len());
    assert_eq!(
        classified_results
            .values()
            .filter(|result| **result == LiveToolResult::Search)
            .count(),
        1
    );
    assert_eq!(
        classified_results
            .values()
            .filter(|result| **result == LiveToolResult::Probe)
            .count(),
        1
    );
    assert!(
        final_marker_seen,
        "the post-probe final model answer did not contain the required marker"
    );
    assert!(
        finished,
        "Grok model Turn did not complete before the deadline"
    );
}

// Yo outer sandbox smoke는 실제 mount/write attestation 뒤 native sandbox 대신 exact
// no-tools ACP argv로 인증까지만 수행하고 Agent Session이나 inference Turn을 만들지 않습니다.
#[test]
#[ignore = "requires the Yo bwrap profile plus a compatible installed and logged-in Grok CLI"]
fn local_outer_sandbox_grok_authenticates_without_a_session() {
    let cwd = env::current_dir().unwrap();
    let config = GrokBackendConfig::new(cwd)
        .with_read_only_review(true)
        .with_outer_sandboxed_review(true);

    GrokBackend::verify(config).unwrap();
}
