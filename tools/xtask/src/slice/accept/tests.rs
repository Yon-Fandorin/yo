use std::path::{Path, PathBuf};

use super::{
    ACCEPT_REQUEST_SCHEMA, ACCEPT_REQUEST_SCHEMA_V1_ALPHA3, AcceptRequest,
    COMMIT_CANDIDATE_HK_RECEIPT, COMMIT_GIT_HOOKS, Push, accept_result, canonical_diff,
    compose_message, effect_scope, fast_commit_verification, fast_effect_scope,
    integrate_candidate_with, integration_worktree,
};
use crate::{slice::gate as slice_gate, slice_worktree, test_support::TestRepository};

struct AcceptanceFixture {
    repository: TestRepository,
    candidate: PathBuf,
    message: PathBuf,
    integration_head: String,
    candidate_head: String,
}

impl AcceptanceFixture {
    fn new(label: &str) -> Self {
        let repository = TestRepository::new(label);
        repository.write("base.txt", "base\n");
        repository.git(["add", "base.txt"]);
        repository.git(["commit", "--quiet", "-m", "base"]);
        let integration_head = output(&repository.path, &["rev-parse", "HEAD"]);
        repository.git(["branch", "slice/direct/example"]);
        let candidate = crate::test_support::unique_path(label);
        repository.git([
            "worktree",
            "add",
            "--quiet",
            candidate.to_str().unwrap(),
            "slice/direct/example",
        ]);
        let changed = candidate.join("tools/example.rs");
        std::fs::create_dir_all(changed.parent().unwrap()).unwrap();
        std::fs::write(&changed, "pub fn accepted() {}\n").unwrap();
        git(&candidate, &["add", "tools/example.rs"]);
        git(&candidate, &["commit", "--quiet", "-m", "candidate"]);
        let candidate_head = output(&candidate, &["rev-parse", "HEAD"]);
        let message = crate::test_support::unique_path(&format!("{label}-message"));
        Self {
            repository,
            candidate,
            message,
            integration_head,
            candidate_head,
        }
    }

    fn write_message(&self, docs_impact: &str) {
        std::fs::write(
            &self.message,
            format!(
                "feat: accepted candidate\n\n\
                 Developer-Docs-Impact: {docs_impact}\n\
                 Slice-Review: fresh-context - completed - human/yon - clear\n\
                 Slice-Review: code-quality - completed - human/yon - clear\n"
            ),
        )
        .unwrap();
    }

    fn integrate(
        &self,
        commit: impl FnOnce(&Path, &Path) -> Result<(), String>,
    ) -> Result<String, String> {
        integrate_candidate_with(
            &self.repository.path,
            "refs/heads/develop",
            &self.integration_head,
            &self.candidate,
            "refs/heads/slice/direct/example",
            &self.integration_head,
            &self.candidate_head,
            &self.message,
            commit,
        )
    }
}

impl Drop for AcceptanceFixture {
    fn drop(&mut self) {
        let _ = crate::git::command_in(&self.repository.path, false)
            .args(["worktree", "remove", "--force", "--"])
            .arg(&self.candidate)
            .status();
        let _ = std::fs::remove_file(&self.message);
    }
}

// 게이트가 소유하는 review trailer만 기계적으로 덧붙이고 사람이 작성한 의미 설명과
// Developer Docs 판단은 그대로 보존한다.
#[test]
fn commit_message_appends_exact_gate_trailers() {
    let message = compose_message(
        b"feat: accept Slice\n\nExplain the accepted effect.\n\nDeveloper-Docs-Impact: updated\n",
        &[
            "Slice-Review: fresh-context - completed - codex/test - clear".to_owned(),
            "Review-Coverage: fresh-context - exact - model-high/codex/test - sha256:abc"
                .to_owned(),
        ],
    )
    .unwrap();

    assert_eq!(
        String::from_utf8(message).unwrap(),
        "feat: accept Slice\n\nExplain the accepted effect.\n\nDeveloper-Docs-Impact: updated\n\nSlice-Review: fresh-context - completed - codex/test - clear\nReview-Coverage: fresh-context - exact - model-high/codex/test - sha256:abc\n"
    );
}

// 한 번의 사용자 결정은 후보뿐 아니라 squash, exact push ref, close까지 모두 같은
// canonical scope에 포함해야 하며 remote/ref 변화가 곧 다른 승인이 됩니다.
#[test]
fn effect_scope_binds_every_orchestrated_mutation() {
    let scope = effect_scope("example", &"a".repeat(40), "origin", "refs/heads/develop");
    assert_eq!(
        scope,
        format!(
            "yo.slice-accept-effects/v1alpha1;slice=example;candidate={};squash=true;push=origin:refs/heads/develop;close=true",
            "a".repeat(40)
        )
    );
    assert_ne!(
        scope,
        effect_scope("example", &"a".repeat(40), "backup", "refs/heads/develop")
    );
}

// fast accept의 승인 범위는 push 생략과 commit 검증 방식을 명시적으로 고정하여
// 로컬 통합 승인이 나중에 원격 효과나 hook 우회로 확대되지 않게 합니다.
#[test]
fn fast_effect_scope_binds_no_push_and_commit_verification() {
    let scope = fast_effect_scope(
        "example",
        &"a".repeat(40),
        None,
        "refs/heads/develop",
        COMMIT_CANDIDATE_HK_RECEIPT,
    );
    assert_eq!(
        scope,
        format!(
            "yo.slice-accept-effects/v1alpha2;slice=example;candidate={};squash=true;push=none;commit_verification=candidate_hk_receipt;close=true",
            "a".repeat(40)
        )
    );
}

// 후보 base가 integration HEAD와 같고 현재 host/toolchain에서 exact 후보 diff를
// 선택한 hk receipt가 있을 때만 중복 Git hook을 생략합니다.
#[test]
fn fast_commit_requires_exact_current_hk_receipt() {
    let candidate = "a".repeat(40);
    let mut gate = slice_gate::ReadyGate {
        slice: "example".to_owned(),
        candidate_commit: candidate.clone(),
        diff_hash: "sha256:diff".to_owned(),
        validation: vec![slice_gate::ReadyValidation {
            name: "hk".to_owned(),
            argv: vec![
                "hk".to_owned(),
                "check".to_owned(),
                "--check".to_owned(),
                "--from-ref".to_owned(),
                "base".to_owned(),
                "--to-ref".to_owned(),
                candidate,
            ],
            status: "passed".to_owned(),
            reused: false,
            current_reusable_context: true,
        }],
        review_count: 1,
        known_unverified_environments: Vec::new(),
        commit_trailers: Vec::new(),
    };
    assert_eq!(
        fast_commit_verification(&gate, "base", "base"),
        COMMIT_CANDIDATE_HK_RECEIPT
    );
    gate.validation[0].current_reusable_context = false;
    assert_eq!(
        fast_commit_verification(&gate, "base", "base"),
        COMMIT_GIT_HOOKS
    );
    gate.validation[0].current_reusable_context = true;
    assert_eq!(
        fast_commit_verification(&gate, "base", "advanced"),
        COMMIT_GIT_HOOKS
    );
    gate.validation[0].argv = vec!["hk".to_owned(), "check".to_owned()];
    assert_eq!(
        fast_commit_verification(&gate, "base", "base"),
        COMMIT_GIT_HOOKS
    );
}

// 동결된 accept result에는 새 verification 필드를 보태지 않고, 새 v1alpha3
// 결과에서만 실제 선택된 hook 검증 방식을 공개합니다.
#[test]
fn accept_result_keeps_legacy_fields_frozen() {
    let mut request = AcceptRequest {
        schema: ACCEPT_REQUEST_SCHEMA.to_owned(),
        slice: "example".to_owned(),
        gate_request_path: "gate.json".to_owned(),
        gate_request_hash: format!("sha256:{}", "a".repeat(64)),
        message_source_path: "message.txt".to_owned(),
        message_source_hash: format!("sha256:{}", "b".repeat(64)),
        message_output_path: "message.out".to_owned(),
        close_prepare_request_path: "close.json".to_owned(),
        close_prepare_request_hash: format!("sha256:{}", "c".repeat(64)),
        close_plan_path: "plan.json".to_owned(),
        push: Some(Push {
            remote: "origin".to_owned(),
            reference: "refs/heads/develop".to_owned(),
        }),
        commit_verification: None,
        approval_scope: Some("legacy".to_owned()),
        effect_scope: None,
    };
    let legacy = accept_result(
        &request,
        "candidate",
        "accepted",
        "develop",
        COMMIT_GIT_HOOKS,
    );
    assert!(legacy.get("commit_verification").is_none());

    request.schema = ACCEPT_REQUEST_SCHEMA_V1_ALPHA3.to_owned();
    let current = accept_result(
        &request,
        "candidate",
        "accepted",
        "develop",
        COMMIT_GIT_HOOKS,
    );
    assert_eq!(current["commit_verification"], COMMIT_GIT_HOOKS);
}

// 사람이 준비한 원문에 이전 후보의 review trailer가 섞이면 ready 게이트의 exact
// trailer와 공존시키지 않고 입력 경계에서 바로 거부한다.
#[test]
fn commit_message_rejects_caller_review_trailers() {
    for trailer in ["Slice-Review:", "Review-Coverage:"] {
        let source = format!("feat: stale\n\n{trailer} old\n");
        let error = compose_message(
            source.as_bytes(),
            &["Slice-Review: none - exact".to_owned()],
        )
        .unwrap_err();
        assert!(error.contains("must omit gate-derived review trailers"));
    }
}

// accept는 호출한 Slice worktree를 integration으로 재사용하지 않고 공용 worktree
// registry에서 계약의 full branch ref를 가진 유일한 worktree를 선택합니다.
#[test]
fn integration_worktree_is_selected_by_full_branch_ref() {
    let repository = TestRepository::new("slice-accept-integration-worktree");
    repository.write("tracked.txt", "base\n");
    repository.git(["add", "tracked.txt"]);
    repository.git(["commit", "--quiet", "-m", "base"]);
    repository.git(["switch", "-q", "-c", "slice/direct/example"]);
    let integration = repository.path.join("develop-integration");
    repository.git([
        "worktree",
        "add",
        "-q",
        integration.to_str().unwrap(),
        "develop",
    ]);

    assert_eq!(
        integration_worktree(&repository.path, "refs/heads/develop").unwrap(),
        integration
    );
    assert!(
        integration_worktree(&repository.path, "refs/heads/missing")
            .unwrap_err()
            .contains("no registered integration worktree")
    );

    let listed = slice_worktree::worktrees(&repository.path).unwrap();
    assert!(
        listed
            .iter()
            .any(|worktree| worktree.branch.as_deref() == Some("refs/heads/develop"))
    );
}

// Developer Docs trailer처럼 staged path만 필요하던 검사를 실제 squash 전에 수행해
// 잘못된 메시지가 integration index를 더럽히지 않는지 전체 후보 흐름으로 확인한다.
#[test]
fn candidate_preflight_rejects_before_squash_mutation() {
    let fixture = AcceptanceFixture::new("slice-accept-preflight-before-squash");
    fixture.write_message("updated");

    let error = fixture
        .integrate(|_, _| panic!("commit must not run"))
        .unwrap_err();

    assert!(error.contains("docs/src has no staged change"));
    assert_eq!(
        output(&fixture.repository.path, &["rev-parse", "HEAD"]),
        fixture.integration_head
    );
    assert!(status(&fixture.repository.path).is_empty());
}

// exact squash 뒤 hook이나 commit 실행이 실패하면 원래 integration HEAD와 candidate
// diff가 그대로인 경우에만 자동 restore하여 다음 accept가 수동 정리 없이 재개됩니다.
#[test]
fn precommit_failure_restores_the_exact_squash() {
    let fixture = AcceptanceFixture::new("slice-accept-rollback");
    fixture.write_message("none - Developer Docs responsibilities remain accurate");

    let error = fixture
        .integrate(|_, _| Err("synthetic commit failure".to_owned()))
        .unwrap_err();

    assert!(error.contains("synthetic commit failure"));
    assert!(error.contains("automatically restored"));
    assert_eq!(
        output(&fixture.repository.path, &["rev-parse", "HEAD"]),
        fixture.integration_head
    );
    assert!(status(&fixture.repository.path).is_empty());
}

// 사전검증, exact squash, staged 재검증, commit까지 같은 helper를 통과시켜 실제
// integration commit의 canonical bytes가 후보 diff와 일치하는지 검증한다.
#[test]
fn candidate_integration_roundtrip_commits_the_exact_diff() {
    let fixture = AcceptanceFixture::new("slice-accept-roundtrip");
    fixture.write_message("none - Developer Docs responsibilities remain accurate");
    let expected = canonical_diff(
        &fixture.candidate,
        &fixture.integration_head,
        &fixture.candidate_head,
    )
    .unwrap();

    let accepted = fixture
        .integrate(|repository, message| {
            let status = crate::git::command_in(repository, false)
                .args(["commit", "--quiet", "--file"])
                .arg(message)
                .status()
                .map_err(|error| format!("cannot start test commit: {error}"))?;
            status
                .success()
                .then_some(())
                .ok_or_else(|| format!("test commit failed ({status})"))
        })
        .unwrap();

    assert_eq!(
        accepted,
        output(&fixture.repository.path, &["rev-parse", "HEAD"])
    );
    assert_eq!(
        canonical_diff(&fixture.repository.path, &format!("{accepted}^"), &accepted).unwrap(),
        expected
    );
    assert!(status(&fixture.repository.path).is_empty());
}

fn output(repository: &Path, arguments: &[&str]) -> String {
    crate::git::output_in(repository, arguments, false)
        .unwrap()
        .trim()
        .to_owned()
}

fn status(repository: &Path) -> Vec<u8> {
    crate::git::output_bytes_in(
        repository,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        false,
    )
    .unwrap()
}

fn git(repository: &Path, arguments: &[&str]) {
    assert!(
        crate::git::command_in(repository, false)
            .args(arguments)
            .status()
            .unwrap()
            .success()
    );
}
