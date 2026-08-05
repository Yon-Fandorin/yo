//! Registered ContextBuild manifest refresh for a prospective activation.

use std::{fs, path::PathBuf};

use serde_json::{Value, json};

use super::support::*;

const DIRECT_REQUEST: &str = "tools/methexis/examples/context-contract/direct-request.json";
const DIRECT_CONTEXT: &str = "tools/methexis/examples/context-contract/context.md";
const DIRECT_MANIFEST: &str = "tools/methexis/examples/context-contract/manifest.json";
const LEAF_REQUEST: &str = "tools/methexis/examples/context-contract/stable-leaf-request.json";
const LEAF_CONTEXT: &str = "tools/methexis/examples/context-contract/stable-leaf-context.md";
const LEAF_MANIFEST: &str = "tools/methexis/examples/context-contract/stable-leaf-manifest.json";
const SECOND_ID: &str = "tui.second";

struct RefreshCandidate {
    repository: GitRepository,
    activation_request: PathBuf,
    checkpoint: Value,
}

// 동일한 activation request를 두 번 실행하면 첫 실행만 manifest를 쓰고 두 번째 실행은
// 같은 BuildId와 바이트를 확인한 unchanged 결과를 내어 수동 hash 계산이 필요 없게 한다.
#[test]
fn refresh_is_deterministic_and_idempotent() {
    let candidate = refresh_candidate();

    let first = success_json(candidate.repository.run(&[
        "refresh-context-manifests",
        candidate.activation_request.to_str().unwrap(),
    ]));
    let first_bytes = fs::read(candidate.repository.path.join(DIRECT_MANIFEST)).unwrap();
    let second = success_json(candidate.repository.run(&[
        "refresh-context-manifests",
        candidate.activation_request.to_str().unwrap(),
    ]));

    assert_eq!(first["status"], "written");
    assert_eq!(second["status"], "unchanged");
    for index in 0..2 {
        assert_eq!(
            first["manifests"][index]["build_id"],
            second["manifests"][index]["build_id"]
        );
        assert_eq!(
            first["manifests"][index]["hash"],
            second["manifests"][index]["hash"]
        );
    }
    assert_ne!(
        first["manifests"][0]["build_id"],
        first["manifests"][1]["build_id"]
    );
    assert_eq!(
        fs::read(candidate.repository.path.join(DIRECT_MANIFEST)).unwrap(),
        first_bytes
    );
}

// prospective compile 결과의 context.md가 tracked golden과 달라지면 manifest만 새 값으로
// 덮어써 의미 변경을 숨기지 않고, 별도 payload 검수 Slice를 요구하는 오류로 멈춘다.
#[test]
fn refresh_rejects_payload_changes_before_mutating_manifests() {
    let candidate = refresh_candidate();
    fs::write(
        candidate.repository.path.join(DIRECT_CONTEXT),
        b"changed payload\n",
    )
    .unwrap();
    let before = fs::read(candidate.repository.path.join(DIRECT_MANIFEST)).unwrap();

    let failure = failure_json(candidate.repository.run(&[
        "refresh-context-manifests",
        candidate.activation_request.to_str().unwrap(),
    ]));

    assert_eq!(failure["error"]["code"], "context_payload_changed");
    assert_eq!(
        fs::read(candidate.repository.path.join(DIRECT_MANIFEST)).unwrap(),
        before
    );
}

// refresh가 생성한 두 manifest를 Checkpoint와 active record와 함께 stage하면 기존
// read-only staged activation 검사가 exact 네 경로와 prospective provenance를 승인한다.
#[test]
fn refreshed_manifests_complete_the_staged_activation_flow() {
    let candidate = refresh_candidate();
    success_json(candidate.repository.run(&[
        "refresh-context-manifests",
        candidate.activation_request.to_str().unwrap(),
    ]));
    candidate.repository.git(&[
        "add",
        candidate.checkpoint["path"].as_str().unwrap(),
        "methexis/active-checkpoint.yaml",
        DIRECT_MANIFEST,
        LEAF_MANIFEST,
    ]);

    let report = success_json(candidate.repository.run(&["check", "--staged-activation"]));

    assert_eq!(report["authority"], "prospective");
    assert_eq!(
        report["checkpoint_id"],
        candidate.checkpoint["checkpoint_id"]
    );
    assert_eq!(report["staged_paths"].as_array().map(Vec::len), Some(4));
}

// PREPARED journal이 남은 동안 manifest 두 개가 모두 새 bytes여도 staged reader는
// 그 순간을 완료된 atomic batch로 오인하지 않고 recovery 전까지 fail closed한다.
#[test]
fn staged_check_rejects_a_pending_manifest_transaction() {
    let candidate = refresh_candidate();
    success_json(candidate.repository.run(&[
        "refresh-context-manifests",
        candidate.activation_request.to_str().unwrap(),
    ]));
    candidate.repository.git(&[
        "add",
        candidate.checkpoint["path"].as_str().unwrap(),
        "methexis/active-checkpoint.yaml",
        DIRECT_MANIFEST,
        LEAF_MANIFEST,
    ]);
    let entries = [DIRECT_MANIFEST, LEAF_MANIFEST]
        .into_iter()
        .map(|path| {
            let bytes = fs::read(candidate.repository.path.join(path)).unwrap();
            json!({"path": path, "old": bytes, "new": bytes})
        })
        .collect::<Vec<_>>();
    fs::write(
        candidate
            .repository
            .path
            .join("tools/methexis/examples/context-contract/.manifest-refresh-transaction.json"),
        serde_json::to_vec(&json!({
            "schema": "methexis.context-manifest-refresh-transaction/v1alpha1",
            "state": "prepared",
            "entries": entries,
        }))
        .unwrap(),
    )
    .unwrap();

    let failure = failure_json(candidate.repository.run(&["check", "--staged-activation"]));

    assert_eq!(
        failure["error"]["code"],
        "manifest_refresh_transaction_pending"
    );
}

// activation request의 Checkpoint hash가 proposal과 하나라도 다르면 prospective authority를
// 만들지 않고 exact request/proposal 결합 오류로 실패한다.
#[test]
fn refresh_rejects_an_activation_request_for_different_checkpoint_bytes() {
    let candidate = refresh_candidate();
    let mut request: Value =
        serde_json::from_slice(&fs::read(&candidate.activation_request).unwrap()).unwrap();
    request["checkpoint_hash"] = json!(hash_bytes(b"different checkpoint"));
    let wrong = candidate
        .repository
        .request("activation-wrong-hash.json", &request);

    let failure = failure_json(
        candidate
            .repository
            .run(&["refresh-context-manifests", wrong.to_str().unwrap()]),
    );

    assert_eq!(failure["error"]["code"], "checkpoint_mismatch");
}

#[cfg(unix)]
// request 경로 자체가 symlink이면 대상 JSON이 올바르더라도 caller가 입력 capture를 다른
// 파일로 바꿀 수 있으므로 repository-local regular file 요구로 즉시 거부한다.
#[test]
fn refresh_rejects_a_symlinked_activation_request() {
    use std::os::unix::fs::symlink;

    let candidate = refresh_candidate();
    let link = candidate.repository.path.join("activation-link.json");
    symlink(&candidate.activation_request, &link).unwrap();

    let failure = failure_json(
        candidate
            .repository
            .run(&["refresh-context-manifests", link.to_str().unwrap()]),
    );

    assert_eq!(failure["error"]["code"], "symlink_forbidden");
}

fn refresh_candidate() -> RefreshCandidate {
    let repository = GitRepository::foundation();
    let second = repository.path.join("methexis/knowledge/second.md");
    fs::write(&second, format!("---\nschema: methexis.knowledge/v1alpha1\nid: {SECOND_ID}\nkind: rule\nowner: tui-architecture\nsources:\n  - id: tui.fixture\n    revision: sha256:3d3ff9057aadcbf2f44300bce0f97c5c84dc3c59a1a76e09eb012b299892f130\n---\n## Statement\n\nSecond registered context contract.\n")).unwrap();
    repository.git(&["add", "methexis/knowledge/second.md"]);
    repository.git(&["commit", "-m", "add second fixture knowledge"]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);
    repository.approve_units(&[KNOWLEDGE_ID, SECOND_ID]);
    repository.git(&["add", "methexis"]);
    repository.git(&["commit", "-m", "approve fixture knowledge"]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);
    repository.integrate_active_checkpoint_roots(&[KNOWLEDGE_ID, SECOND_ID]);
    install_context_contract(&repository);
    repository.git(&["add", "tools/methexis/examples/context-contract"]);
    repository.git(&["commit", "-m", "track fixture context contract"]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);

    let create = repository.request(
        "checkpoint-refresh.json",
        &json!({
            "schema": "methexis.checkpoint-request/v1alpha1",
            "roots": [KNOWLEDGE_ID, SECOND_ID]
        }),
    );
    let checkpoint = success_json(repository.run(&["create-checkpoint", create.to_str().unwrap()]));
    let current_active = fs::read(repository.path.join("methexis/active-checkpoint.yaml")).unwrap();
    let activation_request = repository.request(
        "activation-refresh.json",
        &json!({
            "schema": "methexis.activation-request/v1alpha1",
            "checkpoint_id": checkpoint["checkpoint_id"],
            "checkpoint_hash": checkpoint["hash"],
            "replace_active_hash": hash_bytes(&current_active)
        }),
    );
    success_json(repository.run(&["propose-activation", activation_request.to_str().unwrap()]));
    RefreshCandidate {
        repository,
        activation_request,
        checkpoint,
    }
}

fn install_context_contract(repository: &GitRepository) {
    let direct_request = json!({
        "schema": "methexis.context-request/v1alpha1",
        "anchors": [{"kind": "knowledge_id", "value": KNOWLEDGE_ID}],
        "tokenizer_profile": "o200k_base/v1",
        "max_tokens": 8000
    });
    let leaf_request = json!({
        "schema": "methexis.context-request/v1alpha1",
        "anchors": [{"kind": "knowledge_id", "value": SECOND_ID}],
        "tokenizer_profile": "o200k_base/v1",
        "max_tokens": 8000
    });
    for (path, request) in [
        (DIRECT_REQUEST, &direct_request),
        (LEAF_REQUEST, &leaf_request),
    ] {
        let target = repository.path.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, serde_json::to_vec_pretty(&request).unwrap()).unwrap();
    }
    let direct = success_json(repository.run(&["resolve-context", DIRECT_REQUEST]));
    let leaf = success_json(repository.run(&["resolve-context", LEAF_REQUEST]));
    let direct_context = fs::read(
        repository
            .path
            .join(direct["context"]["path"].as_str().unwrap()),
    )
    .unwrap();
    let direct_manifest = fs::read(
        repository
            .path
            .join(direct["manifest"]["path"].as_str().unwrap()),
    )
    .unwrap();
    let leaf_context = fs::read(
        repository
            .path
            .join(leaf["context"]["path"].as_str().unwrap()),
    )
    .unwrap();
    let leaf_manifest = fs::read(
        repository
            .path
            .join(leaf["manifest"]["path"].as_str().unwrap()),
    )
    .unwrap();
    fs::write(repository.path.join(DIRECT_CONTEXT), direct_context).unwrap();
    fs::write(repository.path.join(LEAF_CONTEXT), leaf_context).unwrap();
    fs::write(repository.path.join(DIRECT_MANIFEST), direct_manifest).unwrap();
    fs::write(repository.path.join(LEAF_MANIFEST), leaf_manifest).unwrap();
}

fn hash_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut output = String::from("sha256:");
    for byte in Sha256::digest(bytes) {
        use std::fmt::Write;
        write!(output, "{byte:02x}").unwrap();
    }
    output
}
