use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

use yo_core::{AccountCapacitySnapshot, AccountId, LocalConnectionRepository, ProviderId};

use super::{parse_query, resolve_targets};
use crate::command::account::domain::{
    AccountCapacityReport, AccountCoordinate, AccountQuery, AccountTarget, exact_refresh_matches,
};

// account query는 전체, Provider 전체, exact Provider:Account 범위를 구분합니다.
#[test]
fn parses_account_query_scopes() {
    assert_eq!(parse_query(None).unwrap(), AccountQuery::All);
    assert_eq!(
        parse_query(Some("codex")).unwrap(),
        AccountQuery::Provider(ProviderId::new("codex").unwrap())
    );
    assert_eq!(
        parse_query(Some("kimi:default")).unwrap(),
        AccountQuery::Exact(AccountCoordinate::new("kimi", "default").unwrap())
    );
    assert_eq!(
        parse_query(Some("qwencloud:default")).unwrap(),
        AccountQuery::Exact(AccountCoordinate::new("qwencloud", "default").unwrap())
    );
    for source in [
        "kimi:",
        "kimi:default:extra",
        "qwencloud:",
        "qwencloud:default:extra",
    ] {
        assert!(parse_query(Some(source)).is_err());
    }
}

// host cache의 email label을 입력해도 stable 내부 AccountId 좌표로 찾아야 합니다.
#[test]
fn resolves_a_host_account_by_its_email_label() {
    let root = std::env::temp_dir().join(format!(
        "yo-account-alias-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let connections = LocalConnectionRepository::new(root.join("connections.yaml"))
        .capture()
        .unwrap();
    let report = AccountCapacityReport::plain(AccountCapacitySnapshot::new(
        ProviderId::new("codex").unwrap(),
        AccountId::new("0123456789abcdef").unwrap(),
        Vec::new(),
    ))
    .with_account_label("person@example.test")
    .with_observed_at("2026-09-03T01:02:03Z".to_owned());
    let query = parse_query(Some("codex:person@example.test")).unwrap();

    let targets = resolve_targets(&query, &connections, &[report], false).unwrap();

    assert_eq!(
        targets,
        vec![AccountTarget::Exact(
            AccountCoordinate::new("codex", "0123456789abcdef").unwrap()
        )]
    );
    let _ = fs::remove_dir_all(root);
}

// host refresh 결과의 인증 email이 exact 요청과 일치하면 새 좌표를 정상적으로 승인합니다.
#[test]
fn exact_host_refresh_accepts_the_requested_email_identity() {
    let query = parse_query(Some("grok:person@example.test")).unwrap();
    let target = AccountTarget::Exact(AccountCoordinate::new("grok", "fedcba9876543210").unwrap());
    let report = AccountCapacityReport::plain(AccountCapacitySnapshot::new(
        ProviderId::new("grok").unwrap(),
        AccountId::new("fedcba9876543210").unwrap(),
        Vec::new(),
    ))
    .with_account_label("person@example.test");

    assert_eq!(exact_refresh_matches(&query, &target, &report), Some(true));
}

// 저장되지 않은 Kimi 계정은 임의의 cache 행으로 취급하지 않고 선택을 거부합니다.
#[test]
fn does_not_select_an_unsupported_cached_account() {
    let root = std::env::temp_dir().join(format!(
        "yo-account-eligibility-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let connections = LocalConnectionRepository::new(root.join("connections.yaml"))
        .capture()
        .unwrap();
    let report = AccountCapacityReport::plain(AccountCapacitySnapshot::new(
        ProviderId::new("kimi").unwrap(),
        AccountId::new("retired").unwrap(),
        Vec::new(),
    ));
    let query = parse_query(Some("kimi:retired")).unwrap();

    let error = resolve_targets(&query, &connections, &[report], false).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("no stored account-capacity source")
    );
    let _ = fs::remove_dir_all(root);
}

// 이전 account- 접두어 cache도 읽을 수 있어 기존 사용자 cache를 잃지 않습니다.
#[test]
fn accepts_a_legacy_host_cache_key_after_the_prefix_removal() {
    let root = std::env::temp_dir().join(format!(
        "yo-account-legacy-key-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let connections = LocalConnectionRepository::new(root.join("connections.yaml"))
        .capture()
        .unwrap();
    let report = AccountCapacityReport::plain(AccountCapacitySnapshot::new(
        ProviderId::new("codex").unwrap(),
        AccountId::new("account-0123456789abcdef").unwrap(),
        Vec::new(),
    ));
    let query = parse_query(Some("codex:0123456789abcdef")).unwrap();

    let targets = resolve_targets(&query, &connections, &[report], false).unwrap();

    assert_eq!(
        targets,
        vec![AccountTarget::Exact(
            AccountCoordinate::new("codex", "account-0123456789abcdef").unwrap()
        )]
    );
    let _ = fs::remove_dir_all(root);
}
