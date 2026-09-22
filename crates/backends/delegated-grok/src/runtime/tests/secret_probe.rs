use std::{
    io::{Read, Write},
    net::TcpStream,
    thread,
    time::Duration,
};

use serde_json::{Value, json};
use yo_core::{
    ActivityKind, ActivityQuestion, ActivityResponse, AgentCommand, BackendEvent, BackendPoll,
    SecretInput,
};

use super::{AcpClient, Backend, FakePeer, backend, response, session, session_update, turn};
use crate::runtime::secret_probe::SecretProbeBridge;

fn post(url: &str, message: Value) -> String {
    let address = url.strip_prefix("http://").unwrap();
    let (host, path) = address.split_once('/').unwrap();
    let mut stream = TcpStream::connect(host).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let body = message.to_string();
    write!(
        stream,
        "POST /{path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

// 모의 비밀값은 Yo 입력 이벤트에만 존재하고 MCP 응답에는 고정 상태만 남는다.
#[test]
fn mcp_probe_uses_hidden_input_and_discards_the_sample() {
    let (mut backend, sent) = backend([
        response(3, json!({"sessionId":"grok-session-a"})),
        session_update("current_mode_update", json!({})),
    ]);
    backend.secret_probe = Some(SecretProbeBridge::start().unwrap());
    let url = backend.secret_probe.as_ref().unwrap().server_spec()["url"]
        .as_str()
        .unwrap()
        .to_owned();
    backend
        .execute_command(AgentCommand::CreateSession {
            session_id: session(1),
        })
        .unwrap();
    assert_eq!(sent.0.borrow()[2]["params"]["mcpServers"][0]["url"], url);
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(session(1), 1),
            input: yo_core::UserInput::new("diagnostic"),
        })
        .unwrap();

    let call_url = url.clone();
    let caller = thread::spawn(move || {
        post(
            &call_url,
            json!({
                "jsonrpc":"2.0", "id":7, "method":"tools/call",
                "params":{"name":"yo_secret_entry_probe","arguments":{}}
            }),
        )
    });
    let started = (0..100)
        .find_map(|_| match backend.poll_event().unwrap() {
            BackendPoll::Event(BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::UserInputRequest { request_id },
            }) => Some((activity, request_id)),
            BackendPoll::Pending => {
                thread::sleep(Duration::from_millis(10));
                None
            },
            other => panic!("unexpected probe event: {other:?}"),
        })
        .expect("probe prompt");
    let BackendPoll::Event(BackendEvent::ActivityUpdated {
        update: yo_core::ActivityUpdate::TextSnapshot(snapshot),
        ..
    }) = backend.poll_event().unwrap()
    else {
        panic!("missing hidden question");
    };
    let question = ActivityQuestion::from_snapshot(&snapshot).unwrap();
    assert!(question.is_secret);
    assert!(question.plain_text.contains("made-up sample"));
    let request = yo_core::ActivityRequestRef::new(started.0, started.1);
    backend
        .execute_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::SecretInput(SecretInput::new("sample-only-4382").unwrap()),
        })
        .unwrap();
    let response = caller.join().unwrap();
    assert!(response.contains("Sample secret entry verified locally and discarded by Yo"));
    assert!(!response.contains("sample-only-4382"));
    assert!(
        !sent
            .0
            .borrow()
            .iter()
            .any(|message| message.to_string().contains("sample-only-4382"))
    );
}

// 도구 호출에 값이 실려 오면 입력 UI를 열지 않고 고정 오류만 반환한다.
#[test]
fn mcp_probe_rejects_value_bearing_arguments() {
    let bridge = SecretProbeBridge::start().unwrap();
    let url = bridge.server_spec()["url"].as_str().unwrap().to_owned();
    let response = post(
        &url,
        json!({
            "jsonrpc":"2.0", "id":8, "method":"tools/call",
            "params":{"name":"yo_secret_entry_probe","arguments":{"value":"must-not-be-used"}}
        }),
    );
    assert!(response.contains("isError"));
    assert!(!response.contains("must-not-be-used"));
}

// HTTP MCP를 선언하지 않는 Grok 버전에서는 도구를 조용히 누락하지 않는다.
#[test]
fn enabled_probe_requires_advertised_http_mcp() {
    let (peer, _) = FakePeer::new([
        response(
            1,
            json!({
                "protocolVersion": 1,
                "authMethods": [{"id":"cached_token","name":"cached_token"}],
                "agentInfo": {"name":"grok","version":"1.0.40"},
                "agentCapabilities": {}
            }),
        ),
        response(2, json!({})),
    ]);
    let mut backend = Backend::new_uninitialized(
        AcpClient::new(peer, Duration::from_secs(1)),
        "/workspace".into(),
        false,
    );
    backend.secret_probe = Some(SecretProbeBridge::start().unwrap());
    let failure = backend
        .execute_command(AgentCommand::CreateSession {
            session_id: session(1),
        })
        .unwrap_err();
    assert!(
        failure
            .message()
            .contains("does not advertise local HTTP MCP")
    );
}
