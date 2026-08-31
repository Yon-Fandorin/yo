use std::{path::PathBuf, str::FromStr};

use yo_core::{
    ModelRequestFailureKind, SessionId,
    session_repository::{StoredSession, StoredSessionUnavailableReason},
};

use super::{model::ReviewTarget, validate_target};

// 관리형 좌표와 위임형 host는 서로 다른 identity 공간을 사용하며 `host`를 Provider로
// 위조하거나 임의의 실행 파일 이름을 외부 검토 대상으로 넓히지 않습니다.
#[test]
fn review_target_identity_keeps_models_and_hosts_separate() {
    let managed = ReviewTarget::ManagedModel {
        provider: "qwencloud".to_owned(),
        account: "default".to_owned(),
        model: "qwen3.8-max".to_owned(),
    };
    let codex = ReviewTarget::DelegatedHost {
        host: "codex".to_owned(),
    };
    let grok = ReviewTarget::DelegatedHost {
        host: "grok".to_owned(),
    };

    validate_target(&managed).unwrap();
    validate_target(&codex).unwrap();
    validate_target(&grok).unwrap();
    assert_eq!(managed.reference(), "qwencloud:default:qwen3.8-max");
    assert_eq!(codex.reference(), "host:codex");
    assert_eq!(grok.reference(), "host:grok");
    assert!(
        validate_target(&ReviewTarget::DelegatedHost {
            host: "arbitrary".to_owned(),
        })
        .is_err()
    );
}

// 관리형과 delegated target은 서로 다른 실행 경로를 선택하되 둘 다 claim 전 exact
// admission 결과를 제공하며, host가 managed deliver_once로 섞이지 않습니다.
#[test]
fn admitted_next_action_selects_the_disjoint_delivery_protocol() {
    let managed = ReviewTarget::ManagedModel {
        provider: "qwencloud".to_owned(),
        account: "default".to_owned(),
        model: "qwen3.8-max".to_owned(),
    };
    let delegated = ReviewTarget::DelegatedHost {
        host: "codex".to_owned(),
    };

    assert_eq!(
        managed.admitted_outcome(super::model::REQUEST_SCHEMA),
        ("eligible", "deliver_once")
    );
    assert_eq!(
        delegated.admitted_outcome(super::model::REQUEST_SCHEMA),
        ("prepared", "await_delegated_delivery_protocol")
    );
    assert_eq!(
        delegated.admitted_outcome(super::model::REQUEST_SCHEMA_V1_ALPHA2),
        ("eligible", "deliver_delegated_once")
    );
    assert_eq!(
        delegated.admitted_outcome(super::model::REQUEST_SCHEMA_V1_ALPHA3),
        ("eligible", "deliver_delegated_once")
    );
    assert_eq!(
        delegated.admitted_outcome(super::model::REQUEST_SCHEMA_V1_ALPHA4),
        ("eligible", "deliver_delegated_once")
    );
}

// bounded 탐색 구간에 신뢰할 수 없는 Session이 있으면 이를 absence로 건너뛰지 않고,
// matching receipt를 숨길 수 있는 불확실성으로 즉시 보고합니다.
#[test]
fn unavailable_session_inside_bounded_search_returns_unknown() {
    let session = StoredSession::Unavailable {
        session_id: SessionId::from_str("01a03d11-0595-7c53-b58e-b9bdda7fdc82").unwrap(),
        reason: StoredSessionUnavailableReason::NoCompleteEnvelope,
    };

    let search = super::usage::unavailable_search(&session, 1, false).unwrap();

    assert_eq!(search.state, "unknown");
    assert_eq!(search.inspected_sessions, 1);
    assert!(
        search
            .detail
            .unwrap()
            .contains("cannot inspect candidate Session")
    );
}

// 이미 발행된 v1alpha1은 그대로 수용하고 delegated delivery 의미는 v1alpha2에서만
// 추가합니다. stable v1이나 관리형 `route` 모양은 어느 alpha 요청으로도 재해석하지
// 않습니다.
#[test]
fn admission_request_preserves_alpha1_and_accepts_alpha2() {
    let valid = serde_json::json!({
        "schema": "yo.external-review-target-admission-request/v1alpha1",
        "target": {
            "kind": "managed_model",
            "provider": "qwencloud",
            "account": "default",
            "model": "qwen3.8-max"
        },
        "connection_repository_path": "/tmp/connections.yaml"
    });
    let parsed: super::model::Request = serde_json::from_value(valid.clone()).unwrap();
    super::validate_request(&parsed).unwrap();
    assert_eq!(
        super::model::result_schema(&parsed.schema),
        super::model::RESULT_SCHEMA
    );

    let mut stable = valid.clone();
    stable["schema"] = "yo.external-review-target-admission-request/v1".into();
    let stable: super::model::Request = serde_json::from_value(stable).unwrap();
    assert!(
        super::validate_request(&stable)
            .unwrap_err()
            .contains("v1alpha2")
    );

    let mut alpha2 = valid.clone();
    alpha2["schema"] = super::model::REQUEST_SCHEMA_V1_ALPHA2.into();
    let alpha2: super::model::Request = serde_json::from_value(alpha2).unwrap();
    super::validate_request(&alpha2).unwrap();
    assert_eq!(
        super::model::result_schema(&alpha2.schema),
        super::model::RESULT_SCHEMA_V1_ALPHA2
    );

    let mut alpha3 = valid.clone();
    alpha3["schema"] = super::model::REQUEST_SCHEMA_V1_ALPHA3.into();
    let alpha3: super::model::Request = serde_json::from_value(alpha3).unwrap();
    super::validate_request(&alpha3).unwrap();
    assert_eq!(
        super::model::result_schema(&alpha3.schema),
        super::model::RESULT_SCHEMA_V1_ALPHA3
    );

    let mut alpha4 = valid.clone();
    alpha4["schema"] = super::model::REQUEST_SCHEMA_V1_ALPHA4.into();
    let alpha4: super::model::Request = serde_json::from_value(alpha4).unwrap();
    super::validate_request(&alpha4).unwrap();
    assert_eq!(
        super::model::result_schema(&alpha4.schema),
        super::model::RESULT_SCHEMA_V1_ALPHA4
    );

    let mut extra = valid;
    extra["target"]["route"] = "host:codex".into();
    assert!(serde_json::from_value::<super::model::Request>(extra).is_err());
}

// 강한 delegated admission은 실제 host 상태 경로에서 claim 전에 필요한 최소
// create/remove 권한을 증명하고 성공한 probe 파일을 남기지 않습니다.
#[test]
fn delegated_state_readiness_is_request_free_and_self_cleaning() {
    let temporary = crate::test_support::unique_path("delegated-state-readiness");
    std::fs::create_dir_all(&temporary).unwrap();
    super::probe_host_state_writable(&temporary).unwrap();
    assert_eq!(std::fs::read_dir(&temporary).unwrap().count(), 0);

    let missing = temporary.join("missing");
    assert!(
        super::probe_host_state_writable(&missing)
            .unwrap_err()
            .contains("cannot inspect")
    );
    std::fs::remove_dir(&temporary).unwrap();
}

#[cfg(unix)]
fn executable_script(label: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = crate::test_support::unique_path(label);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

#[cfg(unix)]
// profile probe는 prompt나 ACP initialize를 쓰지 않고 stdin EOF에서 exact Grok
// reviewer argv의 sandbox startup 성공만 확인합니다.
#[test]
fn grok_profile_readiness_uses_exact_request_free_argv() {
    let script = executable_script(
        "grok-profile-ready",
        r#"test "$*" = "--sandbox read-only --permission-mode dontAsk --tools Read,Grep --no-subagents --disable-web-search agent stdio" || exit 7
if IFS= read -r unexpected; then exit 8; fi
exit 0"#,
    );
    super::probe_grok_read_only_startup(&script).unwrap();
    std::fs::remove_file(script).unwrap();
}

#[cfg(unix)]
// sandbox가 부분 적용을 거부하면 admission은 stderr 원인을 보존해 claim 전에
// fail-closed하며 unsandboxed fallback을 시도하지 않습니다.
#[test]
fn grok_profile_readiness_reports_sandbox_startup_failure() {
    let script = executable_script(
        "grok-profile-unavailable",
        "echo 'cannot mask /run/containerd/containerd.sock'\necho 'could not apply the read-only sandbox profile' >&2\nexit 1",
    );
    let error = super::probe_grok_read_only_startup(&script).unwrap_err();
    assert!(error.contains("exited without success"));
    assert!(error.contains("cannot mask /run/containerd/containerd.sock"));
    assert!(error.contains("could not apply the read-only sandbox profile"));
    std::fs::remove_file(script).unwrap();
}

// Provider가 공개한 account quota source가 없는 상태는 token 합계를 잔여량으로
// 오인하지 않으며 typed transient failure도 exhaustion으로 확대하지 않습니다.
#[test]
fn admission_result_keeps_unknown_account_limit_explicit() {
    let value = serde_json::to_value(super::model::AccountLimit {
        availability: "unknown",
        remaining: None,
        resets_at: None,
        source: None,
    })
    .unwrap();
    assert_eq!(value["availability"], "unknown");
    assert!(value["remaining"].is_null());
    assert!(value["resets_at"].is_null());
}

// request-free admission은 인증·권한·exact model·로컬 binding 실패만 명시적 불가로
// 취급하고 rate limit이나 token 합계를 quota exhaustion으로 추측하지 않습니다.
#[test]
fn only_typed_unavailability_failures_block_before_claim() {
    for kind in [
        ModelRequestFailureKind::Authentication,
        ModelRequestFailureKind::AccessDenied,
        ModelRequestFailureKind::ModelUnavailable,
        ModelRequestFailureKind::LocalConfiguration,
    ] {
        assert!(super::blocking_failure(kind));
    }
    for kind in [
        ModelRequestFailureKind::RateLimited,
        ModelRequestFailureKind::RequestRejected,
        ModelRequestFailureKind::ProviderUnavailable,
        ModelRequestFailureKind::Transport,
        ModelRequestFailureKind::Timeout,
        ModelRequestFailureKind::Protocol,
        ModelRequestFailureKind::ResponseLimit,
    ] {
        assert!(!super::blocking_failure(kind));
    }
}
