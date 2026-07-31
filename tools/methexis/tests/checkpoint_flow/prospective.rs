//! Exact staged activation validation used by repository hooks.

use std::fs;

use serde_json::json;
use sha2::{Digest, Sha256};

use super::support::*;

const ARTIFACTS: [&str; 2] = [
    "tools/methexis/examples/context-contract/manifest.json",
    "tools/methexis/examples/context-contract/stable-leaf-manifest.json",
];

// active record가 staged되지 않은 일반 Slice에서는 hook 진입점이 별도 의미를 만들지 않고
// 기존 전체 check와 byte-for-byte 같은 결과를 반환한다.
#[test]
fn staged_activation_flag_falls_back_to_the_ordinary_check() {
    let repository = GitRepository::approved();

    let ordinary = success_json(repository.run(&["check"]));
    let hook = success_json(repository.run(&["check", "--staged-activation"]));

    assert_eq!(hook, ordinary);
}

#[cfg(unix)]
// activation과 무관한 non-UTF-8 파일이 staged되어도 active 경로 판별은 raw bytes로
// 먼저 끝내므로 hook은 오류를 만들지 않고 ordinary 전체 check와 같은 결과를 낸다.
#[test]
fn unrelated_non_utf8_staged_path_still_uses_the_ordinary_fallback() {
    use std::{
        ffi::{OsStr, OsString},
        os::unix::ffi::OsStringExt,
    };

    let repository = GitRepository::approved();
    let ordinary = success_json(repository.run(&["check"]));
    let filename = OsString::from_vec(b"unrelated-\xff".to_vec());
    fs::write(repository.path.join(&filename), b"unrelated\n").unwrap();
    repository.git_os(&[OsStr::new("add"), filename.as_os_str()]);

    let hook = success_json(repository.run(&["check", "--staged-activation"]));

    assert_eq!(hook, ordinary);
}

// 최초 activation에는 CAS 전임자가 없지만, exact Checkpoint와 전체 artifact set을
// stage하면 replacement와 같은 prospective trust 검사를 통과한다.
#[test]
fn exact_staged_initial_activation_has_no_predecessor() {
    let repository = GitRepository::approved();
    let create = repository.request("checkpoint-initial.json", &checkpoint_request());
    let checkpoint = success_json(repository.run(&["create-checkpoint", create.to_str().unwrap()]));
    let activation = repository.request(
        "activation-initial.json",
        &json!({
            "schema": "methexis.activation-request/v1alpha1",
            "checkpoint_id": checkpoint["checkpoint_id"],
            "checkpoint_hash": checkpoint["hash"]
        }),
    );
    success_json(repository.run(&["propose-activation", activation.to_str().unwrap()]));
    write_artifacts(&repository, &checkpoint);
    repository.git(&[
        "add",
        checkpoint["path"].as_str().unwrap(),
        "methexis/active-checkpoint.yaml",
        ARTIFACTS[0],
        ARTIFACTS[1],
    ]);

    let report = success_json(repository.run(&["check", "--staged-activation"]));

    assert_eq!(report["authority"], "prospective");
    assert!(report["current_active_hash"].is_null());
}

// 현재 trusted active의 정확한 hash를 계보로 남긴 Checkpoint와 두 파생 artifact만
// stage하면, 후보는 아직 authority가 아니어도 통합 전 전체 전환 검사를 통과한다.
#[test]
fn exact_staged_replacement_is_validated_as_prospective() {
    let repository = replacement_candidate();

    let transitional = failure_json(repository.run(&["check"]));
    assert_eq!(
        transitional["diagnostics"][0]["code"],
        "stale_tracked_artifact"
    );

    let report = success_json(repository.run(&["check", "--staged-activation"]));

    assert_eq!(report["schema"], "methexis.prospective-activation/v1alpha1");
    assert_eq!(report["authority"], "prospective");
    assert_eq!(report["checkpoint"], "active");
    assert_eq!(report["staged_paths"].as_array().map(Vec::len), Some(4));

    let context_request = repository.request(
        "prospective-context.json",
        &json!({
            "schema": "methexis.context-request/v1alpha1",
            "anchors": [{"kind": "knowledge_id", "value": KNOWLEDGE_ID}],
            "tokenizer_profile": "o200k_base/v1",
            "max_tokens": 8000
        }),
    );
    let context =
        success_json(repository.run(&["resolve-context", context_request.to_str().unwrap()]));
    assert_eq!(context["authority"], "trusted_integration");
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(
            repository
                .path
                .join(context["manifest"]["path"].as_str().unwrap()),
        )
        .unwrap(),
    )
    .unwrap();
    assert_ne!(
        manifest["plan"]["checkpoint"]["id"],
        report["checkpoint_id"]
    );

    repository.git(&["commit", "-m", "activate replacement fixture checkpoint"]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);
    let integrated = success_json(repository.run(&["check"]));
    assert_eq!(integrated["checkpoint"], "active");
}

// activation과 무관한 파일을 함께 stage하면 hook 전용 경로가 일반 변경을 숨기지 않고
// exact candidate 경계 위반으로 실패하는지 확인한다.
#[test]
fn staged_replacement_rejects_bundled_unrelated_changes() {
    let repository = replacement_candidate();
    fs::write(repository.path.join("README.md"), "unrelated\n").unwrap();
    repository.git(&["add", "README.md"]);

    let failure = failure_json(repository.run(&["check", "--staged-activation"]));

    assert_eq!(
        failure["error"]["code"],
        "invalid_activation_candidate_paths"
    );
}

// 두 manifest 중 하나라도 이전 Checkpoint provenance를 가리키면 candidate 전체를
// 거부해, 통합 뒤에야 stale 파생물을 발견하는 회귀를 막는다.
#[test]
fn staged_replacement_rejects_stale_artifact_provenance() {
    let repository = replacement_candidate();
    let path = repository.path.join(ARTIFACTS[0]);
    let mut artifact: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    artifact["plan"]["checkpoint"]["hash"] = json!(hash_bytes(b"stale"));
    fs::write(&path, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();
    repository.git(&["add", ARTIFACTS[0]]);

    let failure = failure_json(repository.run(&["check", "--staged-activation"]));

    assert_eq!(failure["error"]["code"], "stale_tracked_artifact");
}

#[cfg(unix)]
// index에 symlink mode로 올라간 artifact는 대상 문자열이 우연히 파싱 가능한지와 무관하게
// regular tracked evidence가 아니므로 blob을 해석하기 전에 거부한다.
#[test]
fn staged_replacement_rejects_symlinked_candidate_artifacts() {
    use std::os::unix::fs::symlink;

    let repository = replacement_candidate();
    let path = repository.path.join(ARTIFACTS[0]);
    fs::remove_file(&path).unwrap();
    symlink("manifest-target.json", &path).unwrap();
    repository.git(&["add", ARTIFACTS[0]]);

    let failure = failure_json(repository.run(&["check", "--staged-activation"]));

    assert_eq!(failure["error"]["code"], "unsupported_staged_entry");
}

// Git이 commit용 alternate index를 제공하면 prospective 검사는 default index가 아니라
// 그 exact proposal index를 pin해, hook이 검사한 바이트와 commit할 바이트를 일치시킨다.
#[test]
fn staged_replacement_uses_the_callers_exact_git_index() {
    let repository = replacement_candidate();
    let alternate = repository.path.join(".git/prospective-index");
    fs::copy(repository.path.join(".git/index"), &alternate).unwrap();
    repository.git(&["reset", "HEAD"]);

    let report = success_json(repository.run_with_env(
        &["check", "--staged-activation"],
        &[("GIT_INDEX_FILE", &alternate)],
    ));

    assert_eq!(report["authority"], "prospective");
}

// selected code Source가 현재 바이트와 달라 checkpoint가 degraded라면 proposal 형식과
// artifact provenance가 맞더라도 새 active 전이로 통합하도록 허용하지 않는다.
#[test]
fn staged_replacement_rejects_degraded_source_freshness() {
    let repository = replacement_candidate_from(GitRepository::code_approved());
    fs::write(
        repository.path.join("methexis/code-source.txt"),
        b"drifted\n",
    )
    .unwrap();

    let failure = failure_json(repository.run(&["check", "--staged-activation"]));

    assert_eq!(failure["error"]["code"], "prospective_checkpoint_degraded");
    assert_eq!(failure["error"]["affected_ids"], json!([KNOWLEDGE_ID]));
}

// working tree의 임의 active bytes를 CAS 전임자로 사용해 만든 proposal은
// canonical이어도 trusted active의 실제 hash와 다르므로 prospective 검사를 통과하지 못한다.
#[test]
fn staged_replacement_rejects_a_non_trusted_compare_and_swap_predecessor() {
    let repository = GitRepository::approved();
    let first_create = repository.request("checkpoint-first.json", &checkpoint_request());
    let first =
        success_json(repository.run(&["create-checkpoint", first_create.to_str().unwrap()]));
    let first_activation = repository.request(
        "activation-first.json",
        &json!({
            "schema": "methexis.activation-request/v1alpha1",
            "checkpoint_id": first["checkpoint_id"],
            "checkpoint_hash": first["hash"]
        }),
    );
    let first_active =
        success_json(repository.run(&["propose-activation", first_activation.to_str().unwrap()]));
    repository.git(&[
        "add",
        "methexis/checkpoints",
        "methexis/active-checkpoint.yaml",
    ]);
    repository.git(&["commit", "-m", "activate first fixture checkpoint"]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);
    repository.git(&["commit", "--allow-empty", "-m", "advance trusted approvals"]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);

    let second_create = repository.request("checkpoint-second.json", &checkpoint_request());
    let second =
        success_json(repository.run(&["create-checkpoint", second_create.to_str().unwrap()]));
    let second_activation = repository.request(
        "activation-second.json",
        &json!({
            "schema": "methexis.activation-request/v1alpha1",
            "checkpoint_id": second["checkpoint_id"],
            "checkpoint_hash": second["hash"],
            "replace_active_hash": first_active["hash"]
        }),
    );
    let second_active =
        success_json(repository.run(&["propose-activation", second_activation.to_str().unwrap()]));

    // 두 번째 proposal은 trusted ref에 넣지 않은 채 authority commit만 다시 전진시킨다.
    // 세 번째 proposal은 이 local-only active를 predecessor로 삼으므로 canonical이지만
    // 현재 trusted active에서 직접 이어지지는 않는다.
    repository.git(&[
        "commit",
        "--allow-empty",
        "-m",
        "advance trusted approvals again",
    ]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);
    let third_create = repository.request("checkpoint-third.json", &checkpoint_request());
    let third =
        success_json(repository.run(&["create-checkpoint", third_create.to_str().unwrap()]));
    let third_activation = repository.request(
        "activation-third.json",
        &json!({
            "schema": "methexis.activation-request/v1alpha1",
            "checkpoint_id": third["checkpoint_id"],
            "checkpoint_hash": third["hash"],
            "replace_active_hash": second_active["hash"]
        }),
    );
    success_json(repository.run(&["propose-activation", third_activation.to_str().unwrap()]));
    fs::remove_file(repository.path.join(second["path"].as_str().unwrap())).unwrap();
    write_artifacts(&repository, &third);
    repository.git(&[
        "add",
        third["path"].as_str().unwrap(),
        "methexis/active-checkpoint.yaml",
        ARTIFACTS[0],
        ARTIFACTS[1],
    ]);

    let failure = failure_json(repository.run(&["check", "--staged-activation"]));

    assert_eq!(
        failure["error"]["code"],
        "active_checkpoint_compare_and_swap_mismatch"
    );
}

fn replacement_candidate() -> GitRepository {
    replacement_candidate_from(GitRepository::approved())
}

fn replacement_candidate_from(repository: GitRepository) -> GitRepository {
    let first_create = repository.request("checkpoint-first.json", &checkpoint_request());
    let first =
        success_json(repository.run(&["create-checkpoint", first_create.to_str().unwrap()]));
    let first_activation = repository.request(
        "activation-first.json",
        &json!({
            "schema": "methexis.activation-request/v1alpha1",
            "checkpoint_id": first["checkpoint_id"],
            "checkpoint_hash": first["hash"]
        }),
    );
    let first_active =
        success_json(repository.run(&["propose-activation", first_activation.to_str().unwrap()]));
    repository.git(&[
        "add",
        "methexis/checkpoints",
        "methexis/active-checkpoint.yaml",
    ]);
    repository.git(&["commit", "-m", "activate first fixture checkpoint"]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);

    repository.git(&["commit", "--allow-empty", "-m", "advance trusted approvals"]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);
    let second_create = repository.request("checkpoint-second.json", &checkpoint_request());
    let second =
        success_json(repository.run(&["create-checkpoint", second_create.to_str().unwrap()]));
    let second_activation = repository.request(
        "activation-second.json",
        &json!({
            "schema": "methexis.activation-request/v1alpha1",
            "checkpoint_id": second["checkpoint_id"],
            "checkpoint_hash": second["hash"],
            "replace_active_hash": first_active["hash"]
        }),
    );
    success_json(repository.run(&["propose-activation", second_activation.to_str().unwrap()]));

    write_artifacts(&repository, &second);
    repository.git(&[
        "add",
        "methexis/checkpoints",
        "methexis/active-checkpoint.yaml",
        ARTIFACTS[0],
        ARTIFACTS[1],
    ]);
    repository
}

fn write_artifacts(repository: &GitRepository, checkpoint: &serde_json::Value) {
    let authority_basis_commit = repository.git(&["rev-parse", "develop"]).stdout;
    let authority_basis_commit = String::from_utf8(authority_basis_commit)
        .unwrap()
        .trim()
        .to_owned();
    for path in ARTIFACTS {
        let target = repository.path.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(
            target,
            serde_json::to_vec_pretty(&json!({
                "plan": {
                    "checkpoint": {
                        "id": checkpoint["checkpoint_id"],
                        "hash": checkpoint["hash"],
                        "authority_basis_commit": authority_basis_commit
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hash = String::from("sha256:");
    for byte in Sha256::digest(bytes) {
        hash.push(char::from(HEX[usize::from(byte >> 4)]));
        hash.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    hash
}
