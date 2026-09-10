use std::time::Duration;

use serde_json::{Value, json};

use super::{AppServerClient, MAX_OUTBOUND_JSONL_MESSAGE_BYTES};
use crate::{protocol, test_support::FakePeer};

fn turn_params(text: String) -> Value {
    json!({
        "threadId": "thread-a",
        "input": [{"type": "text", "text": text}],
        "cwd": "/workspace"
    })
}

fn boundary_text(prefix: &str) -> String {
    let base = serde_json::to_vec(&protocol::request(
        1,
        "turn/start",
        turn_params(prefix.to_owned()),
    ))
    .expect("the JSON fixture is serializable");
    let text = format!(
        "{prefix}{}",
        "x".repeat(MAX_OUTBOUND_JSONL_MESSAGE_BYTES - base.len())
    );
    assert_eq!(
        serde_json::to_vec(&protocol::request(
            1,
            "turn/start",
            turn_params(text.clone())
        ))
        .unwrap()
        .len(),
        MAX_OUTBOUND_JSONL_MESSAGE_BYTES
    );
    text
}

// complete turn/start envelope가 정확히 32MiB이면 전송되고, 첫 excess byte는 peer에
// 도달하기 전에 InputOverBudget로 분류되어 submission retry를 위한 wire 상태를 남기지 않습니다.
#[test]
fn bounds_complete_turn_request_before_peer_send() {
    let text = boundary_text(
        "escaped \\\" text\n\nExplicit skill instructions (yo.skill-instructions/v1): {\\\"name\\\":\\\"review\\\",\\\"instructions\\\":\\\"keep \\\\\\\"quoted\\\"\\\"}",
    );

    let (peer, sent) = FakePeer::new([json!({"id": 1, "result": {}})]);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));
    client
        .call("turn/start", turn_params(text.clone()))
        .expect("the exact complete envelope boundary is accepted");
    assert_eq!(
        serde_json::to_vec(&sent.0.borrow()[0]).unwrap().len(),
        MAX_OUTBOUND_JSONL_MESSAGE_BYTES
    );

    let (peer, sent) = FakePeer::new([json!({"id": 1, "result": {}})]);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));
    let failure = match client.call("turn/start", turn_params(format!("{text}x"))) {
        Ok(_) => panic!("the first byte above the complete request bound is rejected"),
        Err(failure) => failure,
    };
    assert_eq!(failure.kind(), yo_core::BackendFailureKind::InputOverBudget);
    assert!(sent.0.borrow().is_empty());
}

// server request 응답도 같은 complete JSONL boundary를 통과하며, turn input이 아닌
// overflow는 provider protocol failure로 남겨 질문 응답을 허위 제출하지 않습니다.
#[test]
fn bounds_server_responses_as_protocol_failures() {
    let (peer, sent) = FakePeer::new([]);
    let mut client = AppServerClient::new(peer, Duration::from_secs(1));
    let failure = client
        .respond(
            json!(7),
            json!({"answer": "x".repeat(MAX_OUTBOUND_JSONL_MESSAGE_BYTES)}),
        )
        .expect_err("an oversized server response must be rejected before send");
    assert_eq!(failure.kind(), yo_core::BackendFailureKind::Protocol);
    assert!(sent.0.borrow().is_empty());
}
