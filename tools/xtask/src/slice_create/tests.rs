use std::path::PathBuf;

use super::{Effect, acquire_bootstrap_lock, encode_failure, prepare, run};
use crate::{git, test_support};

struct Fixture {
    repository: test_support::TestRepository,
    integration: PathBuf,
    source: PathBuf,
    slice: String,
    base: String,
}

impl Fixture {
    fn new(label: &str, write_rule: &str, owned_contract: &str) -> Self {
        Self::new_for_ref(label, write_rule, owned_contract, "refs/heads/develop")
    }

    fn new_for_ref(label: &str, write_rule: &str, owned_contract: &str, base_ref: &str) -> Self {
        let repository = test_support::TestRepository::new(label);
        repository.write("base.txt", "base\n");
        repository.commit_all("test: base");
        std::fs::write(
            repository.path.join(".git/info/exclude"),
            ".local-exclude/\n",
        )
        .unwrap();
        let base = output(&repository.path, &["rev-parse", "HEAD"]);
        let integration = repository
            .path
            .join(".local-exclude/worktrees/develop-integration");
        let integration_branch = base_ref.strip_prefix("refs/heads/").unwrap();
        if integration_branch != "develop" {
            repository.git(["branch", integration_branch]);
        }
        repository.git(["config", "core.bare", "true"]);
        repository.git([
            "worktree",
            "add",
            "--quiet",
            integration.to_str().unwrap(),
            integration_branch,
        ]);
        let slice = format!("{label}-slice");
        let source = test_support::unique_path("slice-create-contract.json");
        std::fs::write(
            &source,
            contract_for_ref(&slice, &base, base_ref, write_rule, owned_contract),
        )
        .unwrap();
        Self {
            repository,
            integration,
            source,
            slice,
            base,
        }
    }

    fn coordination_contract(&self) -> PathBuf {
        self.repository
            .path
            .join(".local-exclude/coordination")
            .join(&self.slice)
            .join("slice-contract.json")
    }

    fn slice_worktree(&self) -> PathBuf {
        self.repository
            .path
            .join(".local-exclude/worktrees")
            .join(&self.slice)
    }

    fn advance_develop(&self) {
        std::fs::write(self.integration.join("advanced.txt"), b"advanced\n").unwrap();
        command(&self.integration, &["add", "advanced.txt"]);
        command(
            &self.integration,
            &["commit", "--quiet", "-m", "test: advance develop"],
        );
    }
}

// Wave 계약은 develop을 암묵적으로 쓰지 않고 exact Wave integration worktree와
// `slice/<wave>/<slice>` branch identity를 선택합니다.
#[test]
fn creates_a_wave_slice_from_its_declared_integration_ref() {
    let fixture = Fixture::new_for_ref(
        "slice-create-wave",
        "tools/wave/**",
        "test.wave",
        "refs/heads/wave/w1-workflow",
    );

    let result = prepare(&fixture.integration, &fixture.source).unwrap();

    assert_eq!(
        result.branch_ref,
        format!("refs/heads/slice/w1-workflow/{}", fixture.slice)
    );
    assert_eq!(result.integration_worktree, fixture.integration);
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.source);
    }
}

// bare workspace의 유일한 develop worktree를 찾아 계약, Direct Slice branch,
// worktree와 binding을 한 번에 만들고 호출자가 바로 실행할 한 명령만 반환합니다.
#[test]
fn creates_and_binds_a_slice_from_the_unique_integration_worktree() {
    let fixture = Fixture::new("slice-create-success", "tools/owner/**", "test.owner");

    let result = prepare(&fixture.integration, &fixture.source).unwrap();

    assert_eq!(result.integration_worktree, fixture.integration);
    assert_eq!(result.base, fixture.base);
    assert_eq!(
        result.branch_ref,
        format!("refs/heads/slice/direct/{}", fixture.slice)
    );
    assert!(matches!(result.effects.contract, Effect::Created));
    assert!(matches!(result.effects.branch, Effect::Created));
    assert!(matches!(result.effects.worktree, Effect::Created));
    assert!(matches!(result.effects.binding, Effect::Created));
    assert_eq!(
        std::fs::read(&result.contract_path).unwrap(),
        std::fs::read(&fixture.source).unwrap()
    );
    assert_eq!(result.next_action.cwd, fixture.slice_worktree());
    assert_eq!(
        result.next_action.argv,
        ["cargo", "xtask", "check", "slice-scope"]
    );
    let branch = output(
        &fixture.slice_worktree(),
        &["symbolic-ref", "--quiet", "HEAD"],
    );
    assert_eq!(branch, result.branch_ref);
}

// 완성된 exact 효과에 같은 계약을 다시 적용해도 새 branch나 binding을 쓰지 않고
// 모두 reused로 수렴합니다.
#[test]
fn exact_retry_reuses_every_effect() {
    let fixture = Fixture::new("slice-create-retry", "tools/retry/**", "test.retry");
    prepare(&fixture.integration, &fixture.source).unwrap();

    let result = prepare(&fixture.integration, &fixture.source).unwrap();

    assert!(matches!(result.effects.contract, Effect::Reused));
    assert!(matches!(result.effects.branch, Effect::Reused));
    assert!(matches!(result.effects.worktree, Effect::Reused));
    assert!(matches!(result.effects.binding, Effect::Reused));
}

// contract와 branch까지 준비된 중단 상태는 수동 ref 삭제 없이 동일 base의
// worktree와 binding만 복구합니다.
#[test]
fn exact_retry_recovers_a_missing_worktree_and_binding() {
    let fixture = Fixture::new("slice-create-partial", "tools/partial/**", "test.partial");
    let first = prepare(&fixture.integration, &fixture.source).unwrap();
    command(
        &fixture.integration,
        &[
            "worktree",
            "remove",
            "--force",
            "--",
            first.worktree_path.to_str().unwrap(),
        ],
    );

    let result = prepare(&fixture.integration, &fixture.source).unwrap();

    assert!(matches!(result.effects.contract, Effect::Reused));
    assert!(matches!(result.effects.branch, Effect::Reused));
    assert!(matches!(result.effects.worktree, Effect::Created));
    assert!(matches!(result.effects.binding, Effect::Created));
}

// contract와 branch만 남은 partial Slice도 coordination lease를 계속 소유하므로,
// worktree가 없다는 이유로 겹치는 두 번째 Slice가 입장하지 못합니다.
#[test]
fn partial_contract_still_blocks_an_overlapping_lease() {
    let fixture = Fixture::new(
        "slice-create-partial-lease",
        "tools/partial-shared/**",
        "test.partial-shared",
    );
    let first = prepare(&fixture.integration, &fixture.source).unwrap();
    command(
        &fixture.integration,
        &[
            "worktree",
            "remove",
            "--force",
            "--",
            first.worktree_path.to_str().unwrap(),
        ],
    );
    let second = test_support::unique_path("slice-create-partial-lease-second.json");
    std::fs::write(
        &second,
        contract(
            "slice-create-partial-lease-second",
            &fixture.base,
            "tools/partial-shared/file.rs",
            "test.other",
        ),
    )
    .unwrap();

    let error = prepare(&fixture.integration, &second).unwrap_err();

    assert!(error.contains("overlapping write leases"), "{error}");
    std::fs::remove_file(second).unwrap();
}

// cooperating 생성 명령은 lease 검사와 effect publication 전체를 공용 Git lock으로
// 직렬화하여 둘 다 빈 coordination 상태를 관찰하는 경쟁을 막습니다.
#[test]
fn concurrent_bootstrap_is_rejected_before_any_slice_effect() {
    let fixture = Fixture::new(
        "slice-create-concurrent",
        "tools/concurrent/**",
        "test.concurrent",
    );
    let _lock = acquire_bootstrap_lock(&fixture.integration).unwrap();

    let error = prepare(&fixture.integration, &fixture.source).unwrap_err();

    assert!(
        error.contains("another cooperating Slice bootstrap"),
        "{error}"
    );
    assert!(!fixture.coordination_contract().exists());
    assert!(!fixture.slice_worktree().exists());
}

#[cfg(unix)]
// coordination root가 symlink면 외부 디렉터리를 lease registry로 읽거나 그 아래에
// 표준 계약을 발행하지 않고 target Slice effect 전에 거절합니다.
#[test]
fn symlinked_coordination_root_is_rejected_without_external_publication() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new(
        "slice-create-coordination-symlink",
        "tools/symlink/**",
        "test.symlink",
    );
    let external = test_support::unique_path("slice-create-external-coordination");
    std::fs::create_dir(&external).unwrap();
    let local = fixture.repository.path.join(".local-exclude");
    std::fs::create_dir_all(&local).unwrap();
    symlink(&external, local.join("coordination")).unwrap();

    let error = prepare(&fixture.integration, &fixture.source).unwrap_err();

    assert!(
        error.contains("must be a directory without symlinks"),
        "{error}"
    );
    assert!(!external.join(&fixture.slice).exists());
    std::fs::remove_dir(external).unwrap();
}

// 최초 생성은 현재 integration tip과 정확히 일치하는 계약만 받아, 오래된 계약이
// 뒤늦게 새 coordination lease를 차지하지 못하게 합니다.
#[test]
fn rejects_a_stale_base_before_any_effect() {
    let fixture = Fixture::new("slice-create-stale", "tools/stale/**", "test.stale");
    fixture.advance_develop();

    let error = prepare(&fixture.integration, &fixture.source).unwrap_err();

    assert!(error.contains("is stale"), "{error}");
    assert!(!fixture.coordination_contract().exists());
    assert!(!fixture.slice_worktree().exists());
}

// 이미 exact contract와 branch로 식별된 재실행은 develop가 전진한 뒤에도 처음
// 고정한 Slice base를 바꾸지 않습니다.
#[test]
fn exact_retry_keeps_its_pinned_base_after_develop_advances() {
    let fixture = Fixture::new(
        "slice-create-advanced-retry",
        "tools/advanced/**",
        "test.advanced",
    );
    prepare(&fixture.integration, &fixture.source).unwrap();
    fixture.advance_develop();

    let result = prepare(&fixture.integration, &fixture.source).unwrap();

    assert_eq!(result.base, fixture.base);
    assert!(matches!(result.effects.branch, Effect::Reused));
}

// 다른 active Slice의 contract ownership이나 path lease와 겹치면 새 표준 계약을
// 발행하기 전에 실패하여 동시 작업의 책임 경계를 보존합니다.
#[test]
fn rejects_overlapping_active_leases_before_publication() {
    let fixture = Fixture::new("slice-create-lease", "tools/shared/**", "test.shared");
    prepare(&fixture.integration, &fixture.source).unwrap();
    let second_slice = "slice-create-second-slice";
    let second = test_support::unique_path("slice-create-second-contract.json");
    std::fs::write(
        &second,
        contract(
            second_slice,
            &fixture.base,
            "tools/shared/file.rs",
            "test.other",
        ),
    )
    .unwrap();

    let error = prepare(&fixture.integration, &second).unwrap_err();

    assert!(error.contains("overlapping write leases"), "{error}");
    assert!(
        !fixture
            .repository
            .path
            .join(".local-exclude/coordination")
            .join(second_slice)
            .exists()
    );
    std::fs::remove_file(second).unwrap();
}

// 경로가 겹치지 않아도 같은 semantic contract owner를 두 active Slice가 동시에
// 주장하면 두 번째 bootstrap은 coordination publication 전에 멈춥니다.
#[test]
fn rejects_duplicate_active_contract_ownership() {
    let fixture = Fixture::new(
        "slice-create-owner-lease",
        "tools/first-owner/**",
        "test.shared-owner",
    );
    prepare(&fixture.integration, &fixture.source).unwrap();
    let second_slice = "slice-create-second-owner-slice";
    let second = test_support::unique_path("slice-create-second-owner-contract.json");
    std::fs::write(
        &second,
        contract(
            second_slice,
            &fixture.base,
            "tools/second-owner/**",
            "test.shared-owner",
        ),
    )
    .unwrap();

    let error = prepare(&fixture.integration, &second).unwrap_err();

    assert!(error.contains("both own contracts"), "{error}");
    assert!(
        !fixture
            .repository
            .path
            .join(".local-exclude/coordination")
            .join(second_slice)
            .exists()
    );
    std::fs::remove_file(second).unwrap();
}

// 실패 응답도 versioned JSON 안에 이미 준비된 효과를 다시 관찰해 호출자가
// ad-hoc cleanup을 추측하지 않도록 합니다.
#[test]
fn conflicting_binding_returns_structured_observed_state() {
    let fixture = Fixture::new("slice-create-failure", "tools/failure/**", "test.failure");
    let result = prepare(&fixture.integration, &fixture.source).unwrap();
    std::fs::write(&result.binding_path, b"/different/contract.json\n").unwrap();

    let encoded = run(&fixture.integration, &fixture.source).unwrap_err();
    let failure: serde_json::Value = serde_json::from_str(&encoded).unwrap();

    assert_eq!(failure["schema"], "yo.slice-bootstrap/v1alpha1");
    assert_eq!(failure["ok"], false);
    assert_eq!(failure["effects"]["contract"]["state"], "prepared");
    assert_eq!(failure["effects"]["branch"]["state"], "prepared");
    assert_eq!(failure["effects"]["worktree"]["state"], "prepared");
    assert_eq!(failure["effects"]["binding"]["state"], "conflicting");
}

// worktree까지 준비됐지만 binding publication 전 중단된 상태는 conflict가 아니라
// absent로 구분하여 동일 명령이 복구할 수 있는 효과임을 정확히 보여줍니다.
#[test]
fn missing_binding_is_observed_as_absent() {
    let fixture = Fixture::new(
        "slice-create-missing-binding",
        "tools/missing-binding/**",
        "test.missing-binding",
    );
    let result = prepare(&fixture.integration, &fixture.source).unwrap();
    std::fs::remove_file(&result.binding_path).unwrap();
    let bytes = std::fs::read(&fixture.source).unwrap();

    let encoded = encode_failure(
        &fixture.integration,
        Some(&bytes),
        "injected failure".to_owned(),
    )
    .unwrap();
    let failure: serde_json::Value = serde_json::from_str(&encoded).unwrap();

    assert_eq!(failure["effects"]["worktree"]["state"], "prepared");
    assert_eq!(failure["effects"]["binding"]["state"], "absent");
}

fn contract(slice: &str, base: &str, write_rule: &str, owned_contract: &str) -> String {
    contract_for_ref(
        slice,
        base,
        "refs/heads/develop",
        write_rule,
        owned_contract,
    )
}

fn contract_for_ref(
    slice: &str,
    base: &str,
    base_ref: &str,
    write_rule: &str,
    owned_contract: &str,
) -> String {
    format!(
        r#"{{
  "schema": "yo.slice-contract/v1",
  "slice": "{slice}",
  "base": "{base}",
  "base_ref": "{base_ref}",
  "owned_contracts": ["{owned_contract}"],
  "dependencies": [],
  "allowed_write_set": ["{write_rule}"],
  "focused_checks": ["cargo test -p owner"],
  "slice_close_checks": ["git diff --check"]
}}
"#
    )
}

fn command(repository: &std::path::Path, arguments: &[&str]) {
    let status = git::test_command_in(repository)
        .args(arguments)
        .status()
        .unwrap();
    assert!(status.success());
}

fn output(repository: &std::path::Path, arguments: &[&str]) -> String {
    git::output_in(repository, arguments, false)
        .unwrap()
        .trim()
        .to_owned()
}
