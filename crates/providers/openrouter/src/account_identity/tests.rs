use std::{env, fs};

use serde_json::json;
use tokio::runtime::Builder;
use yo_core::{ApiCredential, NormalizedEndpoint};
use yo_test_support::local_tls::{LocalServerMode, LocalTlsServer, run_in_tls_child};

use super::*;

fn fetch_from_local_tls(server: &LocalTlsServer) -> Result<String, OpenRouterAccountIdentityError> {
    let root = env::var_os("YO_MODEL_CONNECTOR_TEST_ROOT")
        .expect("the local TLS child must provide its root certificate");
    let roots = reqwest::Certificate::from_pem_bundle(&fs::read(root).unwrap()).unwrap();
    let client = current_key_client()
        .add_root_certificate(roots[0].clone())
        .build()
        .unwrap();
    let endpoint = NormalizedEndpoint::parse(server.endpoint()).unwrap();
    let url = endpoint.append_path_segment("key").unwrap();
    let credential = ApiCredential::new("sentinel-current-key-secret").unwrap();
    let bytes = Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(fetch_current_key(&client, url, &credential))?;
    parse_current_key(&bytes)
}

// 실제 TLS 요청에서 현재 키 경로, Bearer 전송 및 공개 사용자 ID 추출을 함께 확인합니다.
#[test]
fn observes_current_key_identity_over_local_tls() {
    if run_in_tls_child("account_identity::tests::observes_current_key_identity_over_local_tls") {
        return;
    }
    let server = LocalTlsServer::start(LocalServerMode::Success {
        body: json!({"data": {
            "creator_user_id": "user_2dHFtVWx",
            "organization_id": null,
            "workspace_id": "0df9e665-d932-5740-b2c7-b52af166bc11",
            "label": "masked"
        }})
        .to_string()
        .into_bytes(),
        content_type: "application/json; charset=utf-8".to_owned(),
    });
    assert_eq!(
        fetch_from_local_tls(&server).unwrap(),
        "openrouter-account/v1|creator=user_2dHFtVWx|organization=none|workspace=0df9e665-d932-5740-b2c7-b52af166bc11"
    );
    server.wait_for_response_sent();
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["method"], "GET");
    assert_eq!(requests[0]["path"], "/v1/key");
    assert!(requests[0].get("authorization").is_none());
    assert_eq!(
        requests[0]["authorization_sha256"],
        "48a77667970b02d76682a1d12544d76559939b75d1c121e3086d4c15ae13f092"
    );
    assert_eq!(requests[0]["body"], "");
}

// 인증 실패와 redirect는 응답 본문을 노출하거나 두 번째 origin에 Bearer를 보내지 않습니다.
#[test]
fn rejects_status_and_redirect_without_retransmission() {
    if run_in_tls_child(
        "account_identity::tests::rejects_status_and_redirect_without_retransmission",
    ) {
        return;
    }
    let denied = LocalTlsServer::start(LocalServerMode::Status {
        status: 401,
        body: b"private-provider-error".to_vec(),
    });
    let error = fetch_from_local_tls(&denied).unwrap_err();
    assert_eq!(
        error.kind(),
        OpenRouterAccountIdentityFailureKind::HttpStatus
    );
    assert!(!error.to_string().contains("private-provider-error"));

    let redirect = LocalTlsServer::start(LocalServerMode::Redirect {
        location: "https://127.0.0.1:9/foreign".to_owned(),
        final_body: Vec::new(),
    });
    let error = fetch_from_local_tls(&redirect).unwrap_err();
    assert_eq!(
        error.kind(),
        OpenRouterAccountIdentityFailureKind::HttpStatus
    );
    assert_eq!(redirect.accepted_connections(), 1);
    assert_eq!(redirect.requests().len(), 1);
}

// JSON이 아닌 성공 응답과 선언되지 않은 streaming 초과 body는 인증 증거로 수용하지 않습니다.
#[test]
fn rejects_non_json_and_oversized_streaming_response() {
    if run_in_tls_child(
        "account_identity::tests::rejects_non_json_and_oversized_streaming_response",
    ) {
        return;
    }
    let html = LocalTlsServer::start(LocalServerMode::Success {
        body: b"<html>not a key</html>".to_vec(),
        content_type: "text/html".to_owned(),
    });
    assert_eq!(
        fetch_from_local_tls(&html).unwrap_err().kind(),
        OpenRouterAccountIdentityFailureKind::MediaType
    );

    let oversized = LocalTlsServer::start(LocalServerMode::UnframedSuccess {
        body: vec![b'x'; MAX_RESPONSE_BYTES + 1],
        content_type: "application/json".to_owned(),
    });
    assert_eq!(
        fetch_from_local_tls(&oversized).unwrap_err().kind(),
        OpenRouterAccountIdentityFailureKind::Limit
    );
}

// 같은 생성자의 다른 조직·작업공간은 서로 다른 저장 대상이며 응답 필드 순서는 무관합니다.
#[test]
fn binds_creator_organization_and_workspace_without_aliases() {
    let one = br#"{"data":{"creator_user_id":"user_abc-123","organization_id":"org_1","workspace_id":"0df9e665-d932-5740-b2c7-b52af166bc11"}}"#;
    let another_workspace = br#"{"data":{"workspace_id":"1df9e665-d932-5740-b2c7-b52af166bc11","organization_id":"org_1","creator_user_id":"user_abc-123"}}"#;
    let another_organization = br#"{"data":{"creator_user_id":"user_abc-123","organization_id":"org_2","workspace_id":"0df9e665-d932-5740-b2c7-b52af166bc11"}}"#;
    let no_organization = br#"{"data":{"creator_user_id":"user_abc-123","organization_id":null,"workspace_id":"0df9e665-d932-5740-b2c7-b52af166bc11"}}"#;
    let same_workspace_uppercase = br#"{"data":{"creator_user_id":"user_abc-123","organization_id":"org_1","workspace_id":"0DF9E665-D932-5740-B2C7-B52AF166BC11"}}"#;
    let first = parse_current_key(one).unwrap();
    assert_eq!(
        first,
        "openrouter-account/v1|creator=user_abc-123|organization=some:org_1|workspace=0df9e665-d932-5740-b2c7-b52af166bc11"
    );
    assert_ne!(first, parse_current_key(another_workspace).unwrap());
    assert_ne!(first, parse_current_key(another_organization).unwrap());
    assert_ne!(first, parse_current_key(no_organization).unwrap());
    assert_eq!(first, parse_current_key(same_workspace_uppercase).unwrap());
}

// 필수 경로의 누락·중복·비문자열과 불명확한 조직·작업공간 식별자를 거절합니다.
#[test]
fn rejects_incomplete_or_ambiguous_account_identity() {
    for invalid in [
        br#"{}"#.as_slice(),
        br#"{"data":{}}"#,
        br#"{"data":{"creator_user_id":null,"organization_id":null,"workspace_id":"0df9e665-d932-5740-b2c7-b52af166bc11"}}"#,
        br#"{"data":{"creator_user_id":"user_1","workspace_id":"0df9e665-d932-5740-b2c7-b52af166bc11"}}"#,
        br#"{"data":{"creator_user_id":"user_1","organization_id":null}}"#,
        br#"{"data":{"creator_user_id":"user_1","organization_id":17,"workspace_id":"0df9e665-d932-5740-b2c7-b52af166bc11"}}"#,
        br#"{"data":{"creator_user_id":"user_1","organization_id":null,"workspace_id":"workspace-a"}}"#,
        br#"{"data":{"creator_user_id":"user/name","organization_id":null,"workspace_id":"0df9e665-d932-5740-b2c7-b52af166bc11"}}"#,
        br#"{"data":{"creator_user_id":"user_1","creator_user_id":"user_2","organization_id":null,"workspace_id":"0df9e665-d932-5740-b2c7-b52af166bc11"}}"#,
        br#"{"data":{"creator_user_id":"user_1","organization_id":null,"organization_id":"org_2","workspace_id":"0df9e665-d932-5740-b2c7-b52af166bc11"}}"#,
        br#"{"data":{"creator_user_id":"user_1","organization_id":null,"workspace_id":"0df9e665-d932-5740-b2c7-b52af166bc11"},"data":{"creator_user_id":"user_2","organization_id":null,"workspace_id":"0df9e665-d932-5740-b2c7-b52af166bc11"}}"#,
    ] {
        assert_eq!(
            parse_current_key(invalid).unwrap_err().kind(),
            OpenRouterAccountIdentityFailureKind::Protocol
        );
    }
    let too_long = format!(
        "{{\"data\":{{\"creator_user_id\":\"{}\",\"organization_id\":null,\"workspace_id\":\"0df9e665-d932-5740-b2c7-b52af166bc11\"}}}}",
        "u".repeat(MAX_ACCOUNT_COMPONENT_BYTES + 1)
    );
    assert_eq!(
        parse_current_key(too_long.as_bytes()).unwrap_err().kind(),
        OpenRouterAccountIdentityFailureKind::Protocol
    );
}
