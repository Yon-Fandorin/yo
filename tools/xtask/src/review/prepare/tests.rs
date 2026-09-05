use std::path::PathBuf;

use serde_json::json;

use super::{
    DELEGATED_DELIVERY_SCHEMA, DELEGATED_USAGE_DELIVERY_SCHEMA, MANAGED_DELIVERY_SCHEMA,
    MANAGED_USAGE_DELIVERY_SCHEMA, PreparedBytes, PreparedPaths, Request, RouteKind, Target,
    authority_paths_for_changed_paths_v1alpha1, authority_paths_for_changed_paths_v1alpha2,
    egress_document, prepared_review_questions, require_empty_directory,
    require_prepared_requests_current, target_preparation, validate_and_normalize,
};
use crate::{review::packet::PublishedReview, test_support::unique_path};

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
        authority_paths_for_changed_paths_v1alpha1(&["crates/yo-core/src/lib.rs".to_owned()]),
        vec!["AGENTS.md".to_owned()]
    );
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha1(&[
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
        authority_paths_for_changed_paths_v1alpha1(&[
            "CONTRIBUTING/review-and-integration.md".to_owned(),
        ]),
        vec!["AGENTS.md".to_owned(), "CONTRIBUTING.md".to_owned()]
    );
}

// alpha2 policy는 변경된 workflow 책임만 직접 소유 문서로 라우팅하고, 공용 facade는
// 구체 구현 경로가 정한 소유자를 불필요하게 넓히지 않습니다.
#[test]
fn changed_authority_policy_v1alpha2_routes_precise_workflow_owners() {
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha2(&[
            "tools/xtask/src/lib.rs".to_owned(),
            "tools/xtask/src/review.rs".to_owned(),
            "tools/xtask/src/review/prepare.rs".to_owned(),
        ]),
        vec![
            "AGENTS.md".to_owned(),
            "CONTRIBUTING/review-packets.md".to_owned()
        ]
    );
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha2(&[
            "tools/xtask/src/lib.rs".to_owned(),
            "tools/xtask/src/review/delivery.rs".to_owned(),
        ]),
        vec![
            "AGENTS.md".to_owned(),
            "CONTRIBUTING/review-delivery.md".to_owned()
        ]
    );
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha2(&[
            "tools/xtask/src/lib.rs".to_owned(),
            "tools/xtask/src/slice.rs".to_owned(),
            "tools/xtask/src/slice/gate.rs".to_owned(),
        ]),
        vec![
            "AGENTS.md".to_owned(),
            "CONTRIBUTING/review-and-integration.md".to_owned(),
        ]
    );
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha2(&[
            "tools/xtask/src/slice_contract.rs".to_owned(),
        ]),
        vec!["AGENTS.md".to_owned(), "CONTRIBUTING.md".to_owned()]
    );
}

// 여러 책임을 실제로 건드린 후보는 소유 문서의 합집합을 포함하고, 변경된 nested
// AGENTS authority도 exact path로 보존합니다.
#[test]
fn changed_authority_policy_v1alpha2_unions_cross_owner_changes() {
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha2(&[
            "tools/xtask/src/review/packet.rs".to_owned(),
            "tools/xtask/src/review/egress.rs".to_owned(),
            "tools/xtask/src/slice.rs".to_owned(),
            "tools/xtask/src/slice/close.rs".to_owned(),
            "nested/AGENTS.md".to_owned(),
        ]),
        vec![
            "AGENTS.md".to_owned(),
            "CONTRIBUTING/review-and-integration.md".to_owned(),
            "CONTRIBUTING/review-delivery.md".to_owned(),
            "CONTRIBUTING/review-packets.md".to_owned(),
            "nested/AGENTS.md".to_owned(),
        ]
    );
}

// packet, delivery, gate가 함께 소비하는 protocol과 packet/gate가 함께 소비하는 result
// 파일은 한 소유자로 축소하지 않고 실제 소비 영역의 authority 합집합을 포함합니다.
#[test]
fn changed_authority_policy_v1alpha2_routes_shared_protocol_owners() {
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha2(&[
            "tools/xtask/src/review_protocol.rs".to_owned(),
        ]),
        vec![
            "AGENTS.md".to_owned(),
            "CONTRIBUTING/review-and-integration.md".to_owned(),
            "CONTRIBUTING/review-delivery.md".to_owned(),
            "CONTRIBUTING/review-packets.md".to_owned(),
        ]
    );
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha2(&[
            "tools/xtask/src/review_result.rs".to_owned(),
        ]),
        vec![
            "AGENTS.md".to_owned(),
            "CONTRIBUTING/review-and-integration.md".to_owned(),
            "CONTRIBUTING/review-packets.md".to_owned(),
        ]
    );
}

// 소유자를 식별할 companion 없이 공용 facade나 shared workflow 기반만 바뀌면 모든
// workflow owner를 포함해 누락 대신 비용 증가로 fail-closed합니다.
#[test]
fn changed_authority_policy_v1alpha2_fails_closed_for_ambiguous_workflow() {
    let expected = vec![
        "AGENTS.md".to_owned(),
        "CONTRIBUTING.md".to_owned(),
        "CONTRIBUTING/review-and-integration.md".to_owned(),
        "CONTRIBUTING/review-delivery.md".to_owned(),
        "CONTRIBUTING/review-packets.md".to_owned(),
    ];
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha2(&["tools/xtask/src/lib.rs".to_owned()]),
        expected
    );
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha2(&["tools/xtask/src/review.rs".to_owned()]),
        expected
    );
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha2(&[
            "tools/xtask/src/bounded_file.rs".to_owned(),
            "tools/xtask/src/review/prepare.rs".to_owned(),
        ]),
        expected
    );
}

// product-only 후보는 기존처럼 작은 root router만 포함하며, 직접 변경한 review owner는
// root CONTRIBUTING을 경유하지 않고 그 exact file을 포함합니다.
#[test]
fn changed_authority_policy_v1alpha2_keeps_product_cost_minimal() {
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha2(&["crates/yo-core/src/lib.rs".to_owned()]),
        vec!["AGENTS.md".to_owned()]
    );
    assert_eq!(
        authority_paths_for_changed_paths_v1alpha2(
            &["CONTRIBUTING/review-delivery.md".to_owned(),]
        ),
        vec![
            "AGENTS.md".to_owned(),
            "CONTRIBUTING/review-delivery.md".to_owned()
        ]
    );
}

// 실제 repository authority bytes를 동일 tokenizer로 비교해, packet 전용 작은 후보와
// packet+integration 중간 후보가 기존 coarse root authority보다 각각 2,000 tokens 이상
// 줄어드는지 고정합니다. 이 수치는 lens나 owner를 생략하지 않은 라우팅 결과입니다.
#[test]
fn precise_authority_owners_materially_reduce_small_and_medium_fixed_cost() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = std::fs::read(repository.join("CONTRIBUTING.md")).unwrap();
    let packets = std::fs::read(repository.join("CONTRIBUTING/review-packets.md")).unwrap();
    let integration =
        std::fs::read(repository.join("CONTRIBUTING/review-and-integration.md")).unwrap();
    let tokenizer = tiktoken_rs::o200k_base_singleton();
    let count = |bytes: &[u8]| {
        tokenizer
            .encode_with_special_tokens(std::str::from_utf8(bytes).unwrap())
            .len()
    };
    let root_tokens = count(&root);
    let packet_tokens = count(&packets);
    let mut medium = packets;
    medium.push(b'\n');
    medium.extend(integration);
    let medium_tokens = count(&medium);

    assert!(root_tokens >= packet_tokens + 2_000);
    assert!(root_tokens >= medium_tokens + 2_000);
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
    std::fs::create_dir_all(authorization.parent().unwrap()).unwrap();
    std::fs::write(&authorization, b"delegated authority\n").unwrap();
    let request = request(json!({
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
        target_preparation(&managed.target, false, false, false)
            .unwrap()
            .delivery_schema,
        MANAGED_DELIVERY_SCHEMA
    );
    managed.schema = "yo.slice-review-prepare-request/v1alpha3".to_owned();
    validate_and_normalize(&mut managed).unwrap();
    assert_eq!(
        target_preparation(&managed.target, true, false, false)
            .unwrap()
            .delivery_schema,
        MANAGED_USAGE_DELIVERY_SCHEMA
    );

    let delegated = request(json!({"kind": "delegated_host", "host": "grok"}));
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
        prepared_review_questions(&managed)
            .last()
            .map(String::as_str),
        Some(crate::review_result::OUTPUT_INSTRUCTION)
    );
}

// alpha4는 기존 alpha1 policy를 계속 요구해 이미 발행된 요청의 라우팅 의미가 새
// 세분화 정책으로 바뀌지 않도록 동결합니다.
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

// alpha5는 caller 목록을 금지하고 alpha2 policy만 받아, 세분화된 owner 선택이 임의
// authority 누락이나 alpha4 의미 재해석으로 바뀌지 않게 합니다.
#[test]
fn alpha5_requires_precise_derived_authority_policy() {
    let mut derived = request(json!({"kind": "delegated_host", "host": "grok"}));
    derived.schema = "yo.slice-review-prepare-request/v1alpha5".to_owned();
    derived.repository_authority_paths.clear();
    derived.repository_authority_policy = Some("changed-workflow-authority/v1alpha2".to_owned());
    validate_and_normalize(&mut derived).unwrap();

    derived.repository_authority_policy = Some("changed-workflow-authority/v1alpha1".to_owned());
    assert!(
        validate_and_normalize(&mut derived)
            .unwrap_err()
            .contains("changed-workflow-authority/v1alpha2")
    );
}

// alpha6는 기존 Usage/authority 계약을 보존하면서 Grok에만 request-free exact
// execution-profile admission을 추가합니다. Codex는 검증되지 않은 invocation을
// 발명하지 않고 alpha3 state readiness를 계속 사용합니다.
#[test]
fn alpha6_selects_profile_readiness_only_for_grok() {
    let mut grok = request(json!({"kind": "delegated_host", "host": "grok"}));
    grok.schema = "yo.slice-review-prepare-request/v1alpha6".to_owned();
    grok.repository_authority_paths.clear();
    grok.repository_authority_policy = Some("changed-workflow-authority/v1alpha2".to_owned());
    validate_and_normalize(&mut grok).unwrap();
    let prepared = target_preparation(&grok.target, true, true, false).unwrap();
    let admission: serde_json::Value = serde_json::from_slice(&prepared.admission).unwrap();
    assert_eq!(
        admission["schema"],
        "yo.external-review-target-admission-request/v1alpha4"
    );
    assert_eq!(prepared.delivery_schema, DELEGATED_USAGE_DELIVERY_SCHEMA);

    let mut codex = request(json!({"kind": "delegated_host", "host": "codex"}));
    codex.schema = "yo.slice-review-prepare-request/v1alpha6".to_owned();
    codex.repository_authority_paths.clear();
    codex.repository_authority_policy = Some("changed-workflow-authority/v1alpha2".to_owned());
    validate_and_normalize(&mut codex).unwrap();
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
    let mut managed = request(json!({
        "kind": "managed_model",
        "provider": "kimi",
        "account": "default",
        "model": "k3-256k",
        "connection_repository_path": "/tmp/connections.yaml"
    }));
    managed.schema = "yo.slice-review-prepare-request/v1alpha7".to_owned();
    managed.repository_authority_paths.clear();
    managed.repository_authority_policy = Some("changed-workflow-authority/v1alpha2".to_owned());
    validate_and_normalize(&mut managed).unwrap();
    let prepared = target_preparation(&managed.target, true, true, true).unwrap();
    let admission: serde_json::Value = serde_json::from_slice(&prepared.admission).unwrap();
    assert_eq!(
        admission["schema"],
        "yo.external-review-target-admission-request/v1alpha6"
    );
    assert_eq!(prepared.delivery_schema, MANAGED_USAGE_DELIVERY_SCHEMA);

    let mut grok = request(json!({"kind": "delegated_host", "host": "grok"}));
    grok.schema = "yo.slice-review-prepare-request/v1alpha7".to_owned();
    grok.repository_authority_paths.clear();
    grok.repository_authority_policy = Some("changed-workflow-authority/v1alpha2".to_owned());
    validate_and_normalize(&mut grok).unwrap();
    let prepared = target_preparation(&grok.target, true, true, true).unwrap();
    let admission: serde_json::Value = serde_json::from_slice(&prepared.admission).unwrap();
    assert_eq!(
        admission["schema"],
        "yo.external-review-target-admission-request/v1alpha5"
    );
    assert_eq!(prepared.delivery_schema, DELEGATED_USAGE_DELIVERY_SCHEMA);

    let mut codex = request(json!({"kind": "delegated_host", "host": "codex"}));
    codex.schema = "yo.slice-review-prepare-request/v1alpha7".to_owned();
    codex.repository_authority_paths.clear();
    codex.repository_authority_policy = Some("changed-workflow-authority/v1alpha2".to_owned());
    validate_and_normalize(&mut codex).unwrap();
    let prepared = target_preparation(&codex.target, true, true, true).unwrap();
    let admission: serde_json::Value = serde_json::from_slice(&prepared.admission).unwrap();
    assert_eq!(
        admission["schema"],
        "yo.external-review-target-admission-request/v1alpha3"
    );
}
