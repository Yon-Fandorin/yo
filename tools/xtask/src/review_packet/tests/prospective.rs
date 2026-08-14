use super::{
    super::{
        REVIEW_ID_DOMAIN_V1_ALPHA3,
        bootstrap::{CAPABILITY_BYTES, CAPABILITY_PATH, require_prospective_activation_boundary},
        canonical::{build_manifest, build_plan},
        capture::{capture_prospective_context_from_result, captured},
        model::{DELIVERY_PROFILE_V1_ALPHA2, MANIFEST_SCHEMA_V1_ALPHA3, PLAN_SCHEMA_V1_ALPHA3},
        render::{count_tokens, render_packet_with_metadata},
        validate_request,
        verifier::verify_canonical_artifacts,
    },
    support::sample_inputs_v1_alpha3,
};
use crate::{
    review_protocol::{artifact, digest, domain_digest},
    test_support::TestRepository,
};

fn artifacts() -> (
    super::super::capture::Inputs,
    super::super::model::Manifest,
    Vec<u8>,
    Vec<u8>,
) {
    let inputs = sample_inputs_v1_alpha3("/tmp/validation.json");
    let plan = build_plan(&inputs);
    let review_id = domain_digest(
        REVIEW_ID_DOMAIN_V1_ALPHA3,
        &serde_json::to_vec(&plan).unwrap(),
    );
    let rendered = render_packet_with_metadata(&review_id, &plan, &inputs).unwrap();
    let manifest = build_manifest(
        review_id,
        plan,
        &inputs,
        digest(&rendered.bytes),
        count_tokens(&rendered.bytes).unwrap(),
        rendered.input_prefix,
    );
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
    manifest_bytes.push(b'\n');
    (inputs, manifest, manifest_bytes, rendered.bytes)
}

fn bootstrap_fixture(
    label: &str,
    capability_in_trusted: bool,
    implementation_change: bool,
) -> (
    TestRepository,
    String,
    String,
    crate::review_protocol::Captured,
) {
    let repository = TestRepository::new(label);
    repository.write(".gitignore", ".local-exclude/\n");
    repository.write("methexis/active-checkpoint.yaml", "active: old\n");
    repository.write(
        "tools/methexis/examples/context-contract/manifest.json",
        "{\"build\":\"old-direct\"}\n",
    );
    repository.write(
        "tools/methexis/examples/context-contract/stable-leaf-manifest.json",
        "{\"build\":\"old-stable\"}\n",
    );
    if capability_in_trusted {
        repository.write(
            CAPABILITY_PATH,
            std::str::from_utf8(CAPABILITY_BYTES).unwrap(),
        );
    }
    repository.git(["add", "."]);
    repository.git(["commit", "--quiet", "-m", "trusted develop"]);
    let trusted_commit = crate::git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
        .unwrap()
        .trim()
        .to_owned();

    let checkpoint_id = format!("sha256:{}", "b".repeat(64));
    repository.write("methexis/active-checkpoint.yaml", "active: proposed\n");
    repository.write(
        format!(
            "methexis/checkpoints/{}.yaml",
            checkpoint_id.strip_prefix("sha256:").unwrap()
        ),
        "checkpoint: proposed\n",
    );
    repository.write(
        "tools/methexis/examples/context-contract/manifest.json",
        "{\"build\":\"new-direct\"}\n",
    );
    repository.write(
        "tools/methexis/examples/context-contract/stable-leaf-manifest.json",
        "{\"build\":\"new-stable\"}\n",
    );
    if !capability_in_trusted {
        repository.write(
            CAPABILITY_PATH,
            std::str::from_utf8(CAPABILITY_BYTES).unwrap(),
        );
    }
    if implementation_change {
        repository.write(
            "tools/xtask/src/review_packet/mod.rs",
            "// candidate changes its own prospective implementation\n",
        );
    }
    repository.git(["add", "."]);
    repository.git(["commit", "--quiet", "-m", "activation candidate"]);
    let candidate_commit = crate::git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
        .unwrap()
        .trim()
        .to_owned();
    let activation_bytes = format!(
        "{{\"schema\":\"methexis.activation-request/v1alpha1\",\
         \"checkpoint_id\":\"{checkpoint_id}\",\
         \"checkpoint_hash\":\"sha256:{}\",\
         \"replace_active_hash\":\"sha256:{}\"}}\n",
        "c".repeat(64),
        "a".repeat(64)
    )
    .into_bytes();
    let activation = captured(
        repository
            .write(
                ".local-exclude/activation.json",
                std::str::from_utf8(&activation_bytes).unwrap(),
            )
            .to_string_lossy()
            .into_owned(),
        activation_bytes,
    )
    .unwrap();
    (repository, trusted_commit, candidate_commit, activation)
}

// v1alpha3는 후보 활성화를 active authority로 승격하지 않고 exact proposal 세 입력과
// prospective 표식을 plan, manifest, model-visible prefix에 함께 결합한다.
#[test]
fn prospective_packet_binds_the_exact_proposal_without_claiming_active_authority() {
    let (inputs, manifest, manifest_bytes, packet) = artifacts();

    assert_eq!(manifest.schema, MANIFEST_SCHEMA_V1_ALPHA3);
    assert_eq!(manifest.plan.schema, PLAN_SCHEMA_V1_ALPHA3);
    assert_eq!(manifest.plan.authority_mode.as_deref(), Some("prospective"));
    assert!(manifest.plan.active_checkpoint.is_none());
    assert!(manifest.plan.prospective_checkpoint.is_some());
    assert!(manifest.plan.prospective_activation.is_some());
    assert!(manifest.inputs.prospective_activation.is_some());
    let prefix = manifest.input_prefix.as_ref().unwrap();
    let visible = std::str::from_utf8(&packet[..prefix.bytes]).unwrap();
    assert!(visible.starts_with("# yo Prospective Activation Review Packet\n"));
    assert!(visible.contains("\"kind\":\"prospective_activation_request\""));
    assert!(visible.contains("\"kind\":\"prospective_checkpoint\""));
    assert!(visible.contains("\"kind\":\"prospective_active_record\""));
    assert_eq!(visible.matches("<<<YO-REVIEW-SECTION ").count(), 7);
    verify_canonical_artifacts(&manifest, &manifest_bytes, &packet, &inputs).unwrap();
}

// prospective plan을 옛 delivery profile과 섞거나 authority 표식을 지우면 다른 형식으로
// 추론하지 않고 closed manifest contract에서 거부한다.
#[test]
fn prospective_consumer_rejects_profile_and_authority_cross_use() {
    let (inputs, manifest, manifest_bytes, packet) = artifacts();

    let mut wrong_profile = manifest.clone();
    wrong_profile.plan.delivery_profile.id = DELIVERY_PROFILE_V1_ALPHA2.to_owned();
    assert!(
        verify_canonical_artifacts(&wrong_profile, &manifest_bytes, &packet, &inputs)
            .unwrap_err()
            .contains("unsupported contract")
    );

    let mut missing_authority = manifest;
    missing_authority.plan.authority_mode = None;
    assert!(
        verify_canonical_artifacts(&missing_authority, &manifest_bytes, &packet, &inputs)
            .unwrap_err()
            .contains("unsupported contract")
    );
}

// request schema와 delivery profile은 activation-request 유무를 함께 판별해 ordinary와
// prospective 경로가 누락·추론·교차 사용으로 서로 전환되지 않게 한다.
#[test]
fn prospective_request_requires_its_exact_closed_shape() {
    let base = serde_json::json!({
        "schema": "yo.slice-review-packet-request/v1alpha3",
        "context_request_path": ".local-exclude/context.json",
        "activation_request_path": ".local-exclude/activation.json",
        "required_knowledge_ids": ["methexis.review.bounded-packet"],
        "slice_contract_path": ".local-exclude/contract.json",
        "repository_authority_paths": ["CONTRIBUTING.md"],
        "validation_evidence": [{"name": "baseline", "path": ".local-exclude/evidence.json"}],
        "review_lenses": ["fresh-context"],
        "review_questions": ["Is the authority prospective only?"],
        "delivery_profile": "yo.slice-review-markdown/v1alpha3",
        "tokenizer_profile": "o200k_base/v1",
        "max_managed_payload_tokens": 1000
    });
    let request = serde_json::from_value(base.clone()).unwrap();
    validate_request(&request).unwrap();

    let mut missing_activation = base.clone();
    missing_activation
        .as_object_mut()
        .unwrap()
        .remove("activation_request_path");
    let request = serde_json::from_value(missing_activation).unwrap();
    assert!(
        validate_request(&request)
            .unwrap_err()
            .contains("must name")
    );

    let mut ordinary_cross_use = base;
    ordinary_cross_use["schema"] = "yo.slice-review-packet-request/v1".into();
    ordinary_cross_use["delivery_profile"] = DELIVERY_PROFILE_V1_ALPHA2.into();
    let request = serde_json::from_value(ordinary_cross_use).unwrap();
    assert!(
        validate_request(&request)
            .unwrap_err()
            .contains("must omit activation_request_path")
    );
}

// 구현 후보가 capability 파일까지 함께 추가해도 trusted develop에 그 exact 표식이 없으면
// 새 경로가 자기 구현을 리뷰하지 못하고 ordinary review를 요구한다.
#[test]
fn prospective_bootstrap_rejects_candidate_supplied_enablement() {
    let (repository, trusted, candidate, activation) =
        bootstrap_fixture("prospective-bootstrap-missing", false, false);

    let error = require_prospective_activation_boundary(
        &repository.path,
        &trusted,
        &candidate,
        &activation,
    )
    .unwrap_err();

    assert!(error.contains("not enabled by trusted develop"));
}

// trusted develop에 exact capability가 이미 있고 후보가 active record, Checkpoint, 두 manifest
// 만 바꾸는 후속 활성화라면 prospective review 경계를 통과한다.
#[test]
fn prospective_bootstrap_accepts_later_activation_only_candidate() {
    let (repository, trusted, candidate, activation) =
        bootstrap_fixture("prospective-bootstrap-enabled", true, false);

    require_prospective_activation_boundary(&repository.path, &trusted, &candidate, &activation)
        .unwrap();
}

// capability가 trusted여도 후보가 prospective 구현을 함께 바꾸면 activation-only 경계가
// 그 self-modifying candidate를 거부해 ordinary review로 돌린다.
#[test]
fn prospective_bootstrap_rejects_implementation_change() {
    let (repository, trusted, candidate, activation) =
        bootstrap_fixture("prospective-bootstrap-modified", true, true);

    let error = require_prospective_activation_boundary(
        &repository.path,
        &trusted,
        &candidate,
        &activation,
    )
    .unwrap_err();

    assert!(error.contains("activation-only candidate"));
}

// 실제 candidate의 active record는 JSON이 아니라 canonical YAML이다. Proposal capture는
// 그 bytes를 다시 해석하지 않고 Methexis 결과와 exact hash로 결속해 첫 실사용이 성공한다.
#[test]
fn prospective_capture_accepts_the_real_yaml_active_record_boundary() {
    let repository = TestRepository::new("prospective-review-yaml-active");
    repository.write(".gitignore", ".local-exclude/\n");
    repository.git(["add", ".gitignore"]);
    repository.git(["commit", "--quiet", "-m", "trusted base"]);
    let trusted_commit = crate::git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
        .unwrap()
        .trim()
        .to_owned();
    let checkpoint_id = format!("sha256:{}", "b".repeat(64));
    let checkpoint_bytes = b"schema: methexis.checkpoint/v1alpha1\n";
    let checkpoint_hash = digest(checkpoint_bytes);
    let predecessor = format!("sha256:{}", "a".repeat(64));
    let active_yaml = format!(
        "schema: methexis.active-checkpoint/v1alpha1\n\
         checkpoint_id: {checkpoint_id}\n\
         checkpoint_hash: {checkpoint_hash}\n\
         trusted_commit: {trusted_commit}\n\
         replaces_active_hash: {predecessor}\n\
         request_hash: sha256:{}\n",
        "c".repeat(64)
    );
    repository.write(
        format!(
            "methexis/checkpoints/{}.yaml",
            checkpoint_id.strip_prefix("sha256:").unwrap()
        ),
        std::str::from_utf8(checkpoint_bytes).unwrap(),
    );
    repository.write("methexis/active-checkpoint.yaml", &active_yaml);
    repository.git(["add", "methexis"]);
    repository.git(["commit", "--quiet", "-m", "activation candidate"]);
    let candidate_commit = crate::git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
        .unwrap()
        .trim()
        .to_owned();

    let activation_path = repository.write(
        ".local-exclude/activation.json",
        &format!(
            "{{\"schema\":\"methexis.activation-request/v1alpha1\",\
             \"checkpoint_id\":\"{checkpoint_id}\",\
             \"checkpoint_hash\":\"{checkpoint_hash}\",\
             \"replace_active_hash\":\"{predecessor}\"}}\n"
        ),
    );
    let activation = captured(
        activation_path.to_string_lossy().into_owned(),
        std::fs::read(&activation_path).unwrap(),
    )
    .unwrap();
    let context_request = captured(
        repository
            .write(".local-exclude/context-request.json", "{}\n")
            .to_string_lossy()
            .into_owned(),
        b"{}\n".to_vec(),
    )
    .unwrap();
    let context = captured(
        ".local-exclude/build/context.md".to_owned(),
        b"context\n".to_vec(),
    )
    .unwrap();
    repository.write(&context.path, std::str::from_utf8(&context.bytes).unwrap());
    let build_id = format!("sha256:{}", "d".repeat(64));
    let checkpoint = super::super::model::CheckpointIdentity {
        id: checkpoint_id,
        hash: checkpoint_hash,
        authority_basis_commit: trusted_commit.clone(),
    };
    let manifest_value = serde_json::json!({
        "schema": "methexis.context-manifest/v1alpha1",
        "build_id": build_id,
        "plan": {
            "checkpoint": checkpoint,
            "units": [{"id": "methexis.review.bounded-packet"}],
            "tokenizer_profile": "o200k_base/v1"
        },
        "context": {
            "path": "context.md",
            "hash": context.hash
        }
    });
    let mut manifest_bytes = serde_json::to_vec(&manifest_value).unwrap();
    manifest_bytes.push(b'\n');
    let manifest = captured(
        ".local-exclude/build/manifest.json".to_owned(),
        manifest_bytes,
    )
    .unwrap();
    repository.write(
        &manifest.path,
        std::str::from_utf8(&manifest.bytes).unwrap(),
    );
    let result = super::super::model::ContextResult {
        schema: "methexis.activation-review-context-result/v1alpha1".to_owned(),
        ok: true,
        operation: "resolve_activation_review_context".to_owned(),
        authority: "prospective".to_owned(),
        trusted_commit,
        build_id,
        context: artifact(&context),
        manifest: artifact(&manifest),
        checkpoint: Some(checkpoint),
        activation_request: Some(super::super::model::SemanticInput {
            path: ".local-exclude/activation.json".to_owned(),
            hash: activation.hash.clone(),
        }),
        predecessor_active_record_hash: Some(predecessor),
        proposed_active_record_hash: Some(digest(active_yaml.as_bytes())),
    };

    let (_, proposal) = capture_prospective_context_from_result(
        &repository.path,
        &candidate_commit,
        &activation_path,
        activation,
        context_request,
        result,
    )
    .unwrap();

    assert_eq!(
        proposal.proposed_active_record.bytes,
        active_yaml.as_bytes()
    );
}
