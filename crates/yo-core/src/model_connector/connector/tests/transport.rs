use reqwest::{StatusCode, Url};

use super::super::{
    failure::http_status_failure,
    transport::{Origin, validate_redirect},
};
use crate::model_connector::ConnectorFailureKind;

// 인증·rate-limit·server 오류는 상태 코드만 보존하고 민감한 body는 진단에 싣지 않는다.
#[test]
fn http_failures_preserve_only_the_status_code() {
    for status in [
        StatusCode::UNAUTHORIZED,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::INTERNAL_SERVER_ERROR,
    ] {
        let error = http_status_failure(status);
        assert_eq!(error.kind(), ConnectorFailureKind::HttpStatus);
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
