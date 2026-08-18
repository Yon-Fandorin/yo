use std::{env, fs};

use reqwest::{Client, redirect};
use serde_json::json;

use super::{
    KimiCatalogAvailability, KimiCatalogDisabledReason, KimiCatalogSeed,
    normalize::normalize_catalog,
};
use crate::{
    AccountId, ProviderId, VersionedProfileId,
    model_connector::tests::local_tls::{LocalServerMode, LocalTlsServer, run_in_tls_child},
};

fn seed() -> KimiCatalogSeed {
    KimiCatalogSeed::resolve(
        VersionedProfileId::new("kimi-platform-ai/v1").unwrap(),
        ProviderId::new("kimi").unwrap(),
        AccountId::new("default").unwrap(),
        None,
        None,
    )
    .unwrap()
}

fn code_seed() -> KimiCatalogSeed {
    KimiCatalogSeed::resolve(
        VersionedProfileId::new("kimi-code-membership/v1").unwrap(),
        ProviderId::new("kimi").unwrap(),
        AccountId::new("code").unwrap(),
        None,
        None,
    )
    .unwrap()
}

// 인증 inventory의 네 개 reviewed ModelId만 exact overlay로 실행 가능해지고, K3/K2.7은
// private replay를, K2.6은 semantic-only replay를 가진 완전한 binding으로 해석됩니다.
#[test]
fn reviewed_rows_resolve_exact_profiles_and_badges() {
    let rows = json!({
        "object": "list",
        "data": [
            {"object":"model","id":"kimi-k3","context_length":1048576},
            {"object":"model","id":"kimi-k2.7-code","context_length":262144},
            {"object":"model","id":"kimi-k2.7-code-highspeed","context_length":262144},
            {"object":"model","id":"kimi-k2.6","context_length":262144,"supports_reasoning":false}
        ]
    });
    let models = normalize_catalog(&seed(), &serde_json::to_vec(&rows).unwrap()).unwrap();
    assert_eq!(models.len(), 4);
    for model in &models {
        assert!(model.is_enabled(), "{}", model.model_id());
        let complete = model.entry().unwrap().complete_binding().unwrap();
        assert_eq!(
            complete.binding().connector_id().as_str(),
            "kimi-chat-completions"
        );
        let expected = if model.model_id().as_str() == "kimi-k2.6" {
            "semantic-only/v1"
        } else {
            "kimi-private-local-plaintext/v1"
        };
        assert_eq!(complete.profile().replay_profile().as_str(), expected);
    }
    let k3 = models
        .iter()
        .find(|model| model.model_id().as_str() == "kimi-k3")
        .unwrap();
    assert!(k3.recommended());
    let fast = models
        .iter()
        .find(|model| model.model_id().as_str() == "kimi-k2.7-code-highspeed")
        .unwrap();
    assert!(fast.high_speed());
}

// Code membership inventory는 Platform ModelId를 재사용하지 않고 네 exact Code ModelId만
// 완전한 binding으로 만들며, k3-256k와 high-speed 표시를 원격 행 순서와 무관하게 보존합니다.
#[test]
fn code_membership_rows_resolve_the_separate_endpoint_and_profiles() {
    let rows = json!({
        "object": "list",
        "data": [
            {"object":"model","id":"k3","context_length":262144},
            {"object":"model","id":"k3-256k","context_length":262144},
            {"object":"model","id":"kimi-for-coding","context_length":262144},
            {"object":"model","id":"kimi-for-coding-highspeed","context_length":262144}
        ]
    });
    let models = normalize_catalog(&code_seed(), &serde_json::to_vec(&rows).unwrap()).unwrap();
    assert_eq!(models.len(), 4);
    for model in &models {
        assert!(model.is_enabled(), "{}", model.model_id());
        let complete = model.entry().unwrap().complete_binding().unwrap();
        assert_eq!(
            complete.binding().endpoint().as_str(),
            "https://api.kimi.com/coding/v1"
        );
        assert_eq!(
            complete.profile().replay_profile().as_str(),
            "kimi-private-local-plaintext/v1"
        );
    }
    let recommended = models
        .iter()
        .find(|model| model.model_id().as_str() == "k3-256k")
        .unwrap();
    assert!(recommended.recommended());
    let fast = models
        .iter()
        .find(|model| model.model_id().as_str() == "kimi-for-coding-highspeed")
        .unwrap();
    assert!(fast.high_speed());
}

// Code k3의 1,048,576 context는 hard max 131,072인 complete binding으로 활성화하지만,
// k3-256k에 같은 1M context를 붙이거나 Platform ModelId를 교차하면 inventory에만 남기고
// 선택 불가능하게 해 reviewed product 경계를 추론하지 않습니다.
#[test]
fn code_membership_keeps_membership_range_and_cross_product_fail_closed() {
    let rows = json!({
        "object": "list",
        "data": [
            {"object":"model","id":"k3","context_length":1048576},
            {"object":"model","id":"k3-256k","context_length":1048576},
            {"object":"model","id":"kimi-k3","context_length":1048576}
        ]
    });
    let models = normalize_catalog(&code_seed(), &serde_json::to_vec(&rows).unwrap()).unwrap();
    let k3 = models
        .iter()
        .find(|model| model.model_id().as_str() == "k3")
        .unwrap();
    assert!(k3.is_enabled());
    let profile = k3.entry().unwrap().complete_binding().unwrap().profile();
    assert_eq!(profile.context().input_token_limit(), 1_048_576);
    assert_eq!(profile.context().max_output_tokens(), Some(131_072));
    for id in ["k3-256k", "kimi-k3"] {
        assert_eq!(
            models
                .iter()
                .find(|model| model.model_id().as_str() == id)
                .unwrap()
                .availability(),
            KimiCatalogAvailability::Disabled(KimiCatalogDisabledReason::ProfileUnavailable)
        );
    }
}

// provider가 명시적으로 reasoning 불가를 보고한 forced-thinking 모델과 퇴역/미검토
// ModelId는 inventory에는 남지만 정확한 비활성 사유와 함께 선택 불가능합니다.
#[test]
fn conflicting_retired_and_unreviewed_rows_remain_visible_but_disabled() {
    let rows = json!({
        "object": "list",
        "data": [
            {"object":"model","id":"kimi-k3","context_length":1048576,"supports_reasoning":false},
            {"object":"model","id":"kimi-k2.5","context_length":262144},
            {"object":"model","id":"kimi-k2.7","context_length":262144}
        ]
    });
    let models = normalize_catalog(&seed(), &serde_json::to_vec(&rows).unwrap()).unwrap();
    assert_eq!(models.len(), 3);
    assert_eq!(
        models[0].availability(),
        KimiCatalogAvailability::Disabled(KimiCatalogDisabledReason::ProviderRetirement)
    );
    assert_eq!(
        models[1].availability(),
        KimiCatalogAvailability::Disabled(KimiCatalogDisabledReason::ProfileUnavailable)
    );
    assert_eq!(
        models[2].availability(),
        KimiCatalogAvailability::Disabled(KimiCatalogDisabledReason::CapabilityConflict)
    );
    assert!(models.iter().all(|model| model.entry().is_none()));
}

// identity를 만들 수 없는 행은 생략하고 첫 valid ModelId가 disabled여도 duplicate가 이를
// 덮어쓰지 못하므로 원격 응답 순서가 선택 가능성을 몰래 바꾸지 않습니다.
#[test]
fn first_valid_model_identity_wins_before_overlay_admission() {
    let rows = json!({
        "object": "list",
        "data": [
            null,
            {"object":"other","id":"kimi-k3"},
            {"object":"model","id":"kimi-k3","context_length":1},
            {"object":"model","id":"kimi-k3","context_length":1048576}
        ]
    });
    let models = normalize_catalog(&seed(), &serde_json::to_vec(&rows).unwrap()).unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].model_id().as_str(), "kimi-k3");
    assert!(!models[0].is_enabled());
}

// 4,097번째 행이 하나라도 있으면 snapshot 전체를 거절해 앞부분만 잘라 picker에 보여주는
// 부분 inventory가 entitlement나 ModelId 선택을 왜곡하지 못합니다.
#[test]
fn snapshot_rejects_more_than_4096_rows_before_normalization() {
    let rows = (0..4_097)
        .map(|index| json!({"object":"model","id":format!("model-{index}")}))
        .collect::<Vec<_>>();
    let encoded = serde_json::to_vec(&json!({"object":"list","data":rows})).unwrap();
    let error = normalize_catalog(&seed(), &encoded).unwrap_err();
    assert_eq!(error.kind(), super::KimiCatalogFailureKind::Limit);
}

// transport 외의 bounded snapshot parser를 직접 호출해도 8 MiB를 넘는 원격 bytes는
// JSON parser에 전달하기 전에 거절되어 메모리 상한이 호출 경로에 따라 달라지지 않습니다.
#[test]
fn direct_snapshot_parser_keeps_the_transport_byte_limit() {
    let oversized = vec![b' '; 8 * 1024 * 1024 + 1];
    let error = super::parse_kimi_catalog_snapshot(&seed(), &oversized).unwrap_err();
    assert_eq!(error.kind(), super::KimiCatalogFailureKind::Limit);
}

// 실제 local TLS listener가 Kimi의 exact `/v1/models` GET과 Bearer hash, JSON body를
// 관찰해 production HTTP 경계가 pure snapshot parser와 따로 놀지 않는지 판별합니다.
#[test]
fn fetches_one_authenticated_kimi_inventory_over_local_tls() {
    if run_in_tls_child(
        "model_service::kimi_catalog::tests::fetches_one_authenticated_kimi_inventory_over_local_tls",
    ) {
        return;
    }
    let body = serde_json::to_vec(&json!({
        "object":"list",
        "data":[{"object":"model","id":"kimi-k3","context_length":1048576}]
    }))
    .unwrap();
    let server = LocalTlsServer::start(LocalServerMode::Success {
        body: body.clone(),
        content_type: "application/json; charset=utf-8".to_owned(),
    });
    let root = env::var_os("YO_MODEL_CONNECTOR_TEST_ROOT").unwrap();
    let roots = reqwest::Certificate::from_pem_bundle(&fs::read(root).unwrap()).unwrap();
    let client = Client::builder()
        .add_root_certificate(roots[0].clone())
        .redirect(redirect::Policy::none())
        .retry(reqwest::retry::never())
        .build()
        .unwrap();
    let url = crate::NormalizedEndpoint::parse(server.endpoint())
        .unwrap()
        .append_path_segment("models")
        .unwrap();
    let received = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(super::transport::fetch(
            &client,
            url,
            &crate::ApiCredential::new("sentinel-kimi-catalog-key").unwrap(),
        ))
        .unwrap();
    assert_eq!(received, body);
    server.wait_for_response_sent();
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["method"], "GET");
    assert_eq!(requests[0]["path"], "/v1/models");
    assert!(requests[0].get("authorization").is_none());
    assert!(requests[0]["authorization_sha256"].is_string());
    assert!(
        !serde_json::to_string(&requests)
            .unwrap()
            .contains("sentinel-kimi-catalog-key")
    );
}
