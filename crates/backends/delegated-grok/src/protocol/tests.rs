use serde_json::json;

use super::*;

// ACP 메시지는 JSON-RPC 2.0 표식을 생략하면 method와 id가 올바르더라도 wire 경계를
// 통과하지 못해야 하며, 표식이 있는 server request의 문자열 id는 그대로 보존합니다.
#[test]
fn requires_json_rpc_two_and_preserves_server_request_ids() {
    assert!(classify(json!({ "method": "session/update", "params": {} })).is_err());
    assert!(
        classify(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {},
            "error": { "code": -32000, "message": "ambiguous" }
        }))
        .is_err()
    );
    let incoming = classify(json!({
        "jsonrpc": "2.0",
        "id": "permission-1",
        "method": "session/request_permission",
        "params": {}
    }))
    .unwrap();
    assert!(matches!(
        incoming,
        Incoming::ServerRequest { id, method, .. }
            if id == json!("permission-1") && method == "session/request_permission"
    ));

    for invalid_id in [json!(1.5), json!(true), json!([1]), json!({ "id": 1 })] {
        assert!(
            classify(json!({
                "jsonrpc": "2.0",
                "id": invalid_id,
                "method": "session/request_permission",
                "params": {}
            }))
            .is_err()
        );
    }
    assert!(
        classify(json!({
            "jsonrpc": "2.0",
            "id": "x".repeat(MAX_REQUEST_ID_BYTES + 1),
            "method": "session/request_permission",
            "params": {}
        }))
        .is_err()
    );
}

// 현재 backend가 구현하는 ACP v1만 초기화에서 수락하고, 다른 negotiated version은
// Session을 만들기 전에 Initialization 실패로 구분합니다.
#[test]
fn accepts_only_acp_protocol_version_one() {
    let accepted = decode_initialize(json!({ "protocolVersion": 1, "authMethods": [] })).unwrap();
    assert_eq!(accepted.agent_version, "acp-v1");

    let failure =
        decode_initialize(json!({ "protocolVersion": 2, "authMethods": [] })).unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::Initialization);
}

// 인증 method의 누락·잘못된 ID를 인증 불필요로 오인하면 cached login 경계를 우회하므로
// authMethods는 배열과 각 고유 ID를 완전하게 검증합니다.
#[test]
fn rejects_malformed_or_duplicate_authentication_methods() {
    let missing = json!({
        "protocolVersion": 1,
        "agentCapabilities": {}
    });
    assert!(decode_initialize(missing).is_err());

    let malformed = json!({
        "protocolVersion": 1,
        "authMethods": [{ "name": "Cached" }]
    });
    assert!(decode_initialize(malformed).is_err());

    let duplicate = json!({
        "protocolVersion": 1,
        "authMethods": [
            { "id": "cached_token" },
            { "id": "cached_token" }
        ]
    });
    assert!(decode_initialize(duplicate).is_err());
}

// ACP initialize의 modelState는 Automatic 같은 합성 row 없이 host가 광고한 exact
// currentModelId와 availableModels를 그대로 보존합니다.
#[test]
fn decodes_exact_grok_model_state_without_an_automatic_row() {
    let initialized = decode_initialize(json!({
        "protocolVersion": 1,
        "authMethods": [],
        "_meta": {
            "modelState": {
                "currentModelId": "grok-4.6",
                "availableModels": [
                    {"modelId": "grok-4.6", "name": "Grok 4.6"},
                    {"modelId": "grok-4.5", "name": "Grok 4.5"}
                ]
            }
        }
    }))
    .unwrap();

    assert_eq!(initialized.current_model_id.as_deref(), Some("grok-4.6"));
    assert_eq!(
        initialized.available_models,
        vec![
            ("grok-4.6".to_owned(), "Grok 4.6".to_owned()),
            ("grok-4.5".to_owned(), "Grok 4.5".to_owned())
        ]
    );
    assert!(
        initialized
            .available_models
            .iter()
            .all(|(id, _)| id != "automatic")
    );
}

// account catalog은 verified email을 우선하고, 없는 경우 검증된 subscription tier와
// local evidence를 사용해 host picker를 계속 표시합니다.
#[test]
fn grok_account_identity_prefers_email_then_subscription_or_local() {
    let (label, evidence) = decode_account_identity(&json!({
        "_meta": {"email": "person@example.test", "subscription_tier": "supergrok"}
    }))
    .unwrap();
    assert_eq!(label, "person@example.test");
    assert_eq!(evidence[0].0, "email");

    let (label, evidence) = decode_account_identity(&json!({
        "_meta": {"subscription_tier": "supergrok"}
    }))
    .unwrap();
    assert_eq!(label, "supergrok");
    assert_eq!(evidence[0].0, "subscription_tier");

    let (label, evidence) = decode_account_identity(&json!({"_meta": {}})).unwrap();
    assert_eq!(label, "local");
    assert_eq!(evidence[0].0, "local");

    let (label, evidence) = decode_account_identity(&json!({})).unwrap();
    assert_eq!(label, "local");
    assert_eq!(evidence[0].0, "local");
}

// account capacity는 실제 로그인 계정 email이 없으면 subscription을 계정처럼
// 저장하지 않고 refresh 자체를 실패시킵니다.
#[test]
fn grok_account_capacity_identity_requires_email() {
    let (label, evidence) = decode_account_capacity_identity(&json!({
        "_meta": {"email": "person@example.test", "subscription_tier": "supergrok"}
    }))
    .unwrap();
    assert_eq!(label, "person@example.test");
    assert_eq!(evidence[0].0, "email");

    let failure = decode_account_capacity_identity(&json!({
        "_meta": {"subscription_tier": "supergrok"}
    }))
    .unwrap_err();
    assert!(failure.message().contains("no valid `email`"));
}
