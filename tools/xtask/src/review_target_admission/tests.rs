use std::str::FromStr;

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

// 관리형 admission만 기존 managed delivery로 이어지고, 위임형 host probe 성공은
// 별도 egress/delivery 계약이 생기기 전까지 preparation 상태에 머뭅니다.
#[test]
fn admitted_next_action_does_not_authorize_delegated_delivery() {
    let managed = ReviewTarget::ManagedModel {
        provider: "qwencloud".to_owned(),
        account: "default".to_owned(),
        model: "qwen3.8-max".to_owned(),
    };
    let delegated = ReviewTarget::DelegatedHost {
        host: "codex".to_owned(),
    };

    assert_eq!(managed.admitted_outcome(), ("eligible", "deliver_once"));
    assert_eq!(
        delegated.admitted_outcome(),
        ("prepared", "await_delegated_delivery_protocol")
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

// 새 wire family는 저장소 정책대로 v1alpha1에서 시작하고 stable v1이나 관리형
// `route` 모양을 같은 admission 요청으로 재해석하지 않습니다.
#[test]
fn admission_request_requires_v1alpha1_and_closed_target_shape() {
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

    let mut stable = valid.clone();
    stable["schema"] = "yo.external-review-target-admission-request/v1".into();
    let stable: super::model::Request = serde_json::from_value(stable).unwrap();
    assert!(
        super::validate_request(&stable)
            .unwrap_err()
            .contains("v1alpha1")
    );

    let mut extra = valid;
    extra["target"]["route"] = "host:codex".into();
    assert!(serde_json::from_value::<super::model::Request>(extra).is_err());
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
