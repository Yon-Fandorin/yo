use std::{
    env, fs, thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use yo_core::{
    AccountId, ApiCredential, ApiDialect, ConnectorFailureKind, EffectiveModelBinding,
    ModelConnectorCancellation, ModelConnectorEvent, ModelConnectorInputItem,
    ModelConnectorInputRole, ModelConnectorLimits, ModelConnectorPoll, ModelConnectorRequest,
    ModelConnectorTerminal, ModelId, NormalizedEndpoint, ProviderId, RequestToolExposure,
};

use super::local_tls::{LocalServerMode, LocalTlsServer, run_in_tls_child};
use crate::OpenAiResponsesConnector;

fn event(value: serde_json::Value) -> String {
    format!("data: {}\n\n", serde_json::to_string(&value).unwrap())
}

const AUTHORIZATION_SHA256: &str =
    "977c6da970d197c19513744cdc2a995170f8171f4259cb4e8d2433184b71e900";
const UNIQUE_ERROR_BODY: &str = "unique-error-body-7f4d9c2a";

fn loopback_binding(endpoint: &str) -> EffectiveModelBinding {
    EffectiveModelBinding::new(
        ProviderId::new("loopback").unwrap(),
        AccountId::new("test-account").unwrap(),
        ModelId::new("test-model").unwrap(),
        ApiDialect::OpenAiResponses,
        NormalizedEndpoint::parse(endpoint).unwrap(),
    )
}

fn request() -> ModelConnectorRequest {
    ModelConnectorRequest::new(
        vec![ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
            content: "hello".to_owned(),
            refusal: None,
        }],
        RequestToolExposure::disabled(),
        128,
        None,
    )
    .unwrap()
}

fn connector(server: &LocalTlsServer, limits: ModelConnectorLimits) -> OpenAiResponsesConnector {
    let root = env::var_os("YO_MODEL_CONNECTOR_TEST_ROOT")
        .expect("the local TLS child must provide its test root certificate path");
    let root = fs::read(root).expect("the local TLS child must read its test root certificate");
    OpenAiResponsesConnector::new_with_test_root(
        &loopback_binding(server.endpoint()),
        ApiCredential::new("sk-local-characterization").unwrap(),
        limits,
        &root,
    )
    .unwrap()
}

fn terminal_stream() -> Vec<u8> {
    [
        event(json!({
            "type": "response.created",
            "sequence_number": 1,
            "response": {"id": "resp-local"}
        })),
        event(json!({
            "type": "response.completed",
            "sequence_number": 2,
            "response": {"id": "resp-local", "status": "completed", "usage": null}
        })),
    ]
    .concat()
    .into_bytes()
}

fn created_event() -> Vec<u8> {
    event(json!({
        "type": "response.created",
        "sequence_number": 1,
        "response": {"id": "resp-local"}
    }))
    .into_bytes()
}

fn poll_until_closed(
    stream: &mut yo_connector_transport::ConnectorStream,
) -> Vec<ModelConnectorEvent> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut events = Vec::new();
    loop {
        match stream.poll().unwrap() {
            ModelConnectorPoll::Event(event) => events.push(event),
            ModelConnectorPoll::Pending => {
                assert!(Instant::now() < deadline, "stream did not reach Closed");
                thread::yield_now();
            },
            ModelConnectorPoll::Closed => return events,
        }
    }
}

fn poll_until_error(
    stream: &mut yo_connector_transport::ConnectorStream,
    expected_kind: ConnectorFailureKind,
) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match stream.poll() {
            Ok(ModelConnectorPoll::Event(_)) | Ok(ModelConnectorPoll::Pending) => {
                assert!(
                    Instant::now() < deadline,
                    "stream did not reach its failure"
                );
                thread::yield_now();
            },
            Ok(ModelConnectorPoll::Closed) => panic!("stream closed instead of failing"),
            Err(error) => {
                assert_eq!(error.kind(), expected_kind);
                return error.message().to_owned();
            },
        }
    }
}

fn request_body(record: &Value) -> Value {
    serde_json::from_str(record["body"].as_str().unwrap()).unwrap()
}

// 로컬 HTTPS listener가 받은 단 한 번의 POST를 통해 정확한 경로·Bearer 인증·Accept와 JSON
// content type·wire body를 비교하고, HTTP acceptance 뒤의 event와 terminal 및 Closed poll을
// 모두 관찰하여 transport와 worker와 stream 경계의 현재 순서를 고정합니다.
#[test]
fn posts_exact_wire_request_and_emits_events() {
    if run_in_tls_child("tests::transport_lifecycle::posts_exact_wire_request_and_emits_events") {
        return;
    }
    let server = LocalTlsServer::start(LocalServerMode::Success {
        body: terminal_stream(),
        content_type: "text/event-stream; charset=utf-8".to_owned(),
    });
    let connector = connector(&server, ModelConnectorLimits::default());
    let mut stream = connector
        .start(request(), ModelConnectorCancellation::new())
        .unwrap();

    let events = poll_until_closed(&mut stream);
    assert!(matches!(
        events.as_slice(),
        [
            ModelConnectorEvent::ResponseCreated { response_id },
            ModelConnectorEvent::Terminal {
                response_id: terminal_id,
                status: ModelConnectorTerminal::Completed,
                ..
            }
        ] if response_id == "resp-local" && terminal_id == "resp-local"
    ));
    assert_eq!(stream.poll().unwrap(), ModelConnectorPoll::Closed);
    drop(stream);

    let records = server.requests();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record["method"], "POST");
    assert_eq!(record["path"], "/v1/responses");
    assert_eq!(record["authorization_sha256"], AUTHORIZATION_SHA256);
    assert!(
        !serde_json::to_string(record)
            .unwrap()
            .contains("sk-local-characterization")
    );
    assert_eq!(record["headers"]["accept"], "text/event-stream");
    assert_eq!(record["headers"]["content-type"], "application/json");
    assert_eq!(
        request_body(record),
        json!({
            "input": [{"content": "hello", "role": "user"}],
            "max_output_tokens": 128,
            "model": "test-model",
            "stream": true
        })
    );
}

// non-success response body가 상태 오류보다 먼저 profile의 byte bound를 넘으면 body 원문을
// 오류에 싣지 않고 Limit failure로 끝내는 현재 bounded error-body 소비 순서를 고정합니다.
#[test]
fn bounds_a_non_success_body_before_reporting_its_status() {
    if run_in_tls_child(
        "tests::transport_lifecycle::bounds_a_non_success_body_before_reporting_its_status",
    ) {
        return;
    }
    let bounded_server = LocalTlsServer::start(LocalServerMode::Status {
        status: 500,
        body: b"short".to_vec(),
    });
    let bounded_error = connector(&bounded_server, ModelConnectorLimits::default())
        .start(request(), ModelConnectorCancellation::new())
        .unwrap_err();
    assert_eq!(bounded_error.kind(), ConnectorFailureKind::HttpStatus);
    assert!(bounded_error.message().contains("500"));
    assert!(!bounded_error.message().contains("short"));
    assert_eq!(bounded_server.requests().len(), 1);

    let server = LocalTlsServer::start(LocalServerMode::Status {
        status: 500,
        body: UNIQUE_ERROR_BODY.as_bytes().to_vec(),
    });
    let limits = ModelConnectorLimits {
        max_error_body_bytes: 4,
        ..ModelConnectorLimits::default()
    };
    let error = connector(&server, limits)
        .start(request(), ModelConnectorCancellation::new())
        .unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Limit);
    assert!(!error.message().contains(UNIQUE_ERROR_BODY));
    assert!(!error.message().contains("unique-error-body"));
    assert!(!error.message().contains("7f4d9c2a"));
    assert_eq!(server.requests().len(), 1);
}

// same-origin 307 redirect의 두 POST를 모두 local listener에서 확인하고 bearer credential이
// redirected request에도 현재 전달되는지 비교하여 redirect policy와 reqwest forwarding을 함께
// 고정합니다.
#[test]
fn forwards_credentials_across_a_same_origin_redirect() {
    if run_in_tls_child(
        "tests::transport_lifecycle::forwards_credentials_across_a_same_origin_redirect",
    ) {
        return;
    }
    let server = LocalTlsServer::start(LocalServerMode::Redirect {
        location: "/v1/redirected".to_owned(),
        final_body: terminal_stream(),
    });
    let mut stream = connector(&server, ModelConnectorLimits::default())
        .start(request(), ModelConnectorCancellation::new())
        .unwrap();
    let _ = poll_until_closed(&mut stream);
    drop(stream);

    let records = server.requests();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["path"], "/v1/responses");
    assert_eq!(records[1]["path"], "/v1/redirected");
    assert_eq!(records[0]["authorization_sha256"], AUTHORIZATION_SHA256);
    assert_eq!(records[1]["authorization_sha256"], AUTHORIZATION_SHA256);
    assert!(
        !serde_json::to_string(&records)
            .unwrap()
            .contains("sk-local-characterization")
    );
    assert_eq!(request_body(&records[0]), request_body(&records[1]));
}

// 각 307 hop의 header 지연은 개별 limit보다 짧지만 합계는 한 window보다 긴 실제 TLS
// chain에서 세 POST가 모두 완료되어 redirect마다 header clock이 새로 시작하는지 판별합니다.
#[test]
fn each_redirect_attempt_gets_a_fresh_response_header_deadline() {
    if run_in_tls_child(
        "tests::transport_lifecycle::each_redirect_attempt_gets_a_fresh_response_header_deadline",
    ) {
        return;
    }
    let server = LocalTlsServer::start(LocalServerMode::DelayedRedirectChain {
        final_body: terminal_stream(),
        response_delay_millis: 80,
    });
    let limits = ModelConnectorLimits {
        connect_timeout: Duration::from_secs(1),
        response_header_timeout: Duration::from_millis(150),
        ..ModelConnectorLimits::default()
    };
    let started = Instant::now();
    let mut stream = connector(&server, limits)
        .start(request(), ModelConnectorCancellation::new())
        .unwrap();
    let _ = poll_until_closed(&mut stream);
    drop(stream);

    assert!(started.elapsed() >= Duration::from_millis(200));
    let records = server.requests();
    assert_eq!(records.len(), 3);
    assert_eq!(records[0]["path"], "/v1/responses");
    assert_eq!(records[1]["path"], "/v1/redirect-1");
    assert_eq!(records[2]["path"], "/v1/redirect-2");
    assert!(
        records
            .iter()
            .all(|record| record["authorization_sha256"] == AUTHORIZATION_SHA256)
    );
}

// redirect attempt마다 header clock은 새로 열어도 logical request의 optional absolute clock은
// 최초 POST에서 유지되어 누적 chain이 전체 budget을 넘으면 final success 전에 끝납니다.
#[test]
fn redirect_attempts_do_not_reset_the_absolute_request_deadline() {
    if run_in_tls_child(
        "tests::transport_lifecycle::redirect_attempts_do_not_reset_the_absolute_request_deadline",
    ) {
        return;
    }
    let server = LocalTlsServer::start(LocalServerMode::DelayedRedirectChain {
        final_body: terminal_stream(),
        response_delay_millis: 80,
    });
    let limits = ModelConnectorLimits {
        connect_timeout: Duration::from_secs(1),
        response_header_timeout: Duration::from_millis(150),
        absolute_request_timeout: Some(Duration::from_millis(190)),
        ..ModelConnectorLimits::default()
    };
    let started = Instant::now();
    let error = connector(&server, limits)
        .start(request(), ModelConnectorCancellation::new())
        .unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Timeout);
    assert!(error.message().contains("absolute request"));
    assert!(started.elapsed() < Duration::from_millis(400));
    assert!((2..=3).contains(&server.requests().len()));
}

// 다른 loopback port를 별도 origin으로 취급하여 redirect를 거부하고 target listener에는
// 요청이 도착하지 않음을 직접 확인해 bearer credential의 cross-origin forwarding 경계를 고정합니다.
#[test]
fn rejects_a_cross_origin_redirect_before_contacting_the_target() {
    if run_in_tls_child(
        "tests::transport_lifecycle::rejects_a_cross_origin_redirect_before_contacting_the_target",
    ) {
        return;
    }
    let target = LocalTlsServer::start(LocalServerMode::Success {
        body: terminal_stream(),
        content_type: "text/event-stream".to_owned(),
    });
    let source = LocalTlsServer::start(LocalServerMode::Redirect {
        location: format!("{}/responses", target.endpoint()),
        final_body: terminal_stream(),
    });
    let error = connector(&source, ModelConnectorLimits::default())
        .start(request(), ModelConnectorCancellation::new())
        .unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Transport);
    assert!(error.message().contains("redirect"));
    assert_eq!(source.requests().len(), 1);
    assert_eq!(source.accepted_connections(), 1);
    assert_eq!(target.accepted_connections(), 0);
    assert!(target.requests().is_empty());
}

// HTTP 2xx가 반환되어도 media type이 text/event-stream이 아니면 acceptance 전에 Protocol
// failure가 되고 stream worker가 성공으로 보이지 않는 현재 response admission 경계를 고정합니다.
#[test]
fn rejects_a_success_response_with_a_non_sse_content_type() {
    if run_in_tls_child(
        "tests::transport_lifecycle::rejects_a_success_response_with_a_non_sse_content_type",
    ) {
        return;
    }
    let server = LocalTlsServer::start(LocalServerMode::Success {
        body: terminal_stream(),
        content_type: "application/json".to_owned(),
    });
    let error = connector(&server, ModelConnectorLimits::default())
        .start(request(), ModelConnectorCancellation::new())
        .unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Protocol);
    assert!(error.message().contains("text/event-stream"));
    assert_eq!(server.requests().len(), 1);
}

// TLS handshake와 HTTP 요청 수신은 완료되지만 response header를 보내지 않는 listener에서
// response-header deadline이 typed Timeout으로 경계를 종료하는 현재 transport 단계를 고정합니다.
#[test]
fn times_out_waiting_for_response_headers_at_the_header_deadline() {
    if run_in_tls_child(
        "tests::transport_lifecycle::times_out_waiting_for_response_headers_at_the_header_deadline",
    ) {
        return;
    }
    let server = LocalTlsServer::start(LocalServerMode::ResponseHeaderStall);
    let limits = ModelConnectorLimits {
        connect_timeout: Duration::from_secs(1),
        response_header_timeout: Duration::from_millis(100),
        absolute_request_timeout: Some(Duration::from_secs(1)),
        ..ModelConnectorLimits::default()
    };
    let started = Instant::now();
    let error = connector(&server, limits)
        .start(request(), ModelConnectorCancellation::new())
        .unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Timeout);
    assert!(error.message().contains("response-header"));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(server.requests().len(), 1);
    server.wait_for_peer_closed();
}

// loopback TCP 연결은 완료되지만 TLS handshake가 멈춘 listener에서 connect timeout이 HTTP
// request 없이 발생하는 현재 경계를 고정하고, 뒤 phase의 deadline과 혼동하지 않도록 분리합니다.
#[test]
fn times_out_a_stalled_tls_handshake_at_the_connect_deadline() {
    if run_in_tls_child(
        "tests::transport_lifecycle::times_out_a_stalled_tls_handshake_at_the_connect_deadline",
    ) {
        return;
    }
    let server = LocalTlsServer::start(LocalServerMode::TlsHandshakeStall);
    let limits = ModelConnectorLimits {
        connect_timeout: Duration::from_millis(100),
        response_header_timeout: Duration::from_secs(5),
        absolute_request_timeout: Some(Duration::from_secs(5)),
        ..ModelConnectorLimits::default()
    };
    let started = Instant::now();
    let error = connector(&server, limits)
        .start(request(), ModelConnectorCancellation::new())
        .unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Timeout);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(server.accepted_connections(), 1);
    assert!(server.requests().is_empty());
    server.wait_for_peer_closed();
}

// 같은 accepted stream에서 idle phase만 짧게 둔 경우와 optional absolute request만 짧게 둔
// 경우를 각각 관찰하여 두 deadline이 서로 다른 typed Timeout 메시지로 끝나는지 고정합니다.
#[test]
fn distinguishes_stream_idle_and_absolute_request_deadlines() {
    if run_in_tls_child(
        "tests::transport_lifecycle::distinguishes_stream_idle_and_absolute_request_deadlines",
    ) {
        return;
    }
    let idle_server = LocalTlsServer::start(LocalServerMode::EventThenStall {
        body: created_event(),
    });
    let idle_limits = ModelConnectorLimits {
        stream_idle_timeout: Duration::from_millis(100),
        ..ModelConnectorLimits::default()
    };
    let mut idle_stream = connector(&idle_server, idle_limits)
        .start(request(), ModelConnectorCancellation::new())
        .unwrap();
    idle_server.wait_for_response_sent();
    assert_eq!(idle_server.requests().len(), 1);
    let idle_message = poll_until_error(&mut idle_stream, ConnectorFailureKind::Timeout);
    assert!(idle_message.contains("stream idle"));
    drop(idle_stream);
    idle_server.wait_for_peer_closed();

    let total_server = LocalTlsServer::start(LocalServerMode::EventThenStall {
        body: created_event(),
    });
    let total_limits = ModelConnectorLimits {
        stream_idle_timeout: Duration::from_secs(1),
        absolute_request_timeout: Some(Duration::from_millis(500)),
        ..ModelConnectorLimits::default()
    };
    let mut total_stream = connector(&total_server, total_limits)
        .start(request(), ModelConnectorCancellation::new())
        .unwrap();
    total_server.wait_for_response_sent();
    assert_eq!(total_server.requests().len(), 1);
    let total_message = poll_until_error(&mut total_stream, ConnectorFailureKind::Timeout);
    assert!(total_message.contains("absolute request"));
    drop(total_stream);
    total_server.wait_for_peer_closed();
}

// non-success header 뒤 일부 body를 보낸 채 멈추는 실제 HTTPS peer에서 성공 stream의 idle
// 설정과 분리된 error-body inactivity가 시작되고 phase 이름을 보존하는지 관찰합니다.
#[test]
fn times_out_a_stalled_error_body_at_its_own_inactivity_deadline() {
    if run_in_tls_child(
        "tests::transport_lifecycle::times_out_a_stalled_error_body_at_its_own_inactivity_deadline",
    ) {
        return;
    }
    let server = LocalTlsServer::start(LocalServerMode::ErrorBodyThenStall {
        status: 500,
        body: b"bounded partial error".to_vec(),
    });
    let limits = ModelConnectorLimits {
        stream_idle_timeout: Duration::from_secs(2),
        error_body_idle_timeout: Duration::from_millis(100),
        ..ModelConnectorLimits::default()
    };
    let started = Instant::now();
    let error = connector(&server, limits)
        .start(request(), ModelConnectorCancellation::new())
        .unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Timeout);
    assert!(error.message().contains("error-body idle"));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(server.requests().len(), 1);
    server.wait_for_peer_closed();
}

// semantic SSE event가 완성되지 않아도 실제 HTTPS peer의 non-empty heartbeat/comment raw
// chunk가 도착할 때마다 successful-stream inactivity가 연장되는지 누적 시간으로 판별합니다.
#[test]
fn non_empty_raw_heartbeats_reset_successful_stream_inactivity() {
    if run_in_tls_child(
        "tests::transport_lifecycle::non_empty_raw_heartbeats_reset_successful_stream_inactivity",
    ) {
        return;
    }
    let server = LocalTlsServer::start(LocalServerMode::HeartbeatsThenStall);
    let limits = ModelConnectorLimits {
        stream_idle_timeout: Duration::from_millis(90),
        ..ModelConnectorLimits::default()
    };
    let started = Instant::now();
    let mut stream = connector(&server, limits)
        .start(request(), ModelConnectorCancellation::new())
        .unwrap();
    server.wait_for_response_sent();

    let message = poll_until_error(&mut stream, ConnectorFailureKind::Timeout);
    assert!(message.contains("stream idle"));
    assert!(started.elapsed() >= Duration::from_millis(180));
    drop(stream);
    server.wait_for_peer_closed();
}

// non-empty heartbeat가 stream inactivity를 연장하는 동안에도 optional absolute request clock은
// 한 번 시작한 뒤 reset되지 않아 더 짧은 전체 work budget이 정확한 phase로 끝나는지 검증합니다.
#[test]
fn raw_heartbeats_do_not_reset_an_absolute_request_deadline() {
    if run_in_tls_child(
        "tests::transport_lifecycle::raw_heartbeats_do_not_reset_an_absolute_request_deadline",
    ) {
        return;
    }
    let server = LocalTlsServer::start(LocalServerMode::HeartbeatsThenStall);
    let limits = ModelConnectorLimits {
        stream_idle_timeout: Duration::from_secs(1),
        absolute_request_timeout: Some(Duration::from_millis(130)),
        ..ModelConnectorLimits::default()
    };
    let started = Instant::now();
    let mut stream = connector(&server, limits)
        .start(request(), ModelConnectorCancellation::new())
        .unwrap();
    server.wait_for_response_sent();

    let message = poll_until_error(&mut stream, ConnectorFailureKind::Timeout);
    assert!(message.contains("absolute request"));
    assert!(started.elapsed() < Duration::from_millis(400));
    drop(stream);
    server.wait_for_peer_closed();
}

// acceptance 후 cancellation은 pending read를 깨우고, shutdown은 같은 worker를 join하며,
// Drop도 bounded 시간 안에 cancellation·join을 수행하는지 세 가지 공용 cleanup 경로로 고정합니다.
#[test]
fn cancels_and_joins_workers_from_cancel_shutdown_and_drop() {
    if run_in_tls_child(
        "tests::transport_lifecycle::cancels_and_joins_workers_from_cancel_shutdown_and_drop",
    ) {
        return;
    }
    let cancel_server = LocalTlsServer::start(LocalServerMode::EventThenStall {
        body: created_event(),
    });
    let cancellation = ModelConnectorCancellation::new();
    let mut cancelled_stream = connector(&cancel_server, ModelConnectorLimits::default())
        .start(request(), cancellation.clone())
        .unwrap();
    cancel_server.wait_for_response_sent();
    assert_eq!(cancel_server.requests().len(), 1);
    cancellation.cancel();
    let _ = poll_until_error(&mut cancelled_stream, ConnectorFailureKind::Cancelled);
    drop(cancelled_stream);
    cancel_server.wait_for_peer_closed();

    let shutdown_server = LocalTlsServer::start(LocalServerMode::HeadersThenStall {
        content_type: "text/event-stream".to_owned(),
    });
    let shutdown_cancellation = ModelConnectorCancellation::new();
    let mut shutdown_stream = connector(&shutdown_server, ModelConnectorLimits::default())
        .start(request(), shutdown_cancellation.clone())
        .unwrap();
    shutdown_server.wait_for_response_sent();
    assert_eq!(shutdown_server.requests().len(), 1);
    assert_eq!(shutdown_stream.poll().unwrap(), ModelConnectorPoll::Pending);
    shutdown_stream.shutdown().unwrap();
    assert!(shutdown_cancellation.is_cancelled());
    shutdown_server.wait_for_peer_closed();

    let drop_server = LocalTlsServer::start(LocalServerMode::EventThenStall {
        body: created_event(),
    });
    let drop_stream = connector(&drop_server, ModelConnectorLimits::default())
        .start(request(), ModelConnectorCancellation::new())
        .unwrap();
    drop_server.wait_for_response_sent();
    assert_eq!(drop_server.requests().len(), 1);
    let started = Instant::now();
    drop(drop_stream);
    assert!(started.elapsed() < Duration::from_secs(2));
    drop_server.wait_for_peer_closed();
}
