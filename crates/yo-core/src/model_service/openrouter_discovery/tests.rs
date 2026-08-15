use std::{env, fs, time::Duration};

use reqwest::{Client, Url, redirect};
use serde_json::{Value, json};

use super::*;
use crate::{
    ApiDialect, EffectiveModelBinding, ModelProfileLayer, VersionedProfileId,
    model_connector::tests::local_tls::{LocalServerMode, LocalTlsServer, run_in_tls_child},
};

fn base_profile() -> EffectiveModelProfile {
    EffectiveModelProfile::resolve(
        None,
        &ModelProfileLayer::new(
            Some(ApiDialect::OpenAiResponses),
            Some(VersionedProfileId::new("o200k_base/v1").unwrap()),
            Some(100_000),
            Some(8_192),
            Some(serde_json::from_value(Value::Null).unwrap()),
            Some(serde_json::from_value(Value::Null).unwrap()),
            Some(VersionedProfileId::new("tools-required/v1").unwrap()),
            Some(VersionedProfileId::new("semantic-terminal/v1").unwrap()),
        ),
    )
    .unwrap()
}

fn seed() -> OpenRouterDiscoverySeed {
    OpenRouterDiscoverySeed::new(
        ProviderId::new("openrouter").unwrap(),
        AccountId::new("team").unwrap(),
        Some("OpenRouter".to_owned()),
        Some("Team".to_owned()),
        NormalizedEndpoint::parse("https://openrouter.ai/api/v1").unwrap(),
        base_profile(),
        Vec::new(),
    )
    .unwrap()
}

fn row(id: &str) -> Value {
    json!({
        "id": id,
        "name": id,
        "architecture": {
            "input_modalities": ["text"],
            "output_modalities": ["text"]
        },
        "supported_parameters": ["tools", "reasoning"],
        "context_length": 120000,
        "top_provider": {"max_completion_tokens": 12000}
    })
}

// 실제 OpenRouter 필드 경로에서 text/tools 모델만 채택하고 remote context 한도만
// base profile 위에 덮어써 완전한 binding을 만드는지 판별합니다.
#[test]
fn normalizes_selectable_rows_into_complete_bindings() {
    let models = normalize_catalog(
        &seed(),
        json!({"data": [row("vendor/alpha")]})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    let [model] = models.as_slice() else {
        panic!("one model expected")
    };
    assert!(model.reasoning());
    assert_eq!(model.display_name(), "vendor/alpha");
    let complete = model.entry().complete_binding().unwrap();
    assert_eq!(complete.profile().context().input_token_limit(), 120_000);
    assert_eq!(complete.profile().context().max_output_tokens(), 12_000);
    assert_eq!(
        complete.profile().context().tokenizer_profile(),
        "o200k_base/v1"
    );
}

// 첫 valid ModelId 행이 기능 미달이어도 duplicate winner로 남아 나중의 selectable
// duplicate가 권한 목록을 다시 해석해 들어오지 못하는지 관찰합니다.
#[test]
fn first_valid_duplicate_wins_even_when_unselectable() {
    let mut first = row("vendor/duplicate");
    first["supported_parameters"] = json!(["reasoning"]);
    let models = normalize_catalog(
        &seed(),
        json!({"data": [first, row("vendor/duplicate")]})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    assert!(models.is_empty());
}

// 4097번째 행을 일부 잘라 쓰지 않고 응답 전체를 거절하고, malformed optional
// 숫자나 capability shape는 해당 행만 누락하는 두 limit 경계를 구분합니다.
#[test]
fn rejects_row_overflow_and_omits_malformed_rows() {
    let exact_rows = vec![Value::Null; MAX_ROWS];
    assert!(
        normalize_catalog(&seed(), json!({"data": exact_rows}).to_string().as_bytes())
            .unwrap()
            .is_empty()
    );
    let rows = vec![Value::Null; MAX_ROWS + 1];
    let error =
        normalize_catalog(&seed(), json!({"data": rows}).to_string().as_bytes()).unwrap_err();
    assert_eq!(error.kind(), OpenRouterDiscoveryFailureKind::Limit);

    let mut bad_number = row("vendor/bad-number");
    bad_number["context_length"] = json!(0);
    let mut bad_shape = row("vendor/bad-shape");
    bad_shape["architecture"]["input_modalities"] = json!(["text", 1]);
    assert!(
        normalize_catalog(
            &seed(),
            json!({"data": [bad_number, bad_shape]})
                .to_string()
                .as_bytes(),
        )
        .unwrap()
        .is_empty()
    );
}

// 같은 configured Model의 authored complete profile은 remote limit보다 우선하고, 새 Model의
// missing/null limit은 base를 보존하여 absent capability를 0이나 invented default로 바꾸지
// 않습니다.
#[test]
fn preserves_authored_overrides_and_base_values_for_absent_remote_limits() {
    let base = base_profile();
    let authored_profile = profile_with_remote_limits(&base, Some(80_000), Some(4_000)).unwrap();
    let authored = ModelCatalogEntry::with_explicit_profile(
        EffectiveModelBinding::new(
            ProviderId::new("openrouter").unwrap(),
            AccountId::new("team").unwrap(),
            ModelId::new("vendor/authored").unwrap(),
            authored_profile.api_dialect(),
            NormalizedEndpoint::parse("https://openrouter.ai/api/v1").unwrap(),
        ),
        Some("OpenRouter".to_owned()),
        Some("Team".to_owned()),
        Some("Configured label".to_owned()),
        authored_profile,
    )
    .unwrap();
    let authored = OpenRouterAuthoredModel::new(authored, Some(80_000), Some(4_000)).unwrap();
    let seed = OpenRouterDiscoverySeed::new(
        ProviderId::new("openrouter").unwrap(),
        AccountId::new("team").unwrap(),
        Some("OpenRouter".to_owned()),
        Some("Team".to_owned()),
        NormalizedEndpoint::parse("https://openrouter.ai/api/v1").unwrap(),
        base,
        vec![authored],
    )
    .unwrap();
    let mut authored_row = row("vendor/authored");
    authored_row["name"] = json!("Remote label");
    let mut inherited_row = row("vendor/inherited");
    inherited_row
        .as_object_mut()
        .unwrap()
        .remove("context_length");
    inherited_row["top_provider"]["max_completion_tokens"] = Value::Null;

    let models = normalize_catalog(
        &seed,
        json!({"data": [authored_row, inherited_row]})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    let authored = models
        .iter()
        .find(|model| model.entry().binding().model_id().as_str() == "vendor/authored")
        .unwrap();
    assert_eq!(authored.display_name(), "Remote label");
    assert_eq!(authored.entry().context().input_token_limit(), 80_000);
    assert_eq!(authored.entry().context().max_output_tokens(), 4_000);
    assert_eq!(
        authored.entry().model_display_name(),
        Some("Configured label")
    );
    let inherited = models
        .iter()
        .find(|model| model.entry().binding().model_id().as_str() == "vendor/inherited")
        .unwrap();
    assert_eq!(inherited.entry().context().input_token_limit(), 100_000);
    assert_eq!(inherited.entry().context().max_output_tokens(), 8_192);
}

// configured model 자체가 limit 필드를 쓰지 않았으면 base에서 완성된 값이 있더라도
// remote account catalog의 limit을 채택하고, 직접 쓴 한 필드만 remote보다 우선합니다.
#[test]
fn applies_remote_limits_per_field_without_losing_authored_provenance() {
    let configured = |model: &str, input: u64, output: u64| {
        let profile =
            profile_with_remote_limits(&base_profile(), Some(input), Some(output)).unwrap();
        ModelCatalogEntry::with_explicit_profile(
            EffectiveModelBinding::new(
                ProviderId::new("openrouter").unwrap(),
                AccountId::new("team").unwrap(),
                ModelId::new(model).unwrap(),
                profile.api_dialect(),
                NormalizedEndpoint::parse("https://openrouter.ai/api/v1").unwrap(),
            ),
            None,
            None,
            None,
            profile,
        )
        .unwrap()
    };
    let inherited =
        OpenRouterAuthoredModel::new(configured("vendor/inherited", 100_000, 8_192), None, None)
            .unwrap();
    let input_override = OpenRouterAuthoredModel::new(
        configured("vendor/input-override", 80_000, 8_192),
        Some(80_000),
        None,
    )
    .unwrap();
    let seed = OpenRouterDiscoverySeed::new(
        ProviderId::new("openrouter").unwrap(),
        AccountId::new("team").unwrap(),
        None,
        None,
        NormalizedEndpoint::parse("https://openrouter.ai/api/v1").unwrap(),
        base_profile(),
        vec![inherited, input_override],
    )
    .unwrap();

    let models = normalize_catalog(
        &seed,
        json!({"data": [row("vendor/inherited"), row("vendor/input-override")]})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    let limits = |model: &str| {
        let context = models
            .iter()
            .find(|candidate| candidate.entry().binding().model_id().as_str() == model)
            .unwrap()
            .entry()
            .context();
        (context.input_token_limit(), context.max_output_tokens())
    };
    assert_eq!(limits("vendor/inherited"), (120_000, 12_000));
    assert_eq!(limits("vendor/input-override"), (80_000, 12_000));
}

// accepted name은 trim 뒤 96-byte 경계를 지키고 초과하면 ID로 되돌아가며, normalized
// case-insensitive label 뒤 exact ID tie-break로 remote 응답 순서와 무관한 snapshot을 만듭니다.
#[test]
fn bounds_display_names_and_sorts_the_snapshot_deterministically() {
    let mut fallback = row("vendor/fallback");
    fallback["name"] = json!("x".repeat(MAX_REMOTE_NAME_BYTES + 1));
    let mut beta = row("vendor/beta");
    beta["name"] = json!("  Ｂeta  ");
    let mut alpha = row("vendor/alpha");
    alpha["name"] = json!("alpha");
    let models = normalize_catalog(
        &seed(),
        json!({"data": [fallback, beta, alpha]})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    assert_eq!(
        models
            .iter()
            .map(OpenRouterDiscoveredModel::display_name)
            .collect::<Vec<_>>(),
        vec!["alpha", "Ｂeta", "vendor/fallback"]
    );
}

// endpoint의 기존 prefix를 보존해 `/models/user`를 덧붙이고 JSON media type만
// 허용하여 계정 카탈로그가 다른 경로나 content type으로 흐르지 않는지 확인합니다.
#[test]
fn appends_catalog_path_and_accepts_only_json_media_types() {
    let url = discovery_url(&NormalizedEndpoint::parse("https://example.com/prefix/v1").unwrap())
        .unwrap();
    assert_eq!(url.as_str(), "https://example.com/prefix/v1/models/user");
    assert!(is_json_media_type("application/json; charset=utf-8"));
    assert!(is_json_media_type("application/problem+json"));
    assert!(!is_json_media_type("text/json"));
}

// same-origin 판정에 scheme, host, effective port와 credential-safe URL shape를 모두
// 포함하여 다른 origin 또는 userinfo/query/fragment로 Bearer가 전달되지 않게 합니다.
#[test]
fn origin_rejects_every_credential_crossing_boundary() {
    let source = Url::parse("https://example.com/api/v1/models/user").unwrap();
    let origin = Origin::new(&source).unwrap();
    assert!(origin.matches(&Url::parse("https://example.com/next").unwrap()));
    for target in [
        "http://example.com/next",
        "https://other.example/next",
        "https://example.com:444/next",
        "https://user@example.com/next",
        "https://example.com/next?q=1",
        "https://example.com/next#fragment",
    ] {
        assert!(!origin.matches(&Url::parse(target).unwrap()), "{target}");
    }
}

fn fetch_from_local_tls(server: &LocalTlsServer) -> Result<Vec<u8>, OpenRouterDiscoveryError> {
    fetch_from_local_tls_with_timeouts(server, DISCOVERY_TIMEOUTS)
}

fn fetch_from_local_tls_with_timeouts(
    server: &LocalTlsServer,
    timeouts: DiscoveryTimeouts,
) -> Result<Vec<u8>, OpenRouterDiscoveryError> {
    let root = env::var_os("YO_MODEL_CONNECTOR_TEST_ROOT")
        .expect("the local TLS child must provide its root certificate");
    let roots = reqwest::Certificate::from_pem_bundle(&fs::read(root).unwrap()).unwrap();
    let client = Client::builder()
        .add_root_certificate(roots[0].clone())
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(redirect::Policy::none())
        .retry(reqwest::retry::never())
        .build()
        .unwrap();
    let endpoint = NormalizedEndpoint::parse(server.endpoint()).unwrap();
    let url = discovery_url(&endpoint).unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(fetch_catalog_with_timeouts(
            &client,
            url,
            &ApiCredential::new("sentinel-catalog-secret").unwrap(),
            timeouts,
        ))
}

// 실제 local TLS child를 통해 prefix-preserving GET, Bearer 전달, JSON media type과 body
// 수신을 함께 통과시켜 production reqwest 경로가 parser unit test와 분리되지 않게 합니다.
#[test]
fn fetches_the_authenticated_account_catalog_over_local_tls() {
    if run_in_tls_child(
        "model_service::openrouter_discovery::tests::fetches_the_authenticated_account_catalog_over_local_tls",
    ) {
        return;
    }
    let body = json!({"data": [row("vendor/live")]})
        .to_string()
        .into_bytes();
    let server = LocalTlsServer::start(LocalServerMode::Success {
        body: body.clone(),
        content_type: "application/json; charset=utf-8".to_owned(),
    });
    assert_eq!(fetch_from_local_tls(&server).unwrap(), body);
    server.wait_for_response_sent();
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["method"], "GET");
    assert_eq!(requests[0]["path"], "/v1/models/user");
    assert!(requests[0].get("authorization").is_none());
    assert!(requests[0]["authorization_sha256"].is_string());
}

// 다른 port로 향하는 redirect는 같은 loopback host여도 origin 변경으로 보고 두 번째
// listener에 Bearer를 보내기 전에 실패하며 원래 서버 요청 하나만 남기는지 확인합니다.
#[test]
fn rejects_cross_origin_redirect_before_forwarding_the_credential() {
    if run_in_tls_child(
        "model_service::openrouter_discovery::tests::rejects_cross_origin_redirect_before_forwarding_the_credential",
    ) {
        return;
    }
    let server = LocalTlsServer::start(LocalServerMode::Redirect {
        location: "https://127.0.0.1:9/foreign".to_owned(),
        final_body: Vec::new(),
    });
    let error = fetch_from_local_tls(&server).unwrap_err();
    assert_eq!(error.kind(), OpenRouterDiscoveryFailureKind::Transport);
    assert!(error.to_string().contains("changed origin"));
    assert_eq!(server.accepted_connections(), 1);
    assert_eq!(server.requests().len(), 1);
}

// same-origin redirect는 실제 두 번째 GET까지 따라가고 두 요청의 authorization hash가 같아
// in-memory candidate를 유지하되 origin 검증 뒤에만 다시 보내는지 확인합니다.
#[test]
fn follows_a_same_origin_redirect_with_the_same_candidate() {
    if run_in_tls_child(
        "model_service::openrouter_discovery::tests::follows_a_same_origin_redirect_with_the_same_candidate",
    ) {
        return;
    }
    let server = LocalTlsServer::start(LocalServerMode::Redirect {
        location: "/v1/models/user-next".to_owned(),
        final_body: json!({"data": []}).to_string().into_bytes(),
    });
    let error = fetch_from_local_tls(&server).unwrap_err();
    assert_eq!(error.kind(), OpenRouterDiscoveryFailureKind::MediaType);
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["path"], "/v1/models/user");
    assert_eq!(requests[1]["path"], "/v1/models/user-next");
    assert_eq!(
        requests[0]["authorization_sha256"],
        requests[1]["authorization_sha256"]
    );
}

// header를 전혀 내지 않는 TLS peer와 JSON header 뒤 body를 멈춘 peer를 짧은 test clock으로
// 구분하여 두 inactivity phase가 production fetch loop의 서로 다른 위치에 묶였는지 확인합니다.
#[test]
fn distinguishes_response_header_and_body_inactivity_deadlines() {
    if run_in_tls_child(
        "model_service::openrouter_discovery::tests::distinguishes_response_header_and_body_inactivity_deadlines",
    ) {
        return;
    }
    let header_timeouts = DiscoveryTimeouts {
        response_header: Duration::from_millis(50),
        body_idle: Duration::from_millis(30),
        absolute: Duration::from_secs(1),
    };
    let header_stall = LocalTlsServer::start(LocalServerMode::ResponseHeaderStall);
    let header_error =
        fetch_from_local_tls_with_timeouts(&header_stall, header_timeouts).unwrap_err();
    assert_eq!(header_error.kind(), OpenRouterDiscoveryFailureKind::Timeout);
    assert!(header_error.to_string().contains("response-header"));

    let body_stall = LocalTlsServer::start(LocalServerMode::HeadersThenStall {
        content_type: "application/json".to_owned(),
    });
    let body_error = fetch_from_local_tls_with_timeouts(
        &body_stall,
        DiscoveryTimeouts {
            response_header: Duration::from_millis(500),
            body_idle: Duration::from_millis(30),
            absolute: Duration::from_secs(1),
        },
    )
    .unwrap_err();
    assert_eq!(body_error.kind(), OpenRouterDiscoveryFailureKind::Timeout);
    assert!(body_error.to_string().contains("response-body"));
}

// redirect attempt마다 200ms 지연되는 세 응답은 전체 500ms보다 오래 걸려도 attempt별
// 500ms header clock이면 세 번째까지 도달해, redirect chain 전체에 clock 하나를 씌운
// 회귀를 잡습니다. per-attempt 여유는 느린 macOS CI의 TLS scheduling도 허용합니다.
#[test]
fn each_redirect_attempt_gets_a_fresh_response_header_deadline() {
    if run_in_tls_child(
        "model_service::openrouter_discovery::tests::each_redirect_attempt_gets_a_fresh_response_header_deadline",
    ) {
        return;
    }
    let server = LocalTlsServer::start(LocalServerMode::DelayedRedirectChain {
        final_body: json!({"data": []}).to_string().into_bytes(),
        response_delay_millis: 200,
    });
    let error = fetch_from_local_tls_with_timeouts(
        &server,
        DiscoveryTimeouts {
            response_header: Duration::from_millis(500),
            body_idle: Duration::from_millis(500),
            absolute: Duration::from_secs(2),
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), OpenRouterDiscoveryFailureKind::MediaType);
    assert_eq!(server.requests().len(), 3);
}

// attempt별 header clock보다 짧은 absolute clock은 redirect chain 전체를 중단해야 하며,
// absolute bound를 제거하면 같은 fixture가 세 응답을 모두 받아 다른 오류가 됩니다.
#[test]
fn absolute_deadline_caps_the_complete_redirect_chain() {
    if run_in_tls_child(
        "model_service::openrouter_discovery::tests::absolute_deadline_caps_the_complete_redirect_chain",
    ) {
        return;
    }
    let server = LocalTlsServer::start(LocalServerMode::DelayedRedirectChain {
        final_body: json!({"data": []}).to_string().into_bytes(),
        response_delay_millis: 200,
    });
    let error = fetch_from_local_tls_with_timeouts(
        &server,
        DiscoveryTimeouts {
            response_header: Duration::from_millis(500),
            body_idle: Duration::from_millis(500),
            absolute: Duration::from_millis(300),
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), OpenRouterDiscoveryFailureKind::Timeout);
    assert!(server.requests().len() < 3);
}

// 같은 origin redirect는 정확히 세 번까지만 따라가고 네 번째 redirect 응답에서 새 요청을
// 보내지 않은 채 limit 오류가 되어 credential 재전송 횟수를 닫는지 판별합니다.
#[test]
fn rejects_a_fourth_same_origin_redirect() {
    if run_in_tls_child(
        "model_service::openrouter_discovery::tests::rejects_a_fourth_same_origin_redirect",
    ) {
        return;
    }
    let server = LocalTlsServer::start(LocalServerMode::RedirectLoop);
    let error = fetch_from_local_tls(&server).unwrap_err();
    assert_eq!(error.kind(), OpenRouterDiscoveryFailureKind::Transport);
    assert!(error.to_string().contains("redirect limit"));
    assert_eq!(server.requests().len(), MAX_REDIRECTS + 1);
}

// declared Content-Length와 길이 없는 streaming body에서 각각 8 MiB 초과를 거절해
// 어느 한쪽 guard를 제거해도 다른 guard가 대신 테스트를 통과시키지 못하게 합니다.
#[test]
fn rejects_oversize_declared_and_streamed_response_bodies() {
    if run_in_tls_child(
        "model_service::openrouter_discovery::tests::rejects_oversize_declared_and_streamed_response_bodies",
    ) {
        return;
    }
    let declared = LocalTlsServer::start(LocalServerMode::DeclaredOversize);
    let declared_error = fetch_from_local_tls(&declared).unwrap_err();
    assert_eq!(declared_error.kind(), OpenRouterDiscoveryFailureKind::Limit);
    declared.wait_for_response_sent();

    let streamed = LocalTlsServer::start(LocalServerMode::UnframedSuccess {
        body: vec![b'x'; MAX_RESPONSE_BYTES + 1],
        content_type: "application/json".to_owned(),
    });
    let streamed_error = fetch_from_local_tls(&streamed).unwrap_err();
    assert_eq!(streamed_error.kind(), OpenRouterDiscoveryFailureKind::Limit);
    streamed.wait_for_response_sent();
}
