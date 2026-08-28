use std::path::PathBuf;

use serde_json::json;

use super::{
    DELEGATED_DELIVERY_SCHEMA, DELEGATED_USAGE_DELIVERY_SCHEMA, MANAGED_DELIVERY_SCHEMA,
    MANAGED_USAGE_DELIVERY_SCHEMA, PreparedBytes, PreparedPaths, Request, RouteKind, Target,
    authority_paths_for_changed_paths, egress_document, prepared_review_questions,
    require_empty_directory, require_prepared_requests_current, target_preparation,
    validate_and_normalize,
};
use crate::{review_packet::PublishedReview, test_support::unique_path};

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new(label: &str) -> Self {
        let path = unique_path(label);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn published_review() -> PublishedReview {
    PublishedReview {
        status: "created",
        schema: "yo.slice-review-packet-result/v1",
        authority: None,
        review_id: format!("sha256:{}", "1".repeat(64)),
        trusted_commit: "2".repeat(40),
        candidate_commit: "3".repeat(40),
        packet_path: ".local-exclude/methexis/slice-reviews/review/packet.md".to_owned(),
        packet_hash: format!("sha256:{}", "4".repeat(64)),
        packet_bytes: 123,
        managed_payload_tokens: 45,
        manifest_path: ".local-exclude/methexis/slice-reviews/review/manifest.json".to_owned(),
        manifest_hash: format!("sha256:{}", "5".repeat(64)),
        max_managed_payload_tokens: 1000,
    }
}

fn request(target: serde_json::Value) -> Request {
    serde_json::from_value(json!({
        "schema": "yo.slice-review-prepare-request/v1alpha1",
        "slice": "review-preparation",
        "knowledge_ids": ["methexis.review.bounded-packet"],
        "context_max_tokens": 16000,
        "repository_authority_paths": ["CONTRIBUTING.md"],
        "validation_evidence": [{"name": "xtask", "path": "/tmp/xtask.json"}],
        "review_lenses": ["fresh-context", "code-quality"],
        "review_questions": ["Is the boundary exact?"],
        "max_managed_payload_tokens": 100000,
        "target": target
    }))
    .unwrap()
}

// code-only 후보는 큰 repository workflow 문서를 반복 복사하지 않고, workflow 구현이나
// AGENTS authority 자체가 바뀐 후보만 exact authority bytes를 packet에 포함합니다.
#[test]
fn changed_authority_policy_omits_fixed_cost_for_product_code() {
    assert_eq!(
        authority_paths_for_changed_paths(&["crates/yo-core/src/lib.rs".to_owned()]),
        vec!["AGENTS.md".to_owned()]
    );
    assert_eq!(
        authority_paths_for_changed_paths(&[
            "tools/xtask/src/lib.rs".to_owned(),
            "nested/AGENTS.md".to_owned(),
        ]),
        vec![
            "AGENTS.md".to_owned(),
            "CONTRIBUTING.md".to_owned(),
            "nested/AGENTS.md".to_owned()
        ]
    );
    assert_eq!(
        authority_paths_for_changed_paths(&["CONTRIBUTING/review-and-integration.md".to_owned()]),
        vec!["AGENTS.md".to_owned(), "CONTRIBUTING.md".to_owned()]
    );
}

// 관리형 준비는 사람이 반복 작성하던 egress와 admission 문서를 동일 manifest 및
// canonical authorization 해시에 묶고, 실제 Provider 요청 없이 alpha2 delivery로 끝냅니다.
#[test]
fn managed_route_documents_bind_current_authorization_and_delivery_shape() {
    let workspace = TemporaryDirectory::new("review-prepare-managed");
    let authorization = workspace
        .0
        .join(".local-exclude/authorizations/external-review.json");
    std::fs::create_dir_all(authorization.parent().unwrap()).unwrap();
    std::fs::write(&authorization, b"managed authority\n").unwrap();
    let request = request(json!({
        "kind": "managed_model",
        "provider": "qwencloud",
        "account": "default",
        "model": "qwen3.8-max",
        "connection_repository_path": "/tmp/connections.yaml",
        "session_repository_path": "/tmp/sessions"
    }));

    let target = target_preparation(&request.target, false).unwrap();
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
    std::fs::create_dir_all(authorization.parent().unwrap()).unwrap();
    std::fs::write(&authorization, b"delegated authority\n").unwrap();
    let request = request(json!({
        "kind": "delegated_host",
        "host": "codex",
        "session_repository_path": "/tmp/sessions"
    }));

    let target = target_preparation(&request.target, false).unwrap();
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

// 입력 정규화는 중복 authority와 지원되지 않는 host를 초기에 거부하여 ContextBuild나
// packet publication을 시작하기 전에 사람이 고칠 수 있는 작은 오류로 남깁니다.
#[test]
fn input_validation_rejects_duplicates_and_unknown_hosts() {
    let mut duplicate = request(json!({"kind": "delegated_host", "host": "codex"}));
    duplicate.repository_authority_paths = vec!["CONTRIBUTING.md".into(), "CONTRIBUTING.md".into()];
    assert!(
        validate_and_normalize(&mut duplicate)
            .unwrap_err()
            .contains("duplicate value")
    );

    let mut unknown = request(json!({"kind": "delegated_host", "host": "other"}));
    assert!(
        validate_and_normalize(&mut unknown)
            .unwrap_err()
            .contains("must be `codex` or `grok`")
    );
    assert!(matches!(unknown.target, Target::DelegatedHost { .. }));
}

// delivery output에 claim이나 결과가 하나라도 있으면 준비 재실행이 이를 비우거나
// 덮어쓰지 않고 중단되어 exact-once 요청 경계를 보존합니다.
#[test]
fn nonempty_delivery_output_is_never_reprepared() {
    let directory = TemporaryDirectory::new("review-prepare-output");
    require_empty_directory(&directory.0).unwrap();
    std::fs::write(directory.0.join("claim.json"), b"claim\n").unwrap();
    assert!(
        require_empty_directory(&directory.0)
            .unwrap_err()
            .contains("not empty")
    );
}

// packet publication 뒤 생성 요청 하나가 바뀌어도 마지막 통합 경계가 성공 결과를
// 반환하지 않도록, 모든 준비 산출물의 정확한 바이트를 한 번에 다시 확인합니다.
#[test]
fn final_prepared_request_check_rejects_post_publication_drift() {
    let directory = TemporaryDirectory::new("review-prepare-final-drift");
    let context = directory.0.join("context.json");
    let review = directory.0.join("review.json");
    let egress = directory.0.join("egress.json");
    let admission = directory.0.join("admission.json");
    let delivery = directory.0.join("delivery.json");
    for (path, bytes) in [
        (&context, b"context\n".as_slice()),
        (&review, b"review\n".as_slice()),
        (&egress, b"egress\n".as_slice()),
        (&admission, b"admission\n".as_slice()),
        (&delivery, b"delivery\n".as_slice()),
    ] {
        std::fs::write(path, bytes).unwrap();
    }
    let paths = PreparedPaths {
        context: &context,
        review: &review,
        egress: &egress,
        admission: &admission,
        delivery: &delivery,
        delivery_output: &directory.0,
    };
    let bytes = PreparedBytes {
        context: b"context\n",
        review: b"review\n",
        egress: b"egress\n",
        admission: b"admission\n",
        delivery: b"delivery\n",
    };
    require_prepared_requests_current(&paths, &bytes).unwrap();

    std::fs::write(&review, b"changed\n").unwrap();
    assert!(
        require_prepared_requests_current(&paths, &bytes)
            .unwrap_err()
            .contains("Slice review packet request changed")
    );
}

// alpha2 통합 경로만 terminal structured-result 지시를 정확히 한 번 추가하고,
// 이미 발행된 alpha1 준비 의미와 호출자가 작성한 질문은 그대로 보존합니다.
#[test]
fn alpha2_adds_structured_result_instruction_without_reinterpreting_alpha1() {
    let legacy = request(json!({"kind": "delegated_host", "host": "codex"}));
    assert_eq!(prepared_review_questions(&legacy), legacy.review_questions);

    let mut structured = legacy;
    structured.schema = "yo.slice-review-prepare-request/v1alpha2".to_owned();
    let questions = prepared_review_questions(&structured);
    assert_eq!(questions.len(), structured.review_questions.len() + 1);
    assert!(
        questions
            .last()
            .unwrap()
            .contains("yo.slice-review-result/v1alpha1")
    );
    validate_and_normalize(&mut structured).unwrap();
}

// 준비 alpha3만 새 delivery alpha4를 선택해 Provider Usage 결속을 켜며, managed와
// delegated 양쪽의 기존 준비 schema는 그대로 남습니다.
#[test]
fn alpha3_preparation_selects_usage_bound_delivery_without_changing_older_versions() {
    let mut managed = request(json!({
        "kind": "managed_model",
        "provider": "kimi",
        "account": "default",
        "model": "k3",
        "connection_repository_path": "/tmp/connections.yaml"
    }));
    assert_eq!(
        target_preparation(&managed.target, false)
            .unwrap()
            .delivery_schema,
        MANAGED_DELIVERY_SCHEMA
    );
    managed.schema = "yo.slice-review-prepare-request/v1alpha3".to_owned();
    validate_and_normalize(&mut managed).unwrap();
    assert_eq!(
        target_preparation(&managed.target, true)
            .unwrap()
            .delivery_schema,
        MANAGED_USAGE_DELIVERY_SCHEMA
    );

    let delegated = request(json!({"kind": "delegated_host", "host": "grok"}));
    assert_eq!(
        target_preparation(&delegated.target, false)
            .unwrap()
            .delivery_schema,
        DELEGATED_DELIVERY_SCHEMA
    );
    assert_eq!(
        target_preparation(&delegated.target, true)
            .unwrap()
            .delivery_schema,
        DELEGATED_USAGE_DELIVERY_SCHEMA
    );
    assert_eq!(
        prepared_review_questions(&managed)
            .last()
            .map(String::as_str),
        Some(crate::review_result::OUTPUT_INSTRUCTION)
    );
}

// alpha4만 caller authority 목록을 금지하고 고정 policy를 요구해, 비용 절감이 임의
// authority 누락으로 바뀌지 않도록 request 경계에서 닫습니다.
#[test]
fn alpha4_requires_derived_authority_policy() {
    let mut derived = request(json!({"kind": "delegated_host", "host": "grok"}));
    derived.schema = "yo.slice-review-prepare-request/v1alpha4".to_owned();
    derived.repository_authority_paths.clear();
    derived.repository_authority_policy = Some("changed-workflow-authority/v1alpha1".to_owned());
    validate_and_normalize(&mut derived).unwrap();

    derived.repository_authority_paths = vec!["CONTRIBUTING.md".to_owned()];
    assert!(
        validate_and_normalize(&mut derived)
            .unwrap_err()
            .contains("requires the caller list to be empty")
    );
}
