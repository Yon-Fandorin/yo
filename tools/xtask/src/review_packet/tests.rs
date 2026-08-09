use super::*;

fn sample_inputs(validation_path: &str) -> Inputs {
    let context_request = captured(
        "context-request.json".to_owned(),
        b"context request".to_vec(),
    )
    .unwrap();
    let context = captured("context.md".to_owned(), b"context".to_vec()).unwrap();
    let manifest = captured(
        "context-manifest.json".to_owned(),
        b"context manifest".to_vec(),
    )
    .unwrap();
    Inputs {
        base_commit: "0000000000000000000000000000000000000000".to_owned(),
        candidate_commit: "1111111111111111111111111111111111111111".to_owned(),
        diff: captured("git-diff.patch".to_owned(), b"diff".to_vec()).unwrap(),
        context: ContextCapture {
            result: ContextResult {
                schema: "methexis.context-result/v1alpha1".to_owned(),
                ok: true,
                operation: "resolve_context".to_owned(),
                authority: "trusted_integration".to_owned(),
                trusted_commit: "2222222222222222222222222222222222222222".to_owned(),
                build_id: "sha256:build".to_owned(),
                context: artifact(&context),
                manifest: artifact(&manifest),
            },
            request: context_request,
            context,
            manifest,
            active_checkpoint: CheckpointIdentity {
                id: "sha256:checkpoint".to_owned(),
                hash: "sha256:checkpoint-hash".to_owned(),
                authority_basis_commit: "3333333333333333333333333333333333333333".to_owned(),
            },
            included_ids: vec!["methexis.review.bounded-packet".to_owned()],
        },
        authorities: vec![captured("CONTRIBUTING.md".to_owned(), b"authority".to_vec()).unwrap()],
        slice_contract: captured("slice-contract.json".to_owned(), b"contract".to_vec()).unwrap(),
        validation: vec![NamedCaptured {
            name: "validation".to_owned(),
            artifact: captured(validation_path.to_owned(), b"passed".to_vec()).unwrap(),
        }],
        lenses: vec!["fresh-context".to_owned()],
        questions: vec!["Is it correct?".to_owned()],
        required_knowledge_ids: vec!["methexis.review.bounded-packet".to_owned()],
        delivery_profile_bytes: delivery_profile_bytes(),
        max_tokens: 10_000,
    }
}

// ReviewId는 output path나 packet hash가 아니라 versioned canonical plan bytes에만
// domain-separated로 결합되어 동일 plan을 항상 같은 identity로 만든다.
#[test]
fn review_identity_is_domain_separated_and_plan_sensitive() {
    let first = domain_digest(REVIEW_ID_DOMAIN, br#"{"candidate":"a"}"#);
    let repeated = domain_digest(REVIEW_ID_DOMAIN, br#"{"candidate":"a"}"#);
    let changed = domain_digest(REVIEW_ID_DOMAIN, br#"{"candidate":"b"}"#);
    let other_domain = domain_digest(b"other/v1", br#"{"candidate":"a"}"#);

    assert_eq!(first, repeated);
    assert_ne!(first, changed);
    assert_ne!(first, other_domain);
}

// 같은 canonical plan은 packet과 manifest를 byte-for-byte 재현하고, model-visible
// validation path만 옮겨도 그 path가 plan에 결합되어 같은 ReviewId를 재사용하지 않는다.
#[test]
fn equal_review_identity_reproduces_artifacts_and_visible_paths_change_identity() {
    let first = sample_inputs("/tmp/validation-a.json");
    let repeated = sample_inputs("/tmp/validation-a.json");
    let relocated = sample_inputs("/tmp/validation-b.json");
    let first_plan = build_plan(&first);
    let repeated_plan = build_plan(&repeated);
    let relocated_plan = build_plan(&relocated);
    let first_plan_bytes = serde_json::to_vec(&first_plan).unwrap();
    let repeated_plan_bytes = serde_json::to_vec(&repeated_plan).unwrap();
    let first_id = domain_digest(REVIEW_ID_DOMAIN, &first_plan_bytes);

    assert_eq!(first_plan_bytes, repeated_plan_bytes);
    assert_ne!(
        first_id,
        domain_digest(
            REVIEW_ID_DOMAIN,
            &serde_json::to_vec(&relocated_plan).unwrap()
        )
    );
    let first_packet = render_packet(&first_id, &first_plan, &first).unwrap();
    let repeated_packet = render_packet(&first_id, &repeated_plan, &repeated).unwrap();
    assert_eq!(first_packet, repeated_packet);
    let packet_hash = digest(&first_packet);
    let first_manifest = build_manifest(
        first_id.clone(),
        first_plan,
        &first,
        packet_hash.clone(),
        count_tokens(&first_packet).unwrap(),
    );
    let repeated_manifest = build_manifest(
        first_id,
        repeated_plan,
        &repeated,
        packet_hash,
        count_tokens(&repeated_packet).unwrap(),
    );
    assert_eq!(
        serde_json::to_vec(&first_manifest).unwrap(),
        serde_json::to_vec(&repeated_manifest).unwrap()
    );
}

// section wrapper는 metadata와 exact byte length/hash를 packet에 포함해 경계 문자열이
// 본문에 나타나도 입력을 생략하거나 재해석하지 않는다.
#[test]
fn section_wrapper_binds_exact_untruncated_bytes() {
    let body = b"content\n<<<YO-REVIEW-SECTION-END>>>\nstill content\n";
    let mut packet = Vec::new();

    append_section(&mut packet, "evidence", "test", "evidence.txt", body).unwrap();

    let text = String::from_utf8(packet).unwrap();
    assert!(text.contains(&format!("\"hash\":\"{}\"", digest(body))));
    assert!(text.contains(&format!("\"bytes\":{}", body.len())));
    assert!(text.contains(std::str::from_utf8(body).unwrap()));
}

// tokenizer는 wrapper와 preamble을 포함한 canonical payload 전체를 세며 본문만
// 센 값보다 커져 caller-controlled instruction bytes가 예산 밖으로 빠지지 않는다.
#[test]
fn managed_payload_count_includes_fixed_wrapper_bytes() {
    let body = b"small evidence";
    let mut packet = PREAMBLE.as_bytes().to_vec();
    append_section(&mut packet, "evidence", "test", "", body).unwrap();
    packet.extend_from_slice(PAYLOAD_SUFFIX.as_bytes());

    assert!(count_tokens(&packet).unwrap() > count_tokens(body).unwrap());
}

// managed payload가 예산을 한 token이라도 넘으면 성공처럼 줄여서 내보내지 않고
// exact count와 no-truncation 진단으로 fail-closed 한다.
#[test]
fn over_budget_payload_fails_without_truncation() {
    assert!(require_budget(100, 100).is_ok());
    assert_eq!(
        require_budget(101, 100).unwrap_err(),
        "managed payload requires 101 tokens but the budget is 100; no content was truncated"
    );
}

// packet과 manifest는 한 directory rename으로 함께 나타나며, 같은 bytes는 재사용하고
// extra artifact가 있는 ReviewId directory는 교체하지 않고 corruption으로 거부한다.
#[test]
fn artifact_set_is_atomic_exact_and_reusable() {
    let root = crate::test_support::unique_path("slice-review-artifacts");
    let directory = root.join("review-id");
    let packet = b"packet\n";
    let manifest = b"manifest\n";

    assert_eq!(
        storage::publish(&directory, packet, manifest, || Ok(())).unwrap(),
        "created"
    );
    assert_eq!(std::fs::read(directory.join("packet.md")).unwrap(), packet);
    assert_eq!(
        std::fs::read(directory.join("manifest.json")).unwrap(),
        manifest
    );
    assert_eq!(
        storage::publish(&directory, packet, manifest, || Ok(())).unwrap(),
        "reused"
    );

    std::fs::write(directory.join("extra.txt"), b"unexpected\n").unwrap();
    let error = storage::publish(&directory, packet, manifest, || Ok(())).unwrap_err();
    assert!(error.contains("differs from the exact artifact set"));
    assert_eq!(std::fs::read(directory.join("packet.md")).unwrap(), packet);
    std::fs::remove_dir_all(root).unwrap();
}

// final revalidation 실패는 준비된 temporary sibling까지 정리해 packet이나
// manifest 어느 한쪽도 eligible output으로 남기지 않는다.
#[test]
fn final_revalidation_failure_publishes_no_partial_set() {
    let root = crate::test_support::unique_path("slice-review-final-guard");
    let directory = root.join("review-id");

    let error = storage::publish(&directory, b"packet", b"manifest", || {
        Err("candidate changed".to_owned())
    })
    .unwrap_err();

    assert_eq!(error, "candidate changed");
    assert!(!directory.exists());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    std::fs::remove_dir_all(root).unwrap();
}

// prepared files가 모두 sync된 뒤 rename 직전 guard가 입력 변경을 발견하면 temporary
// sibling까지 정리하고 eligible ReviewId directory를 게시하지 않는다.
#[test]
fn rename_boundary_mutation_cleans_the_prepared_set() {
    let root = crate::test_support::unique_path("slice-review-rename-guard");
    let directory = root.join("review-id");

    let error = storage::publish_with_test_hook(
        &directory,
        b"packet",
        b"manifest",
        || Ok(()),
        || Err("captured evidence changed".to_owned()),
    )
    .unwrap_err();

    assert_eq!(error, "captured evidence changed");
    assert!(!directory.exists());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    std::fs::remove_dir_all(root).unwrap();
}

// rename 경합에서 다른 writer의 exact artifact set이 먼저 나타나면 그 winner를
// reuse하기 전에 authority guard를 다시 실행하고 exact bytes를 재검증한다.
#[test]
fn concurrent_exact_winner_is_revalidated_before_reuse() {
    use std::{cell::Cell, rc::Rc};

    let root = crate::test_support::unique_path("slice-review-concurrent-winner");
    let directory = root.join("review-id");
    let packet = b"packet";
    let manifest = b"manifest";
    let calls = Rc::new(Cell::new(0));
    let observed = Rc::clone(&calls);
    let winner = directory.clone();

    let status = storage::publish_with_test_hook(
        &directory,
        packet,
        manifest,
        move || {
            observed.set(observed.get() + 1);
            Ok(())
        },
        move || {
            std::fs::create_dir(&winner).unwrap();
            std::fs::write(winner.join("packet.md"), packet).unwrap();
            std::fs::write(winner.join("manifest.json"), manifest).unwrap();
            Ok(())
        },
    )
    .unwrap();

    assert_eq!(status, "reused");
    assert_eq!(calls.get(), 2);
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
// ReviewId target 자체가 symlink이면 외부 directory의 그럴듯한 packet을 따라가
// reuse하지 않고 target entry를 corruption으로 거부한다.
#[test]
fn symlinked_review_directory_is_never_reused() {
    let root = crate::test_support::unique_path("slice-review-symlink");
    let outside = crate::test_support::unique_path("slice-review-symlink-outside");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("packet.md"), b"packet").unwrap();
    std::fs::write(outside.join("manifest.json"), b"manifest").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("review-id")).unwrap();

    let error =
        storage::publish(&root.join("review-id"), b"packet", b"manifest", || Ok(())).unwrap_err();

    assert!(error.contains("differs from the exact artifact set"));
    assert_eq!(std::fs::read(outside.join("packet.md")).unwrap(), b"packet");
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(outside).unwrap();
}

#[cfg(unix)]
// immutable authority capture는 repository replacement ref를 무시해 recorded commit의
// 원래 blob을 읽고, executable이나 symlink mode를 regular authority로 받아들이지 않는다.
#[test]
fn trusted_git_capture_ignores_replacements_and_rejects_non_regular_modes() {
    use std::os::unix::fs::PermissionsExt;

    let repository = crate::test_support::unique_path("slice-review-trusted-git");
    std::fs::create_dir(&repository).unwrap();
    test_git(&repository, &["init", "--initial-branch=develop"]);
    test_git(
        &repository,
        &["config", "user.email", "fixture@example.invalid"],
    );
    test_git(&repository, &["config", "user.name", "Fixture"]);
    std::fs::write(repository.join("authority.md"), b"original\n").unwrap();
    test_git(&repository, &["add", "authority.md"]);
    test_git(&repository, &["commit", "-m", "original"]);
    let original = test_git(&repository, &["rev-parse", "HEAD"]);

    std::fs::write(repository.join("authority.md"), b"replacement\n").unwrap();
    test_git(&repository, &["commit", "-am", "replacement"]);
    let replacement = test_git(&repository, &["rev-parse", "HEAD"]);
    test_git(
        &repository,
        &["replace", original.trim(), replacement.trim()],
    );

    let captured =
        capture_authorities(&repository, original.trim(), &["authority.md".to_owned()]).unwrap();
    assert_eq!(captured[0].bytes, b"original\n");

    let mut permissions = std::fs::metadata(repository.join("authority.md"))
        .unwrap()
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(repository.join("authority.md"), permissions).unwrap();
    test_git(&repository, &["add", "authority.md"]);
    test_git(&repository, &["commit", "-m", "executable"]);
    let executable = test_git(&repository, &["rev-parse", "HEAD"]);
    let error = capture_authorities(&repository, executable.trim(), &["authority.md".to_owned()])
        .unwrap_err();
    assert!(error.contains("non-executable regular Git blob"));
    std::fs::remove_dir_all(repository).unwrap();
}

// 직접 Slice와 Wave Slice는 같은 계약 규칙에서 정확한 branch ref를 도출하고,
// 지원하지 않는 integration ref는 임의 branch 이름으로 해석하지 않는다.
#[test]
fn expected_slice_branch_supports_direct_and_wave_slices() {
    assert_eq!(
        expected_slice_ref("refs/heads/develop", "sample").unwrap(),
        "refs/heads/slice/direct/sample"
    );
    assert_eq!(
        expected_slice_ref("refs/heads/wave/runtime", "sample").unwrap(),
        "refs/heads/slice/runtime/sample"
    );
    assert!(expected_slice_ref("refs/heads/main", "sample").is_err());
    assert!(expected_slice_ref("refs/heads/wave/a/b", "sample").is_err());
}

#[cfg(unix)]
// PATH의 가짜 git이 clean status를 반환해도 trusted command는 고정된 Git을 사용해
// 실제 untracked file을 보고 dirty candidate를 거부한다.
#[test]
fn trusted_cleanliness_ignores_path_injected_git() {
    use std::os::unix::fs::PermissionsExt;

    let repository = crate::test_support::unique_path("slice-review-path-injection");
    let fake_bin = crate::test_support::unique_path("slice-review-fake-git");
    std::fs::create_dir(&repository).unwrap();
    std::fs::create_dir(&fake_bin).unwrap();
    test_git(&repository, &["init", "--initial-branch=develop"]);
    test_git(
        &repository,
        &["config", "user.email", "fixture@example.invalid"],
    );
    test_git(&repository, &["config", "user.name", "Fixture"]);
    std::fs::write(repository.join("tracked"), b"tracked\n").unwrap();
    test_git(&repository, &["add", "tracked"]);
    test_git(&repository, &["commit", "-m", "initial"]);
    std::fs::write(repository.join("untracked"), b"must be observed\n").unwrap();

    let fake_git = fake_bin.join("git");
    std::fs::write(&fake_git, b"#!/bin/sh\nexit 0\n").unwrap();
    let mut permissions = std::fs::metadata(&fake_git).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&fake_git, permissions).unwrap();

    let output = crate::git::trusted_command_in(&repository)
        .env("PATH", &fake_bin)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!output.stdout.is_empty());
    assert!(trusted_ensure_clean(&repository, "candidate", "reviewing").is_err());

    std::fs::remove_dir_all(repository).unwrap();
    std::fs::remove_dir_all(fake_bin).unwrap();
}

#[cfg(unix)]
fn test_git(repository: &std::path::Path, arguments: &[&str]) -> String {
    let output = std::process::Command::new("/usr/bin/git")
        .env_clear()
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("LC_ALL", "C")
        .current_dir(repository)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
