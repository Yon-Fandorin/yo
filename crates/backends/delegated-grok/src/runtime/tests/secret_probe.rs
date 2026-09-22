use std::{
    io::{ErrorKind, Read, Write},
    net::TcpStream,
    thread,
    time::{Duration, Instant},
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

fn prepared_backend() -> (Backend<FakePeer>, String) {
    let (mut backend, _) = backend([
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
    (backend, url)
}

fn start_turn(backend: &mut Backend<FakePeer>) {
    backend
        .execute_command(AgentCommand::StartTurn {
            turn: turn(session(1), 1),
            input: yo_core::UserInput::new("diagnostic"),
        })
        .unwrap();
}

fn await_secret_request(backend: &mut Backend<FakePeer>) -> yo_core::ActivityRequestRef {
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
        panic!("missing hidden question")
    };
    assert!(
        ActivityQuestion::from_snapshot(&snapshot)
            .unwrap()
            .is_secret
    );
    yo_core::ActivityRequestRef::new(started.0, started.1)
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
    let (mut backend, url) = prepared_backend();
    start_turn(&mut backend);
    let response = post(
        &url,
        json!({
            "jsonrpc":"2.0", "id":8, "method":"tools/call",
            "params":{"name":"yo_secret_entry_probe","arguments":{"value":"must-not-be-used"}}
        }),
    );
    assert!(response.contains("isError"));
    assert!(!response.contains("must-not-be-used"));
    assert_eq!(backend.poll_event().unwrap(), BackendPoll::Pending);
}

// Turn 밖에서 온 호출은 뒤의 Turn에 넘어가 숨김 입력을 열 수 없다.
#[test]
fn idle_probe_call_cannot_open_later_turn() {
    let (mut backend, url) = prepared_backend();
    let response = post(
        &url,
        json!({
            "jsonrpc":"2.0", "id":9, "method":"tools/call",
            "params":{"name":"yo_secret_entry_probe"}
        }),
    );
    assert!(response.contains("isError"));
    start_turn(&mut backend);
    assert_eq!(backend.poll_event().unwrap(), BackendPoll::Pending);
}

// 첫 호출이 대기 중인 동안 두 번째 호출은 큐에 쌓이지 않는다.
#[test]
fn concurrent_probe_call_is_rejected_at_ingress() {
    let (mut backend, url) = prepared_backend();
    start_turn(&mut backend);
    let call_url = url.clone();
    let first = thread::spawn(move || {
        post(
            &call_url,
            json!({
                "jsonrpc":"2.0", "id":10, "method":"tools/call",
                "params":{"name":"yo_secret_entry_probe","arguments":{}}
            }),
        )
    });
    let request = await_secret_request(&mut backend);
    let second = post(
        &url,
        json!({
            "jsonrpc":"2.0", "id":11, "method":"tools/call",
            "params":{"name":"yo_secret_entry_probe","arguments":{}}
        }),
    );
    assert!(second.contains("isError"));
    backend
        .execute_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::SecretInput(SecretInput::new("sample-concurrent").unwrap()),
        })
        .unwrap();
    assert!(first.join().unwrap().contains("verified locally"));
    for _ in 0..4 {
        let _ = backend.poll_event().unwrap();
    }
    assert_eq!(backend.poll_event().unwrap(), BackendPoll::Pending);
}

// MCP 취소는 이미 열린 숨김 입력을 중단하고 모의값을 요구하지 않는다.
#[test]
fn mcp_cancellation_closes_hidden_input() {
    let (mut backend, url) = prepared_backend();
    start_turn(&mut backend);
    let call_url = url.clone();
    let caller = thread::spawn(move || {
        post(
            &call_url,
            json!({
                "jsonrpc":"2.0", "id":12, "method":"tools/call",
                "params":{"name":"yo_secret_entry_probe"}
            }),
        )
    });
    let request = await_secret_request(&mut backend);
    let cancelled = post(
        &url,
        json!({
            "jsonrpc":"2.0", "method":"notifications/cancelled", "params":{"requestId":12}
        }),
    );
    assert!(cancelled.starts_with("HTTP/1.1 202"));
    let event = (0..100)
        .find_map(|_| match backend.poll_event().unwrap() {
            BackendPoll::Event(event @ BackendEvent::ActivityFinished { .. }) => Some(event),
            BackendPoll::Pending => {
                thread::sleep(Duration::from_millis(10));
                None
            },
            other => panic!("unexpected cancellation event: {other:?}"),
        })
        .expect("cancelled activity");
    assert!(
        matches!(event, BackendEvent::ActivityFinished { activity, outcome: yo_core::ActivityOutcome::Interrupted } if activity == request.activity())
    );
    assert!(caller.join().unwrap().contains("isError"));
}

// 취소된 호출 하나를 정리해도 같은 Turn에서 다음 MCP 호출을 받을 수 있다.
#[test]
fn cancelled_probe_does_not_disable_the_rest_of_its_turn() {
    let (mut backend, url) = prepared_backend();
    start_turn(&mut backend);
    let call_url = url.clone();
    let cancelled_call = thread::spawn(move || {
        post(
            &call_url,
            json!({
                "jsonrpc":"2.0", "id":15, "method":"tools/call",
                "params":{"name":"yo_secret_entry_probe"}
            }),
        )
    });
    let cancelled_request = await_secret_request(&mut backend);
    let _ = post(
        &url,
        json!({
            "jsonrpc":"2.0", "method":"notifications/cancelled", "params":{"requestId":15}
        }),
    );
    assert!(matches!(backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished { activity, outcome: yo_core::ActivityOutcome::Interrupted })
            if activity == cancelled_request.activity()));
    assert!(cancelled_call.join().unwrap().contains("isError"));

    let call_url = url.clone();
    let retry = thread::spawn(move || {
        post(
            &call_url,
            json!({
                "jsonrpc":"2.0", "id":16, "method":"tools/call",
                "params":{"name":"yo_secret_entry_probe"}
            }),
        )
    });
    let retry_request = await_secret_request(&mut backend);
    backend
        .execute_command(AgentCommand::RespondToActivity {
            request: retry_request,
            response: ActivityResponse::SecretInput(
                SecretInput::new("sample-after-cancel").unwrap(),
            ),
        })
        .unwrap();
    assert!(retry.join().unwrap().contains("verified locally"));
}

// 취소 알림 직후 사용자가 제출해도 취소가 고정 완료 상태보다 먼저 확정된다.
#[test]
fn cancellation_before_submission_cannot_report_success() {
    let (mut backend, url) = prepared_backend();
    start_turn(&mut backend);
    let call_url = url.clone();
    let caller = thread::spawn(move || {
        post(
            &call_url,
            json!({
                "jsonrpc":"2.0", "id":14, "method":"tools/call",
                "params":{"name":"yo_secret_entry_probe"}
            }),
        )
    });
    let request = await_secret_request(&mut backend);
    let _ = post(
        &url,
        json!({
            "jsonrpc":"2.0", "method":"notifications/cancelled", "params":{"requestId":14}
        }),
    );
    backend
        .execute_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::SecretInput(
                SecretInput::new("sample-after-cancel").unwrap(),
            ),
        })
        .unwrap();
    assert!(caller.join().unwrap().contains("isError"));
    assert!(matches!(backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished { activity, outcome: yo_core::ActivityOutcome::Interrupted })
            if activity == request.activity()));
}

// 호출한 HTTP 연결이 닫히면 숨김 입력을 계속 대기시키지 않는다.
#[test]
fn http_disconnect_closes_hidden_input() {
    let (mut backend, url) = prepared_backend();
    start_turn(&mut backend);
    let address = url.strip_prefix("http://").unwrap();
    let (host, path) = address.split_once('/').unwrap();
    let mut stream = TcpStream::connect(host).unwrap();
    let body = json!({
        "jsonrpc":"2.0", "id":13, "method":"tools/call",
        "params":{"name":"yo_secret_entry_probe"}
    })
    .to_string();
    write!(
        stream,
        "POST /{path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let request = await_secret_request(&mut backend);
    drop(stream);
    let event = (0..100)
        .find_map(|_| match backend.poll_event().unwrap() {
            BackendPoll::Event(event @ BackendEvent::ActivityFinished { .. }) => Some(event),
            BackendPoll::Pending => {
                thread::sleep(Duration::from_millis(10));
                None
            },
            other => panic!("unexpected disconnection event: {other:?}"),
        })
        .expect("disconnected activity");
    assert!(
        matches!(event, BackendEvent::ActivityFinished { activity, outcome: yo_core::ActivityOutcome::Interrupted } if activity == request.activity())
    );
}

// Content-Length를 넘긴 바이트나 파이프라인 요청은 입력 대기 상태로 들어가지 않는다.
#[test]
fn extra_http_bytes_are_rejected_before_opening_hidden_input() {
    let (mut backend, url) = prepared_backend();
    start_turn(&mut backend);
    let address = url.strip_prefix("http://").unwrap();
    let (host, path) = address.split_once('/').unwrap();
    let mut stream = TcpStream::connect(host).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let body = json!({
        "jsonrpc":"2.0", "id":17, "method":"tools/call",
        "params":{"name":"yo_secret_entry_probe"}
    })
    .to_string();
    write!(
        stream,
        "POST /{path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: {}\r\n\r\n{body}x",
        body.len()
    )
    .unwrap();
    let mut response = String::new();
    match stream.read_to_string(&mut response) {
        Ok(_) => assert!(response.starts_with("HTTP/1.1 400")),
        // 소켓에 남은 파이프라인 바이트 때문에 운영체제가 RST를 보내도 호출은 거부된다.
        Err(error) => assert!(matches!(
            error.kind(),
            ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted
        )),
    }
    assert_eq!(backend.poll_event().unwrap(), BackendPoll::Pending);
}

// 느리게 헤더를 보내는 연결도 백엔드 종료를 붙잡지 못한다.
#[test]
fn slow_http_client_does_not_block_probe_shutdown() {
    let bridge = SecretProbeBridge::start().unwrap();
    let url = bridge.server_spec()["url"].as_str().unwrap().to_owned();
    let address = url
        .strip_prefix("http://")
        .unwrap()
        .split_once('/')
        .unwrap()
        .0;
    let mut stream = TcpStream::connect(address).unwrap();
    stream.write_all(b"POST /mcp/").unwrap();
    let start = Instant::now();
    drop(bridge);
    assert!(start.elapsed() < Duration::from_secs(2));
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
