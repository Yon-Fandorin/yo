use serde_json::json;

use super::*;

// 실제 wire 흐름을 검증한 Codex 0.145, 0.146, 0.149 minor line은 실행 파일 이름과
// userAgent의 부가 문자열이 달라도 허용하고, 각 patch 업데이트는 다시 막지 않는다.
#[test]
fn accepts_each_verified_codex_minor_line() {
    assert_eq!(
        version_compatibility_warning("codex_cli_rs/0.145.3 (Linux)").unwrap(),
        None
    );
    assert_eq!(
        version_compatibility_warning("yo/0.146.0 (Arch Linux; x86_64)").unwrap(),
        None
    );
    assert_eq!(
        version_compatibility_warning("codex_cli_rs/0.149.0 (Linux)").unwrap(),
        None
    );
}

// 같은 protocol major의 새 minor line은 설치 업데이트만으로 Yo가 막히지 않게 허용하되,
// 경고에 실제 userAgent와 검증된 line을 함께 남겨 호환성 불확실성을 숨기지 않는다.
#[test]
fn warns_for_an_unverified_codex_minor_line() {
    let warning = version_compatibility_warning("codex_cli_rs/0.150.0")
        .unwrap()
        .expect("an unverified minor line must produce a warning");

    let warning = warning.to_string();
    assert!(warning.contains("codex_cli_rs/0.150.0"));
    assert!(warning.contains("0.145, 0.146, 0.149"));
}

// protocol major가 달라지거나 version을 해석할 수 없으면 minor 업데이트와 구분해
// 초기화 전에 계속 거부하여 실제 비호환 wire를 무조건 실행하지 않는다.
#[test]
fn rejects_a_different_or_unparseable_protocol_major() {
    let failure = version_compatibility_warning("codex_cli_rs/1.0.0").unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::Initialization);
    assert!(failure.message().contains("requires 0.x"));

    let failure = version_compatibility_warning("codex_cli_rs/unknown").unwrap_err();
    assert_eq!(failure.kind(), BackendFailureKind::Initialization);
    assert!(failure.message().contains("unparseable version"));
}

// warning과 version failure가 제어문자와 과도한 길이를 포함해도 bounded 한 줄 안전 출력인지
// 확인합니다.
#[test]
fn bounds_and_escapes_user_agent_in_warning_and_version_errors() {
    let user_agent = format!("codex_cli_rs/0.150.0\n\u{1b}[31m{}", "x".repeat(512));
    let display_user_agent = safe_user_agent(&user_agent);
    assert!(display_user_agent.len() <= MAX_USER_AGENT_DISPLAY_BYTES);
    assert!(!display_user_agent.contains('\n'));
    assert!(!display_user_agent.contains('\u{1b}'));

    let warning = version_compatibility_warning(&user_agent)
        .unwrap()
        .expect("an unverified minor line must produce a warning");

    let warning = warning.to_string();
    assert!(warning.len() <= 512);
    assert!(!warning.contains('\n'));
    assert!(!warning.contains('\u{1b}'));
    assert!(warning.contains("\\n"));
    assert!(warning.contains("\\u{1b}"));

    let malformed = format!("codex_cli_rs/unknown\n\u{1b}[31m{}", "x".repeat(512));
    let failure =
        version_compatibility_warning(&malformed).expect_err("the malformed version must fail");
    assert!(failure.message().len() <= 512);
    assert!(!failure.message().contains('\n'));
    assert!(!failure.message().contains('\u{1b}'));
}

// method와 id가 함께 있는 app-server 메시지는 일반 notification이 아니라 클라이언트가
// 반드시 답해야 하는 server request로 보존되는지 확인한다.
#[test]
fn distinguishes_server_requests_from_notifications() {
    let incoming = classify(json!({
        "id": "approval-1",
        "method": "item/fileChange/requestApproval",
        "params": { "itemId": "item-1" }
    }))
    .unwrap();

    assert!(matches!(
        incoming,
        Incoming::ServerRequest { id, method, .. }
            if id == json!("approval-1") && method == "item/fileChange/requestApproval"
    ));
}

// model/list는 host가 숨긴 항목을 다시 노출하지 않고 exact model ID, display label,
// default marker와 pagination cursor를 함께 보존합니다.
#[test]
fn decodes_only_visible_codex_models_with_default_and_cursor() {
    let page = decode_model_list(json!({
        "data": [
            {"id": "one", "model": "gpt-5.6-codex", "displayName": "GPT-5.6 Codex", "hidden": false, "isDefault": true},
            {"id": "hidden", "model": "internal", "displayName": "Internal", "hidden": true}
        ],
        "nextCursor": "page-2"
    }))
    .unwrap();

    assert_eq!(
        page.models,
        vec![("gpt-5.6-codex".to_owned(), "GPT-5.6 Codex".to_owned(), true)]
    );
    assert_eq!(page.next_cursor.as_deref(), Some("page-2"));
}

// account section은 verified email을 사람이 보는 label로 사용하고 native id가 있으면
// 기존 stable AccountId fingerprint를 보존하며, 없을 때만 email을 증거로 사용합니다.
#[test]
fn codex_account_identity_uses_email_label_and_evidence() {
    let (label, evidence) = decode_account_identity(&json!({
        "account": {"type": "chatgpt", "id": "acct-1", "email": "person@example.test", "planType": "pro"}
    }))
    .unwrap();

    assert_eq!(label, "person@example.test");
    assert_eq!(
        evidence,
        vec![("account_id".to_owned(), "acct-1".to_owned())]
    );
}

// email, subscription, local 순서의 catalog label fallback은 host inventory를 계속
// 표시하고, native id가 없을 때도 선택한 evidence로 안정적인 계정을 만듭니다.
#[test]
fn codex_account_identity_falls_back_to_subscription_or_local() {
    let (label, evidence) = decode_account_identity(&json!({
        "account": {"type": "chatgpt", "planType": "pro"}
    }))
    .unwrap();
    assert_eq!(label, "pro");
    assert_eq!(evidence[0].0, "subscription");

    let (label, evidence) = decode_account_identity(&json!({
        "account": {"type": "apiKey"}
    }))
    .unwrap();
    assert_eq!(label, "local");
    assert_eq!(evidence[0].0, "local");
}

// 이메일이 없는 Codex capacity identity는 공용 default로 추정하지 않고 거부합니다.
#[test]
fn codex_account_capacity_identity_requires_email() {
    let failure = decode_account_capacity_identity(&json!({
        "account": {"id": "acct-1", "planType": "pro"}
    }))
    .unwrap_err();

    assert!(failure.message().contains("no valid `email`"));
}

// native id가 없어도 검증된 이메일을 Codex capacity identity로 사용할 수 있습니다.
#[test]
fn codex_account_capacity_identity_uses_email_when_native_id_is_absent() {
    let (label, evidence) = decode_account_capacity_identity(&json!({
        "account": {"email": "person@example.test", "planType": "pro"}
    }))
    .unwrap();

    assert_eq!(label, "person@example.test");
    assert_eq!(
        evidence,
        vec![("email".to_owned(), "person@example.test".to_owned())]
    );
}

// cache가 허용하는 AccountId 경계 안의 이메일만 capacity identity로 통과시킵니다.
#[test]
fn codex_account_capacity_identity_rejects_an_email_that_cannot_be_cached() {
    let accepted = "a".repeat(256);
    let (label, _) = decode_account_capacity_identity(&json!({
        "account": {"email": accepted}
    }))
    .unwrap();
    assert_eq!(label.len(), 256);

    let rejected = "a".repeat(257);
    let failure = decode_account_capacity_identity(&json!({
        "account": {"email": rejected}
    }))
    .unwrap_err();
    assert!(failure.message().contains("no valid `email`"));
}
