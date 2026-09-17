use std::{env, fs, str};

use reqwest::{Client, header, redirect, retry};
use tokio::runtime::Builder;
use yo_core::ApiCredential;
use yo_test_support::local_tls::{LocalServerMode, LocalTlsServer, run_in_tls_child};

use super::super::{
    http::{build_gateway_request, fetch_bounded},
    model::ExpectedMedia,
};

// Gateway request는 QwenCloud identity와 personal Token Plan API를 exact query·form으로
// 보내고 console cookie는 header에만 두어 URL이나 payload에 복제하지 않습니다.
#[test]
fn builds_exact_qwencloud_gateway_request() {
    let client = Client::builder().build().unwrap();
    let cookie =
        ApiCredential::new("cna=x; login_qwencloud_ticket=google-session; auxiliary=1").unwrap();
    let sec_token = ApiCredential::new("sec-token").unwrap();
    let request = build_gateway_request(&client, &cookie, &sec_token, "usage")
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(request.method(), reqwest::Method::POST);
    assert_eq!(request.url().host_str(), Some("cs-data.qwencloud.com"));
    let query = request.url().query_pairs().collect::<Vec<_>>();
    assert!(query
        .iter()
        .any(|(key, value)| key == "api" && value.ends_with("/tokenplan/personal/api/v2/usage")));
    assert_eq!(
        request.headers().get(header::ORIGIN).unwrap(),
        "https://home.qwencloud.com"
    );
    assert_eq!(
        request.headers().get(header::COOKIE).unwrap(),
        cookie.expose_secret()
    );
    let body = request.body().and_then(reqwest::Body::as_bytes).unwrap();
    let body = str::from_utf8(body).unwrap();
    assert!(body.contains("product=sfm_bailian"));
    assert!(body.contains("action=IntlBroadScopeAspnGateway"));
    assert!(body.contains("region=ap-southeast-1"));
    assert!(body.contains("sec_token=sec-token"));
    assert!(!body.contains("google-session"));
}

fn fetch_from_local_tls(
    server: &LocalTlsServer,
    expected_media: ExpectedMedia,
) -> Result<Vec<u8>, super::super::QwenCloudCapacityError> {
    let root = env::var_os("YO_MODEL_CONNECTOR_TEST_ROOT")
        .expect("local TLS child가 root certificate를 제공해야 합니다");
    let roots = reqwest::Certificate::from_pem_bundle(&fs::read(root).unwrap()).unwrap();
    let client = Client::builder()
        .add_root_certificate(roots[0].clone())
        .redirect(redirect::Policy::none())
        .retry(retry::never())
        .build()
        .unwrap();
    let request = client
        .get(server.endpoint())
        .header(header::ACCEPT, "application/json");
    Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(fetch_bounded(request, expected_media))
}

// 실제 local TLS child를 통해 bounded fetch가 JSON media type과 응답 body를 그대로
// 소비하고, fixture가 관찰한 request가 한 번뿐인지 확인합니다.
#[test]
fn fetches_a_json_capacity_response_over_local_tls() {
    if run_in_tls_child(
        "account_capacity::tests::transport::fetches_a_json_capacity_response_over_local_tls",
    ) {
        return;
    }
    let body = br#"{"capacity":"ok"}"#.to_vec();
    let server = LocalTlsServer::start(LocalServerMode::Success {
        body: body.clone(),
        content_type: "application/json; charset=utf-8".to_owned(),
    });
    assert_eq!(
        fetch_from_local_tls(&server, ExpectedMedia::Json).unwrap(),
        body
    );
    server.wait_for_response_sent();
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["method"], "GET");
    assert_eq!(requests[0]["path"], "/v1");
    assert_eq!(requests[0]["headers"]["accept"], "application/json");
}

// console redirect는 일반 HTTP 상태가 아니라 browser session 만료로 분류하여 호출자가
// 새 Cookie를 입력하도록 하고, redirect 대상에 request를 재전송하지 않습니다.
#[test]
fn classifies_a_console_redirect_as_an_expired_session() {
    if run_in_tls_child(
        "account_capacity::tests::transport::classifies_a_console_redirect_as_an_expired_session",
    ) {
        return;
    }
    let server = LocalTlsServer::start(LocalServerMode::Redirect {
        location: "/login".to_owned(),
        final_body: Vec::new(),
    });
    let error = fetch_from_local_tls(&server, ExpectedMedia::Json).unwrap_err();
    assert_eq!(
        error.kind(),
        super::super::QwenCloudCapacityFailureKind::ExpiredSession
    );
    assert_eq!(server.requests().len(), 1);
}

// Content-Length가 선언한 1 MiB 초과와 framing 없는 streaming body의 1 MiB 초과를
// 각각 관찰하여 bounded transport가 두 입력 경로를 모두 차단하는지 고정합니다.
#[test]
fn rejects_declared_and_streamed_oversize_capacity_responses() {
    if run_in_tls_child(
        "account_capacity::tests::transport::rejects_declared_and_streamed_oversize_capacity_responses",
    ) {
        return;
    }
    let declared = LocalTlsServer::start(LocalServerMode::DeclaredOversize);
    let declared_error = fetch_from_local_tls(&declared, ExpectedMedia::Json).unwrap_err();
    assert_eq!(
        declared_error.kind(),
        super::super::QwenCloudCapacityFailureKind::Limit
    );
    declared.wait_for_response_sent();

    let streamed = LocalTlsServer::start(LocalServerMode::UnframedSuccess {
        body: vec![b'x'; 1024 * 1024 + 1],
        content_type: "application/json".to_owned(),
    });
    let streamed_error = fetch_from_local_tls(&streamed, ExpectedMedia::Json).unwrap_err();
    assert_eq!(
        streamed_error.kind(),
        super::super::QwenCloudCapacityFailureKind::Limit
    );
    streamed.wait_for_response_sent();
}
