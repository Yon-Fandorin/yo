use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::json;

use super::{BuildArtifacts, run_with_before_final, storage, verify_artifacts};
use crate::{
    checkpoint::{CheckpointService, TestRepository},
    context::hash::{StableHasher, digest},
};

static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

// 최초 관리 build를 읽은 뒤 같은 bytes의 새 디렉터리로 path를 교체하면 단순 byte 비교는 이를
// 놓친다. 최종 identity 재검증이 교체를 탐지해 성공을 거부하는지 확인한다.
#[test]
fn same_byte_directory_replacement_is_rejected_at_final_revalidation() {
    let fixture = Fixture::new();
    let displaced = fixture.root.join("displaced-build");
    let result = verify_artifacts(
        &fixture.root,
        &fixture.artifacts.build_id,
        "trusted-commit",
        &fixture.artifacts,
        || {},
        || {
            fs::rename(&fixture.directory, &displaced).unwrap();
            fs::create_dir(&fixture.directory).unwrap();
            write_artifacts(&fixture.directory, &fixture.artifacts);
            Ok(())
        },
    );

    assert_eq!(
        result.unwrap_err().code(),
        "context_build_verification_failed"
    );
}

// 실제 verifier 진입점이 최초 build 검증 뒤 원본 request capture를 다시 확인하는지 검증한다.
// 같은 요청 경로의 bytes를 바꾸면 관리 build가 정상이어도 성공할 수 없다.
#[test]
fn verifier_entrypoint_rejects_request_change_after_initial_build_check() {
    let (repository, request, build_id) = resolved_repository();

    let result = run_with_before_final(&repository.path, &request, &build_id, || {
        let mut bytes = fs::read(&request).unwrap();
        bytes.push(b' ');
        fs::write(&request, bytes).unwrap();
    });

    assert_eq!(
        result.unwrap_err().code(),
        "request_changed_during_verification"
    );
}

// 실제 verifier 진입점이 context에 사용한 mutable Source capture를 최종 재검증하는지 확인한다.
// 저장 artifact가 그대로여도 Source bytes가 바뀌면 그 artifact를 현재 입력의 검증 결과로 인정하지
// 않아야 한다.
#[test]
fn verifier_entrypoint_rejects_source_change_after_initial_build_check() {
    let (repository, request, build_id) = resolved_repository();
    let source = repository
        .path
        .join("methexis/sources/decision/tui.fixture.yaml");

    let result = run_with_before_final(&repository.path, &request, &build_id, || {
        let mut bytes = fs::read(&source).unwrap();
        bytes.push(b'\n');
        fs::write(&source, bytes).unwrap();
    });

    assert_eq!(
        result.unwrap_err().code(),
        "source_changed_during_resolution"
    );
}

// hash-pinned Librarian candidate도 독립 컴파일의 입력이므로 최초 capture 뒤 다시 확인해야 한다.
// 실제 verifier 진입점에서 candidate bytes를 바꾸면 저장 artifact가 정상이어도 성공하지 않는다.
#[test]
fn verifier_entrypoint_rejects_candidate_change_after_initial_build_check() {
    let (repository, request, build_id, candidate) = resolved_candidate_repository();

    let result = run_with_before_final(&repository.path, &request, &build_id, || {
        let mut bytes = fs::read(&candidate).unwrap();
        bytes.push(b' ');
        fs::write(&candidate, bytes).unwrap();
    });

    assert_eq!(
        result.unwrap_err().code(),
        "candidate_changed_during_resolution"
    );
}

// 실제 verifier 진입점이 처음 읽은 trusted commit과 active Checkpoint를 최종 guard까지 고정하는지
// 확인한다. 검증 도중 develop이 전진하면 새 snapshot으로 갈아타지 않고 실패해야 한다.
#[test]
fn verifier_entrypoint_rejects_trusted_ref_change_after_initial_build_check() {
    let (repository, request, build_id) = resolved_repository();

    let result = run_with_before_final(&repository.path, &request, &build_id, || {
        fs::write(repository.path.join("concurrent.txt"), "changed\n").unwrap();
        repository.git(&["add", "concurrent.txt"]);
        repository.git(&["commit", "-m", "advance authority during verification"]);
        repository.git(&["branch", "-f", "develop", "HEAD"]);
    });

    assert_eq!(
        result.unwrap_err().code(),
        "authority_changed_during_resolution"
    );
}

struct Fixture {
    root: PathBuf,
    directory: PathBuf,
    artifacts: BuildArtifacts,
}

impl Fixture {
    fn new() -> Self {
        let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "methexis-context-verify-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let artifacts = BuildArtifacts {
            build_id: format!("sha256:{}", "a".repeat(64)),
            context: b"context\n".to_vec(),
            context_hash: format!("sha256:{}", "b".repeat(64)),
            manifest: b"manifest\n".to_vec(),
            manifest_hash: format!("sha256:{}", "c".repeat(64)),
            tokens: 1,
            included_ids: Vec::new(),
        };
        let directory = storage::build_directory(&root, &artifacts.build_id);
        fs::create_dir_all(&directory).unwrap();
        write_artifacts(&directory, &artifacts);
        Self {
            root,
            directory,
            artifacts,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write_artifacts(directory: &std::path::Path, artifacts: &BuildArtifacts) {
    fs::write(directory.join("context.md"), &artifacts.context).unwrap();
    fs::write(directory.join("manifest.json"), &artifacts.manifest).unwrap();
}

fn resolved_repository() -> (TestRepository, PathBuf, String) {
    let repository = TestRepository::new();
    repository.approve(&["tui.context.base"]);
    let service = CheckpointService::new(&repository.path);
    let create_request = repository.request(
        "checkpoint.json",
        &json!({
            "schema": "methexis.checkpoint-request/v1alpha1",
            "roots": ["tui.context.base"]
        }),
    );
    let created = service.create(&create_request).unwrap();
    let created = serde_json::to_value(created).unwrap();
    let activation_request = repository.request(
        "activation.json",
        &json!({
            "schema": "methexis.activation-request/v1alpha1",
            "checkpoint_id": created["checkpoint_id"],
            "checkpoint_hash": created["hash"]
        }),
    );
    service.propose_activation(&activation_request).unwrap();
    repository.git(&[
        "add",
        "methexis/checkpoints",
        "methexis/active-checkpoint.yaml",
    ]);
    repository.git(&["commit", "-m", "activate context fixture"]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);
    let request = repository.request(
        "verify-context.json",
        &json!({
            "schema": "methexis.context-request/v1alpha1",
            "anchors": [{"kind": "knowledge_id", "value": "tui.context.base"}],
            "tokenizer_profile": "o200k_base/v1",
            "max_tokens": 8000
        }),
    );
    let resolved = crate::context::operations::resolve(&repository.path, &request).unwrap();
    let build_id = serde_json::to_value(resolved).unwrap()["build_id"]
        .as_str()
        .unwrap()
        .to_owned();
    (repository, request, build_id)
}

fn resolved_candidate_repository() -> (TestRepository, PathBuf, String, PathBuf) {
    let (repository, _, _) = resolved_repository();
    let request_hash = format!("sha256:{}", "1".repeat(64));
    let catalog_hash = format!("sha256:{}", "2".repeat(64));
    let compiler = "librarian/test";
    let candidate_items = json!([]);
    let mut identity = StableHasher::new(b"librarian.candidate-set/v1alpha1");
    identity.part(b"request_hash", request_hash.as_bytes());
    identity.part(b"catalog_hash", catalog_hash.as_bytes());
    identity.part(b"compiler", compiler.as_bytes());
    identity.part(
        b"candidates",
        &serde_json::to_vec(&candidate_items).unwrap(),
    );
    let candidate_value = json!({
        "schema": "librarian.candidate-set/v1alpha1",
        "ok": true,
        "candidate_set_id": identity.finish(),
        "request_hash": request_hash,
        "catalog_hash": catalog_hash,
        "compiler": compiler,
        "candidates": candidate_items,
        "unresolved_anchors": [],
        "truncated": 0
    });
    let candidate_directory = repository.path.join(".local-exclude/methexis/candidates");
    fs::create_dir_all(&candidate_directory).unwrap();
    let candidate = candidate_directory.join("verify.json");
    let mut candidate_bytes = serde_json::to_vec(&candidate_value).unwrap();
    candidate_bytes.push(b'\n');
    fs::write(&candidate, &candidate_bytes).unwrap();
    let request = repository.request(
        "verify-candidate-context.json",
        &json!({
            "schema": "methexis.context-request/v1alpha1",
            "candidates": {
                "path": ".local-exclude/methexis/candidates/verify.json",
                "hash": digest(&candidate_bytes)
            },
            "tokenizer_profile": "o200k_base/v1",
            "max_tokens": 8000
        }),
    );
    let resolved = crate::context::operations::resolve(&repository.path, &request).unwrap();
    let build_id = serde_json::to_value(resolved).unwrap()["build_id"]
        .as_str()
        .unwrap()
        .to_owned();
    (repository, request, build_id, candidate)
}
