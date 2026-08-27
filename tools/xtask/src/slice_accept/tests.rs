use super::{compose_message, effect_scope, integration_worktree};
use crate::{slice_worktree, test_support::TestRepository};

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
