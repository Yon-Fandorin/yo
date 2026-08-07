use std::path::{Path, PathBuf};

use super::{model::Effect, prepare, prepare_with_post_binding, run};
use crate::test_support;

struct Fixture {
    repository: test_support::TestRepository,
    request: PathBuf,
    slice: String,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let repository = test_support::TestRepository::new(label);
        repository.write("base.txt", "base\n");
        repository.git(["add", "base.txt"]);
        repository.git(["commit", "--quiet", "-m", "test: base"]);
        std::fs::write(
            repository.path.join(".git/info/exclude"),
            ".local-exclude/\n",
        )
        .unwrap();
        let slice = format!("{label}-activation");
        let request = test_support::unique_path("activation-slice-request.json");
        std::fs::write(
            &request,
            format!(
                r#"{{
  "schema": "yo.activation-slice-request/v1",
  "slice": "{slice}",
  "owned_contracts": ["test.activation"],
  "dependencies": ["approved test revision"]
}}
"#
            ),
        )
        .unwrap();
        Self {
            repository,
            request,
            slice,
        }
    }

    fn prepare(&self) -> super::model::ResultRecord {
        prepare(&self.repository.path, &self.request).unwrap()
    }

    fn worktree(&self) -> PathBuf {
        self.repository
            .path
            .join(".local-exclude/worktrees")
            .join(&self.slice)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let worktree = self.worktree();
        if worktree.exists() {
            let _ = crate::git::command_in(&self.repository.path, false)
                .args(["worktree", "remove", "--force", "--"])
                .arg(&worktree)
                .status();
        }
        let _ = std::fs::remove_file(&self.request);
    }
}

// 한 요청이 정확한 develop commit에서 canonical activation 계약, Direct Slice
// branch와 worktree, Git binding을 모두 만들고 생성 경로를 결과에 고정한다.
#[test]
fn creates_and_binds_the_canonical_activation_slice() {
    let fixture = Fixture::new("activation-create");

    let result = fixture.prepare();

    assert_eq!(
        result.base,
        output(&fixture.repository.path, &["rev-parse", "develop"])
    );
    assert_eq!(
        result.branch_ref,
        format!("refs/heads/slice/direct/{}", fixture.slice)
    );
    assert!(matches!(result.effects.contract, Effect::Created));
    assert!(matches!(result.effects.branch, Effect::Created));
    assert!(matches!(result.effects.worktree, Effect::Created));
    assert!(matches!(result.effects.binding, Effect::Created));
    let contract: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&result.contract_path).unwrap()).unwrap();
    assert_eq!(
        contract["allowed_write_set"],
        serde_json::json!([
            "methexis/active-checkpoint.yaml",
            "methexis/checkpoints/**",
            "tools/methexis/examples/context-contract/manifest.json",
            "tools/methexis/examples/context-contract/stable-leaf-manifest.json"
        ])
    );
    assert_eq!(
        std::fs::read_to_string(result.binding_path).unwrap(),
        format!("{}\n", result.contract_path.display())
    );
}

// 첫 실행이 완성한 exact contract, worktree, branch, binding에 같은 요청을
// 다시 적용하면 아무 대상을 덮어쓰지 않고 모두 reused로 수렴한다.
#[test]
fn exact_retry_reuses_every_prepared_effect() {
    let fixture = Fixture::new("activation-retry");
    fixture.prepare();

    let result = fixture.prepare();

    assert!(matches!(result.effects.contract, Effect::Reused));
    assert!(matches!(result.effects.branch, Effect::Reused));
    assert!(matches!(result.effects.worktree, Effect::Reused));
    assert!(matches!(result.effects.binding, Effect::Reused));
}

// contract publication 뒤 branch만 남은 중단 상태는 exact base와 request가
// 일치할 때 새 worktree와 binding만 복구하여 수동 ref 삭제를 요구하지 않는다.
#[test]
fn retry_recovers_an_exact_contract_and_branch_without_a_worktree() {
    let fixture = Fixture::new("activation-partial");
    let first = fixture.prepare();
    fixture.repository.git([
        "worktree",
        "remove",
        "--force",
        "--",
        first.worktree_path.to_str().unwrap(),
    ]);

    let result = fixture.prepare();

    assert!(matches!(result.effects.contract, Effect::Reused));
    assert!(matches!(result.effects.worktree, Effect::Created));
    assert!(matches!(result.effects.binding, Effect::Created));
}

// 첫 setup이 contract와 branch까지만 남긴 뒤 develop가 전진해도 contract에
// 고정된 최초 base를 복구하여 동일 요청의 worktree와 binding만 완성한다.
#[test]
fn retry_keeps_the_pinned_base_after_develop_advances() {
    let fixture = Fixture::new("activation-advanced-develop");
    let first = fixture.prepare();
    fixture.repository.git([
        "worktree",
        "remove",
        "--force",
        "--",
        first.worktree_path.to_str().unwrap(),
    ]);
    fixture.repository.write("advanced.txt", "advanced\n");
    fixture.repository.git(["add", "advanced.txt"]);
    fixture
        .repository
        .git(["commit", "--quiet", "-m", "test: advance develop"]);

    let result = fixture.prepare();

    assert_eq!(result.base, first.base);
    assert!(matches!(result.effects.contract, Effect::Reused));
    assert!(matches!(result.effects.branch, Effect::Reused));
    assert!(matches!(result.effects.worktree, Effect::Created));
    assert!(matches!(result.effects.binding, Effect::Created));
}

// helper가 소유했다는 exact contract 증거 없이 같은 이름의 branch가 먼저
// 존재하면 이를 인수하지 않고 contract와 worktree를 만들기 전에 거절한다.
#[test]
fn rejects_a_preexisting_branch_without_the_exact_contract() {
    let fixture = Fixture::new("activation-ref-conflict");
    fixture
        .repository
        .git(["branch", &format!("slice/direct/{}", fixture.slice)]);

    let error = prepare(&fixture.repository.path, &fixture.request).unwrap_err();

    assert!(error.contains("without the exact activation contract"));
    assert!(!fixture.worktree().exists());
}

// exact contract 증거 없이 branch만 남은 상태는 ref가 같은 base여도 helper가
// 준비한 effect로 인수하지 않고 conflicting으로 재조회한다.
#[test]
fn failure_observes_a_branch_only_partial_setup() {
    let fixture = Fixture::new("activation-branch-only-state");
    fixture
        .repository
        .git(["branch", &format!("slice/direct/{}", fixture.slice)]);

    let encoded = run(&fixture.repository.path, &fixture.request).unwrap_err();
    let failure: serde_json::Value = serde_json::from_str(&encoded).unwrap();

    assert_eq!(failure["effects"]["contract"]["state"], "absent");
    assert_eq!(failure["effects"]["branch"]["state"], "conflicting");
    assert_eq!(failure["effects"]["worktree"]["state"], "absent");
    assert_eq!(failure["effects"]["binding"]["state"], "unknown");
}

// exact path/ref/base의 worktree가 있어도 owning contract가 사라졌다면 helper가
// 인수하지 않으므로 branch와 worktree를 모두 conflict로 대칭 보고한다.
#[test]
fn failure_observes_a_contractless_worktree_as_conflicting() {
    let fixture = Fixture::new("activation-contractless-worktree");
    let first = fixture.prepare();
    std::fs::remove_file(&first.contract_path).unwrap();

    let encoded = run(&fixture.repository.path, &fixture.request).unwrap_err();
    let failure: serde_json::Value = serde_json::from_str(&encoded).unwrap();

    assert_eq!(failure["effects"]["contract"]["state"], "absent");
    assert_eq!(failure["effects"]["branch"]["state"], "conflicting");
    assert_eq!(failure["effects"]["worktree"]["state"], "conflicting");
    assert_eq!(failure["effects"]["binding"]["state"], "unknown");
}

// failure 관찰은 오류 뒤 mutable request path나 움직인 HEAD를 다시 읽지 않고
// invocation 시작 때 캡처한 request bytes와 base를 그대로 보고한다.
#[test]
fn failure_observation_uses_the_invocation_snapshot() {
    let fixture = Fixture::new("activation-failure-snapshot");
    let request = std::fs::read(&fixture.request).unwrap();
    let initial_base = output(&fixture.repository.path, &["rev-parse", "HEAD"]);
    std::fs::write(
        &fixture.request,
        r#"{
  "schema": "yo.activation-slice-request/v1",
  "slice": "different-activation",
  "owned_contracts": ["different.activation"]
}
"#,
    )
    .unwrap();
    fixture.repository.write("advanced.txt", "advanced\n");
    fixture.repository.git(["add", "advanced.txt"]);
    fixture
        .repository
        .git(["commit", "--quiet", "-m", "test: move head"]);

    let failure = super::observation::failure(
        &fixture.repository.path,
        Some(&request),
        Some(initial_base.clone()),
        "injected failure".to_owned(),
    );

    assert_eq!(failure.slice.as_deref(), Some(fixture.slice.as_str()));
    assert_eq!(failure.base.as_deref(), Some(initial_base.as_str()));
    assert!(
        failure
            .contract_path
            .unwrap()
            .to_string_lossy()
            .contains(&fixture.slice)
    );
}

#[cfg(unix)]
// dangling symlink도 Path::exists의 false가 아니라 점유된 path conflict로
// 관찰하여 contract publication 전에 반복 불가능한 setup을 차단한다.
#[test]
fn dangling_worktree_symlink_is_a_structured_conflict() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new("activation-dangling-worktree");
    let parent = fixture.repository.path.join(".local-exclude/worktrees");
    std::fs::create_dir_all(&parent).unwrap();
    symlink(parent.join("missing-target"), fixture.worktree()).unwrap();

    let encoded = run(&fixture.repository.path, &fixture.request).unwrap_err();
    let failure: serde_json::Value = serde_json::from_str(&encoded).unwrap();

    assert_eq!(failure["effects"]["contract"]["state"], "absent");
    assert_eq!(failure["effects"]["worktree"]["state"], "conflicting");
}

#[cfg(unix)]
// coordination ancestor가 symlink면 component별 NOFOLLOW directory 생성이
// 외부 target 아래에 helper directory를 만들기 전에 실패한다.
#[test]
fn rejects_a_symlinked_local_directory_without_external_creation() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new("activation-directory-symlink");
    let external = test_support::unique_path("activation-external-directory");
    std::fs::create_dir(&external).unwrap();
    std::fs::write(
        fixture.repository.path.join(".git/info/exclude"),
        ".local-exclude\n.local-exclude/\n",
    )
    .unwrap();
    symlink(&external, fixture.repository.path.join(".local-exclude")).unwrap();

    let error = prepare(&fixture.repository.path, &fixture.request).unwrap_err();

    assert!(error.contains("without symlinks"), "{error}");
    assert!(!external.join("coordination").exists());
    std::fs::remove_dir(external).unwrap();
}

// request와 다른 bytes의 coordination contract가 있으면 과거 판단을
// 덮어쓰거나 그 위에 worktree를 붙이지 않고 충돌 경로를 보고한다.
#[test]
fn rejects_a_conflicting_coordination_contract() {
    let fixture = Fixture::new("activation-contract-conflict");
    let contract = fixture
        .repository
        .path
        .join(".local-exclude/coordination")
        .join(&fixture.slice)
        .join("slice-contract.json");
    std::fs::create_dir_all(contract.parent().unwrap()).unwrap();
    std::fs::write(&contract, b"{}\n").unwrap();

    let error = prepare(&fixture.repository.path, &fixture.request).unwrap_err();

    assert!(error.contains("existing activation Slice contract"));
    assert!(!fixture.worktree().exists());
}

// develop worktree가 dirty면 어느 commit과 working state를 기준으로 삼을지
// 모호하므로 local coordination directory나 Slice ref를 만들기 전에 멈춘다.
#[test]
fn rejects_a_dirty_integration_worktree_before_any_effect() {
    let fixture = Fixture::new("activation-dirty");
    fixture.repository.write("untracked.txt", "dirty\n");

    let error = prepare(&fixture.repository.path, &fixture.request).unwrap_err();

    assert!(error.contains("integration worktree must be clean"));
    assert!(!fixture.worktree().exists());
}

// helper는 direct activation 전용이므로 다른 named branch에서 실행해 그
// branch를 develop authority로 오인하거나 Wave 계약을 조용히 만들지 않는다.
#[test]
fn rejects_a_non_develop_integration_branch() {
    let fixture = Fixture::new("activation-wrong-branch");
    fixture.repository.git(["switch", "--quiet", "-c", "other"]);

    let error = prepare(&fixture.repository.path, &fixture.request).unwrap_err();

    assert!(error.contains("must run from `refs/heads/develop`"));
    assert!(!fixture.worktree().exists());
}

// contract를 먼저 발행한 뒤 Git branch 형식 오류로 영구 실패하지 않도록
// request의 Slice 이름이 유효한 전체 ref를 만드는지 side effect 전에 확인한다.
#[test]
fn rejects_an_invalid_git_branch_name_before_any_effect() {
    let fixture = Fixture::new("activation-invalid-ref");
    let invalid_slice = "activation..invalid";
    std::fs::write(
        &fixture.request,
        format!(
            r#"{{
  "schema": "yo.activation-slice-request/v1",
  "slice": "{invalid_slice}",
  "owned_contracts": ["test.activation"]
}}
"#
        ),
    )
    .unwrap();

    let error = prepare(&fixture.repository.path, &fixture.request).unwrap_err();

    assert!(error.contains("does not form a valid Git branch"));
    assert!(
        !fixture
            .repository
            .path
            .join(".local-exclude/coordination")
            .join(invalid_slice)
            .exists()
    );
}

// 중단 뒤 사용자가 수정한 기존 Slice worktree를 exact prepared effect로
// 오인하지 않고 dirty 상태를 보존한 채 binding setup을 중단한다.
#[test]
fn rejects_a_dirty_existing_slice_worktree() {
    let fixture = Fixture::new("activation-dirty-slice");
    fixture.prepare();
    std::fs::write(fixture.worktree().join("untracked.txt"), b"dirty\n").unwrap();

    let error = prepare(&fixture.repository.path, &fixture.request).unwrap_err();

    assert!(error.contains("activation Slice worktree must be clean"));
    assert_eq!(
        std::fs::read(fixture.worktree().join("untracked.txt")).unwrap(),
        b"dirty\n"
    );
}

// binding publication과 성공 응답 사이에 worktree가 바뀌면 마지막 postcondition이
// 성공을 거절하고, 이미 만들어진 binding은 structured retry 관찰 대상으로 남긴다.
#[test]
fn rejects_a_worktree_mutation_after_binding() {
    let fixture = Fixture::new("activation-post-binding-mutation");

    let error = prepare_with_post_binding(&fixture.repository.path, &fixture.request, |worktree| {
        std::fs::write(worktree.join("late.txt"), b"late\n").map_err(|write| write.to_string())
    })
    .unwrap_err();

    assert!(error.contains("activation Slice worktree must be clean"));
    let binding = crate::slice_contract::binding_path_for(&fixture.worktree()).unwrap();
    assert!(binding.exists());
}

// exact contract와 worktree가 있어도 binding이 다른 계약을 가리키면 이를
// 덮어쓰지 않고 기존 bytes를 보존하여 Slice 권한 전환을 명시적으로 막는다.
#[test]
fn rejects_and_preserves_a_conflicting_binding() {
    let fixture = Fixture::new("activation-binding-conflict");
    let first = fixture.prepare();
    std::fs::write(&first.binding_path, b"/different/contract.json\n").unwrap();

    let error = prepare(&fixture.repository.path, &fixture.request).unwrap_err();

    assert!(error.contains("already contains different bytes"));
    assert_eq!(
        std::fs::read(&first.binding_path).unwrap(),
        b"/different/contract.json\n"
    );
}

// mutation 실패 뒤 CLI error도 parse 가능한 versioned JSON으로 contract,
// branch, worktree, binding의 실제 관찰 상태를 모두 돌려준다.
#[test]
fn failure_reports_every_prepared_or_conflicting_effect() {
    let fixture = Fixture::new("activation-structured-failure");
    let first = fixture.prepare();
    std::fs::write(&first.binding_path, b"/different/contract.json\n").unwrap();

    let encoded = run(&fixture.repository.path, &fixture.request).unwrap_err();
    let failure: serde_json::Value = serde_json::from_str(&encoded).unwrap();

    assert_eq!(failure["schema"], "yo.activation-slice-bootstrap/v1");
    assert_eq!(failure["ok"], false);
    assert_eq!(failure["effects"]["contract"]["state"], "prepared");
    assert_eq!(failure["effects"]["branch"]["state"], "prepared");
    assert_eq!(failure["effects"]["worktree"]["state"], "prepared");
    assert_eq!(failure["effects"]["binding"]["state"], "conflicting");
}

#[cfg(unix)]
// request가 symlink면 외부에서 바뀌는 입력을 따라가지 않아 exact setup
// identity가 invocation 도중 다른 bytes로 바뀌는 경로를 닫는다.
#[test]
fn rejects_a_symlink_request() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new("activation-request-symlink");
    let link = test_support::unique_path("activation-request-link.json");
    symlink(&fixture.request, &link).unwrap();

    let error = prepare(&fixture.repository.path, &link).unwrap_err();

    assert!(error.contains("cannot open activation Slice request"));
    std::fs::remove_file(link).unwrap();
}

fn output(repository: &Path, arguments: &[&str]) -> String {
    crate::git::output_in(repository, arguments, false)
        .unwrap()
        .trim()
        .to_owned()
}
