use reqwest::{StatusCode, Url};

use super::super::{
    failure::{
        cancelled_failure, http_status_failure, limit_failure, protocol_failure, transport_failure,
    },
    transport::{Origin, http_client_with_test_root, validate_redirect},
};
use crate::model_connector::{ConnectorFailureKind, ResponsesConnectorLimits};

// fixture root가 PEM certificate가 아니면 network worker를 시작하기 전에 test-only client
// 구성 경계에서 secret-free Configuration 진단으로 실패하는지 고정합니다.
#[test]
fn rejects_an_invalid_local_tls_fixture_root_before_network_work() {
    let error = http_client_with_test_root(
        &Url::parse("https://127.0.0.1:443/v1/responses").unwrap(),
        &ResponsesConnectorLimits::default(),
        b"invalid-local-root-private-sentinel",
    )
    .unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Configuration);
    assert!(error.message().contains("local-TLS fixture"));
    assert!(error.message().contains("root certificate"));
    assert!(!error.message().contains("private-sentinel"));
}

// 인증·rate-limit·server 오류는 상태 코드만 보존하고 민감한 body는 진단에 싣지 않는다.
#[test]
fn http_failures_preserve_only_the_status_code() {
    for (status, expected) in [
        (
            StatusCode::UNAUTHORIZED,
            crate::ModelRequestFailureKind::Authentication,
        ),
        (
            StatusCode::FORBIDDEN,
            crate::ModelRequestFailureKind::AccessDenied,
        ),
        (
            StatusCode::NOT_FOUND,
            crate::ModelRequestFailureKind::RequestRejected,
        ),
        (
            StatusCode::REQUEST_TIMEOUT,
            crate::ModelRequestFailureKind::Timeout,
        ),
        (
            StatusCode::TOO_MANY_REQUESTS,
            crate::ModelRequestFailureKind::RateLimited,
        ),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            crate::ModelRequestFailureKind::RequestRejected,
        ),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            crate::ModelRequestFailureKind::ProviderUnavailable,
        ),
    ] {
        let error = http_status_failure(status);
        assert_eq!(error.kind(), ConnectorFailureKind::HttpStatus);
        assert_eq!(error.request_failure_kind(), Some(expected));
        assert_eq!(
            error.to_string(),
            format!(
                "HttpStatus: model-connector HTTP request returned status {}",
                status.as_u16()
            )
        );
        assert!(!error.to_string().contains("prompt"));
        assert!(!error.to_string().contains("credential"));
    }
}

// request outcome 분류는 typed connector 경계에서만 정해지고 cancellation·cleanup은
// 관찰 대상이 아니며 body text나 진단 문자열로 더 구체적인 kind를 추론하지 않습니다.
#[test]
fn connector_failure_kinds_have_closed_request_observation_mapping() {
    assert_eq!(
        transport_failure("private-body-sentinel").request_failure_kind(),
        Some(crate::ModelRequestFailureKind::Transport)
    );
    assert_eq!(
        protocol_failure("private-body-sentinel").request_failure_kind(),
        Some(crate::ModelRequestFailureKind::Protocol)
    );
    assert_eq!(
        limit_failure("private-body-sentinel").request_failure_kind(),
        Some(crate::ModelRequestFailureKind::ResponseLimit)
    );
    assert_eq!(cancelled_failure().request_failure_kind(), None);
    assert_eq!(
        http_status_failure(StatusCode::EARLY_HINTS).request_failure_kind(),
        Some(crate::ModelRequestFailureKind::Protocol)
    );
}

// redirect는 scheme·host·effective port가 모두 같은 origin이고 profile 횟수 안일 때만
// 허용하여 다른 origin으로 bearer credential이나 request body가 전달되지 않습니다.
#[test]
fn permits_only_bounded_same_origin_redirects() {
    let original = Url::parse("https://example.com/v1/responses").unwrap();
    let origin = Origin::from_url(&original).unwrap();

    assert!(
        validate_redirect(
            &origin,
            &Url::parse("https://example.com:443/next").unwrap(),
            1,
            3,
        )
        .is_ok()
    );
    assert!(
        validate_redirect(
            &origin,
            &Url::parse("https://other.example/next").unwrap(),
            1,
            3,
        )
        .is_err()
    );
    assert!(validate_redirect(&origin, &original, 3, 3).is_err());
}

// 같은 origin이어도 user information·query·fragment가 붙은 redirect는 normalized
// HTTPS target이 아니므로 bearer request를 따라가지 않는지 각각 검증합니다.
#[test]
fn rejects_non_normalized_same_origin_redirect_targets() {
    let original = Url::parse("https://example.com/v1/responses").unwrap();
    let origin = Origin::from_url(&original).unwrap();

    for target in [
        "https://user@example.com/next",
        "https://example.com/next?trace=1",
        "https://example.com/next#fragment",
    ] {
        assert!(validate_redirect(&origin, &Url::parse(target).unwrap(), 0, 3).is_err());
    }
}
