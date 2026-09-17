use std::{env, fs};

use reqwest::{Client, redirect, retry};
use serde_json::json;
use tokio::runtime::Builder;
use yo_core::{ApiCredential, NormalizedEndpoint};
use yo_test_support::local_tls::{LocalServerMode, LocalTlsServer, run_in_tls_child};

use super::{
    super::{decode::parse_kimi_account_plan, http::fetch},
    usage_payload,
};

// local TLS listener로 exact GET `/v1/usages`, JSON Accept, 단 한 연결과 Bearer
// credential 비노출을 함께 관찰합니다.
#[test]
fn fetches_one_authenticated_usage_snapshot_over_local_tls() {
    if run_in_tls_child(
        "account_capacity::tests::transport::fetches_one_authenticated_usage_snapshot_over_local_tls",
    ) {
        return;
    }
    let body = usage_payload();
    let server = LocalTlsServer::start(LocalServerMode::Success {
        body: body.clone(),
        content_type: "application/json; charset=utf-8".to_owned(),
    });
    let root = env::var_os("YO_MODEL_CONNECTOR_TEST_ROOT").unwrap();
    let roots = reqwest::Certificate::from_pem_bundle(&fs::read(root).unwrap()).unwrap();
    let client = Client::builder()
        .add_root_certificate(roots[0].clone())
        .redirect(redirect::Policy::none())
        .retry(retry::never())
        .build()
        .unwrap();
    let url = NormalizedEndpoint::parse(server.endpoint())
        .unwrap()
        .append_path_segment("usages")
        .unwrap();
    let received = Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(fetch(
            &client,
            url,
            &ApiCredential::new("sentinel-kimi-usage-key").unwrap(),
        ))
        .unwrap();
    assert_eq!(received, body);
    server.wait_for_response_sent();
    let requests = server.requests();
    assert_eq!(server.accepted_connections(), 1);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["method"], "GET");
    assert_eq!(requests[0]["path"], "/v1/usages");
    assert_eq!(requests[0]["headers"]["accept"], "application/json");
    assert!(requests[0].get("authorization").is_none());
    assert!(requests[0]["authorization_sha256"].is_string());
    assert!(
        !serde_json::to_string(&requests)
            .unwrap()
            .contains("sentinel-kimi-usage-key")
    );
}

// Kimi Code 계정 플랜 조회도 usage와 같은 no-redirect·no-retry Bearer 경계를
// 사용하며 exact GET `/v1/me` 응답만 profile parser에 전달합니다.
#[test]
fn fetches_one_authenticated_account_profile_over_local_tls() {
    if run_in_tls_child(
        "account_capacity::tests::transport::fetches_one_authenticated_account_profile_over_local_tls",
    ) {
        return;
    }
    let body = serde_json::to_vec(&json!({"user_level_name": "Moderato"})).unwrap();
    let server = LocalTlsServer::start(LocalServerMode::Success {
        body: body.clone(),
        content_type: "application/json".to_owned(),
    });
    let root = env::var_os("YO_MODEL_CONNECTOR_TEST_ROOT").unwrap();
    let roots = reqwest::Certificate::from_pem_bundle(&fs::read(root).unwrap()).unwrap();
    let client = Client::builder()
        .add_root_certificate(roots[0].clone())
        .redirect(redirect::Policy::none())
        .retry(retry::never())
        .build()
        .unwrap();
    let url = NormalizedEndpoint::parse(server.endpoint())
        .unwrap()
        .append_path_segment("me")
        .unwrap();
    let received = Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(fetch(
            &client,
            url,
            &ApiCredential::new("sentinel-kimi-profile-key").unwrap(),
        ))
        .unwrap();
    assert_eq!(parse_kimi_account_plan(&received).unwrap(), "Moderato");
    server.wait_for_response_sent();
    let requests = server.requests();
    assert_eq!(server.accepted_connections(), 1);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["method"], "GET");
    assert_eq!(requests[0]["path"], "/v1/me");
    assert_eq!(requests[0]["headers"]["accept"], "application/json");
    assert!(requests[0].get("authorization").is_none());
    assert!(requests[0]["authorization_sha256"].is_string());
    assert!(
        !serde_json::to_string(&requests)
            .unwrap()
            .contains("sentinel-kimi-profile-key")
    );
}
