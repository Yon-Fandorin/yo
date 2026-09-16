use std::fs::{create_dir_all, write};

use serde_json::json;

use super::{
    super::{
        model::{
            DELEGATED_DELIVERY_SCHEMA, DELEGATED_USAGE_DELIVERY_SCHEMA, MANAGED_DELIVERY_SCHEMA,
            MANAGED_USAGE_DELIVERY_SCHEMA,
        },
        target::{RouteKind, egress_document, target_preparation},
    },
    TemporaryDirectory, published_review, sample_request,
};
use crate::review_result;

// 관리형 준비는 사람이 반복 작성하던 egress와 admission 문서를 동일 manifest 및
// canonical authorization 해시에 묶고, 실제 Provider 요청 없이 alpha2 delivery로 끝냅니다.
#[test]
fn managed_route_documents_bind_current_authorization_and_delivery_shape() {
    let workspace = TemporaryDirectory::new("review-prepare-managed");
    let authorization = workspace
        .0
        .join(".local-exclude/authorizations/external-review.json");
    create_dir_all(authorization.parent().unwrap()).unwrap();
    write(&authorization, b"managed authority\n").unwrap();
    let request = sample_request(json!({
        "kind": "managed_model",
        "provider": "qwencloud",
        "account": "default",
        "model": "qwen3.8-max",
        "connection_repository_path": "/tmp/connections.yaml",
        "session_repository_path": "/tmp/sessions"
    }));

    let target = target_preparation(&request.target, false, false, false).unwrap();
    let egress = egress_document(&workspace.0, &request.target, &published_review()).unwrap();
    assert!(matches!(target.kind, RouteKind::Managed));
    assert_eq!(target.next_action, "deliver_once");
    assert_eq!(target.delivery_schema, MANAGED_DELIVERY_SCHEMA);
    let egress: serde_json::Value = serde_json::from_slice(&egress).unwrap();
    let admission: serde_json::Value = serde_json::from_slice(&target.admission).unwrap();
    assert_eq!(egress["route"]["model"], "qwen3.8-max");
    assert_eq!(egress["session"]["mode"], "fresh");
    assert_eq!(admission["target"]["kind"], "managed_model");
    assert_eq!(
        admission["schema"],
        "yo.external-review-target-admission-request/v1alpha1"
    );
}

// 위임형 준비는 Provider/Account를 발명하지 않고 host와 고정 execution profile만
// 승인 문서에 결합하며, 새 작업에 필요한 state-ready alpha3 admission을 선택합니다.
#[test]
fn delegated_route_documents_keep_host_owned_identity() {
    let workspace = TemporaryDirectory::new("review-prepare-delegated");
    let authorization = workspace
        .0
        .join(".local-exclude/authorizations/external-review-delegated.json");
    create_dir_all(authorization.parent().unwrap()).unwrap();
    write(&authorization, b"delegated authority\n").unwrap();
    let request = sample_request(json!({
        "kind": "delegated_host",
        "host": "codex",
        "session_repository_path": "/tmp/sessions"
    }));

    let target = target_preparation(&request.target, false, false, false).unwrap();
    let egress = egress_document(&workspace.0, &request.target, &published_review()).unwrap();
    assert!(matches!(target.kind, RouteKind::Delegated));
    assert_eq!(target.next_action, "deliver_delegated_once");
    assert_eq!(target.delivery_schema, DELEGATED_DELIVERY_SCHEMA);
    let egress: serde_json::Value = serde_json::from_slice(&egress).unwrap();
    let admission: serde_json::Value = serde_json::from_slice(&target.admission).unwrap();
    assert_eq!(
        egress["target"],
        json!({"kind": "delegated_host", "host": "codex"})
    );
    assert!(egress.get("route").is_none());
    assert_eq!(
        admission["schema"],
        "yo.external-review-target-admission-request/v1alpha3"
    );
}

// 준비 alpha3만 새 delivery alpha4를 선택해 Provider Usage 결속을 켜며, managed와
// delegated 양쪽의 기존 준비 schema는 그대로 남습니다.
#[test]
fn alpha3_preparation_selects_usage_bound_delivery_without_changing_older_versions() {
    let mut managed = sample_request(json!({
        "kind": "managed_model",
        "provider": "kimi",
        "account": "default",
        "model": "k3",
        "connection_repository_path": "/tmp/connections.yaml"
    }));
    assert_eq!(
        target_preparation(&managed.target, false, false, false)
            .unwrap()
            .delivery_schema,
        MANAGED_DELIVERY_SCHEMA
    );
    managed.schema = "yo.slice-review-prepare-request/v1alpha3".to_owned();
    super::super::request::validate_and_normalize(&mut managed).unwrap();
    assert_eq!(
        target_preparation(&managed.target, true, false, false)
            .unwrap()
            .delivery_schema,
        MANAGED_USAGE_DELIVERY_SCHEMA
    );

    let delegated = sample_request(json!({"kind": "delegated_host", "host": "grok"}));
    assert_eq!(
        target_preparation(&delegated.target, false, false, false)
            .unwrap()
            .delivery_schema,
        DELEGATED_DELIVERY_SCHEMA
    );
    assert_eq!(
        target_preparation(&delegated.target, true, false, false)
            .unwrap()
            .delivery_schema,
        DELEGATED_USAGE_DELIVERY_SCHEMA
    );
    assert_eq!(
        super::super::request::prepared_review_questions(&managed)
            .last()
            .map(String::as_str),
        Some(review_result::OUTPUT_INSTRUCTION)
    );
}

// alpha6는 기존 Usage/authority 계약을 보존하면서 Grok에만 request-free exact
// execution-profile admission을 추가합니다. Codex는 검증되지 않은 invocation을
// 발명하지 않고 alpha3 state readiness를 계속 사용합니다.
#[test]
fn alpha6_selects_profile_readiness_only_for_grok() {
    let mut grok = sample_request(json!({"kind": "delegated_host", "host": "grok"}));
    grok.schema = "yo.slice-review-prepare-request/v1alpha6".to_owned();
    grok.repository_authority_paths.clear();
    grok.repository_authority_policy = Some("changed-workflow-authority/v1alpha2".to_owned());
    super::super::request::validate_and_normalize(&mut grok).unwrap();
    let prepared = target_preparation(&grok.target, true, true, false).unwrap();
    let admission: serde_json::Value = serde_json::from_slice(&prepared.admission).unwrap();
    assert_eq!(
        admission["schema"],
        "yo.external-review-target-admission-request/v1alpha4"
    );
    assert_eq!(prepared.delivery_schema, DELEGATED_USAGE_DELIVERY_SCHEMA);

    let mut codex = sample_request(json!({"kind": "delegated_host", "host": "codex"}));
    codex.schema = "yo.slice-review-prepare-request/v1alpha6".to_owned();
    codex.repository_authority_paths.clear();
    codex.repository_authority_policy = Some("changed-workflow-authority/v1alpha2".to_owned());
    super::super::request::validate_and_normalize(&mut codex).unwrap();
    let prepared = target_preparation(&codex.target, true, true, false).unwrap();
    let admission: serde_json::Value = serde_json::from_slice(&prepared.admission).unwrap();
    assert_eq!(
        admission["schema"],
        "yo.external-review-target-admission-request/v1alpha3"
    );
}

// alpha7은 실제 delivery 준비 경로에서 최신 admission만 선택합니다. 관리형은 오래된
// 실패 기록을 재검증할 수 있고, Grok은 outer sandbox 불가 환경까지 판별하며, Codex는
// 검증되지 않은 Grok 전용 실행 계약을 공유하지 않습니다.
#[test]
fn alpha7_selects_current_admission_without_changing_delivery_protocol() {
    let mut managed = sample_request(json!({
        "kind": "managed_model",
        "provider": "kimi",
        "account": "default",
        "model": "k3-256k",
        "connection_repository_path": "/tmp/connections.yaml"
    }));
    managed.schema = "yo.slice-review-prepare-request/v1alpha7".to_owned();
    managed.repository_authority_paths.clear();
    managed.repository_authority_policy = Some("changed-workflow-authority/v1alpha2".to_owned());
    super::super::request::validate_and_normalize(&mut managed).unwrap();
    let prepared = target_preparation(&managed.target, true, true, true).unwrap();
    let admission: serde_json::Value = serde_json::from_slice(&prepared.admission).unwrap();
    assert_eq!(
        admission["schema"],
        "yo.external-review-target-admission-request/v1alpha6"
    );
    assert_eq!(prepared.delivery_schema, MANAGED_USAGE_DELIVERY_SCHEMA);

    let mut grok = sample_request(json!({"kind": "delegated_host", "host": "grok"}));
    grok.schema = "yo.slice-review-prepare-request/v1alpha7".to_owned();
    grok.repository_authority_paths.clear();
    grok.repository_authority_policy = Some("changed-workflow-authority/v1alpha2".to_owned());
    super::super::request::validate_and_normalize(&mut grok).unwrap();
    let prepared = target_preparation(&grok.target, true, true, true).unwrap();
    let admission: serde_json::Value = serde_json::from_slice(&prepared.admission).unwrap();
    assert_eq!(
        admission["schema"],
        "yo.external-review-target-admission-request/v1alpha5"
    );
    assert_eq!(prepared.delivery_schema, DELEGATED_USAGE_DELIVERY_SCHEMA);

    let mut codex = sample_request(json!({"kind": "delegated_host", "host": "codex"}));
    codex.schema = "yo.slice-review-prepare-request/v1alpha7".to_owned();
    codex.repository_authority_paths.clear();
    codex.repository_authority_policy = Some("changed-workflow-authority/v1alpha2".to_owned());
    super::super::request::validate_and_normalize(&mut codex).unwrap();
    let prepared = target_preparation(&codex.target, true, true, true).unwrap();
    let admission: serde_json::Value = serde_json::from_slice(&prepared.admission).unwrap();
    assert_eq!(
        admission["schema"],
        "yo.external-review-target-admission-request/v1alpha3"
    );
}
