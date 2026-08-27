use super::{
    check_candidate_diff_with_cutover, check_commit_with_cutover,
    check_prepare_commit_message_with_cutover, check_with_cutover, current_review_diff, validate,
};
use crate::{impact::ImpactInput, review_protocol::digest, test_support::TestRepository};

const DIFF_HASH: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_HASH: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

// 경계 lens는 high 모델, 기계적 품질 lens는 일반 모델을 기록할 수 있고 각 상세
// provider/session이 기존 compact reviewer와 같은 경우 exact diff coverage로 인정한다.
#[test]
fn accepts_high_boundary_and_mechanical_quality_model_coverage() {
    let message = format!(
        "feat: reviewed\n\n\
         Slice-Review: fresh-context - completed - codex/fresh-session - clear\n\
         Slice-Review: code-quality - completed - codex/quality-session - resolved\n\
         Review-Coverage: fresh-context - exact - \
         model-high/codex/gpt-5.6-sol/fresh-session - {DIFF_HASH}\n\
         Review-Coverage: code-quality - exact - \
         model/codex/gpt-5.6-luna/quality-session - {DIFF_HASH}\n"
    );

    assert_eq!(validate(&message, DIFF_HASH), Ok(()));
}

// delegated host review는 하위 Provider/Model을 Yo가 추측하지 않고 host/session만으로
// compact reviewer와 exact high-capability coverage를 연결합니다.
#[test]
fn accepts_delegated_host_review_without_provider_identity() {
    let message = format!(
        "feat: delegated reviewed\n\n\
         Slice-Review: fresh-context - completed - codex/fresh-session - clear\n\
         Slice-Review: code-quality - completed - grok/quality-session - clear\n\
         Review-Coverage: fresh-context - exact - \
         delegated-high/codex/fresh-session - {DIFF_HASH}\n\
         Review-Coverage: code-quality - exact - \
         delegated/grok/quality-session - {DIFF_HASH}\n"
    );

    assert_eq!(validate(&message, DIFF_HASH), Ok(()));
}

// ledger grammar는 target admission의 closed host 집합과 같아야 하며 미래 이름을
// high-capability review 증거로 선점하지 않습니다.
#[test]
fn rejects_an_unadmitted_delegated_host() {
    let message = format!(
        "feat: delegated reviewed\n\n\
         Slice-Review: fresh-context - completed - future/session - clear\n\
         Review-Coverage: fresh-context - exact - \
         delegated-high/future/session - {DIFF_HASH}\n"
    );

    assert!(
        validate(&message, DIFF_HASH)
            .unwrap_err()
            .contains("invalid Review-Coverage")
    );
}

// 사람이 정확한 patch와 lens를 직접 읽고 같은 identity로 verdict를 남긴 경우에는
// 모델 호출 없이도 fresh-context와 code-quality coverage를 모두 충족한다.
#[test]
fn accepts_human_exact_review_for_every_completed_lens() {
    let message = format!(
        "feat: human reviewed\n\n\
         Slice-Review: fresh-context - completed - human/yon - clear\n\
         Slice-Review: code-quality - completed - human/yon - clear\n\
         Review-Coverage: fresh-context - exact - human/yon - {DIFF_HASH}\n\
         Review-Coverage: code-quality - exact - human/yon - {DIFF_HASH}\n"
    );

    assert_eq!(validate(&message, DIFF_HASH), Ok(()));
}

// 누락 진단은 실제 완료된 lens만 예시로 렌더링하여 fresh-only, quality-only,
// integration-only Slice에 존재하지 않는 trailer를 추가하라고 잘못 안내하지 않는다.
#[test]
fn missing_coverage_usage_names_only_the_completed_lens() {
    for (lens, absent) in [
        ("fresh-context", ["code-quality", "integration"]),
        ("code-quality", ["fresh-context", "integration"]),
        ("integration", ["fresh-context", "code-quality"]),
    ] {
        let message =
            format!("feat: reviewed\n\nSlice-Review: {lens} - completed - human/yon - clear\n");
        let error = validate(&message, DIFF_HASH).unwrap_err();

        assert!(error.contains(&format!("Review-Coverage: {lens} - exact")));
        for other in absent {
            assert!(!error.contains(&format!("Review-Coverage: {other} - exact")));
        }
    }
}

// 일반 승인, 낮은 등급의 경계 리뷰, reviewer 불일치, 다른 diff, 누락·추가 lens는
// 완료된 exact review ledger로 오인되지 않도록 각각 구별되는 실패가 된다.
#[test]
fn rejects_non_exact_or_mismatched_coverage() {
    let cases = [
        (
            "missing",
            "feat: reviewed\n\n\
             Slice-Review: fresh-context - completed - human/yon - clear\n"
                .to_owned(),
            "bind every completed review lens",
        ),
        (
            "low boundary model",
            format!(
                "feat: reviewed\n\n\
                 Slice-Review: fresh-context - completed - codex/session - clear\n\
                 Review-Coverage: fresh-context - exact - \
                 model/codex/gpt-5.6-luna/session - {DIFF_HASH}\n"
            ),
            "require model-high or human",
        ),
        (
            "reviewer mismatch",
            format!(
                "feat: reviewed\n\n\
                 Slice-Review: fresh-context - completed - human/yon - clear\n\
                 Review-Coverage: fresh-context - exact - human/minseo - {DIFF_HASH}\n"
            ),
            "matching Slice-Review reviewer",
        ),
        (
            "different diff",
            format!(
                "feat: reviewed\n\n\
                 Slice-Review: fresh-context - completed - human/yon - clear\n\
                 Review-Coverage: fresh-context - exact - human/yon - {OTHER_HASH}\n"
            ),
            "does not match the accepted review surface",
        ),
        (
            "extra lens",
            format!(
                "feat: reviewed\n\n\
                 Slice-Review: fresh-context - completed - human/yon - clear\n\
                 Review-Coverage: fresh-context - exact - human/yon - {DIFF_HASH}\n\
                 Review-Coverage: code-quality - exact - human/yon - {DIFF_HASH}\n"
            ),
            "lenses must exactly match",
        ),
        (
            "standing approval is not a reviewer",
            format!(
                "feat: reviewed\n\n\
                 Slice-Review: fresh-context - completed - human/yon - clear\n\
                 Review-Coverage: fresh-context - exact - approval/standing - {DIFF_HASH}\n"
            ),
            "invalid Review-Coverage",
        ),
    ];

    for (name, message, expected) in cases {
        let error = validate(&message, DIFF_HASH).unwrap_err();
        assert!(error.contains(expected), "{name}: {error}");
    }
}

// cutover parent 위의 staged diff와 그 accepted commit diff가 같은 canonical hash를
// 만들며, 사람 exact trailer가 두 integration 경계 모두에서 검증되는지 확인한다.
#[test]
fn staged_and_committed_boundaries_share_the_exact_review_surface() {
    let repository = TestRepository::new("accepted-review-coverage");
    repository.write("base.txt", "base\n");
    repository.git(["add", "base.txt"]);
    repository.git(["commit", "--quiet", "-m", "test: coverage cutover"]);
    let cutover = crate::git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
        .unwrap()
        .trim()
        .to_owned();
    repository.write("tools/example/check.rs", "fn reviewed() {}\n");
    repository.git(["add", "tools/example/check.rs"]);
    let temporary_message = repository.write("message", "placeholder\n");
    let temporary_input = ImpactInput::load_from(
        &repository.path,
        temporary_message,
        None,
        Some("develop".to_owned()),
        true,
    )
    .unwrap();
    let expected = digest(&current_review_diff(&temporary_input).unwrap());
    let message = coverage_message(&expected);
    let message_path = repository.write("message", &message);
    let input = ImpactInput::load_from(
        &repository.path,
        message_path.clone(),
        None,
        Some("develop".to_owned()),
        true,
    )
    .unwrap();

    check_with_cutover(&input, &cutover).unwrap();
    repository.git([
        "commit",
        "--quiet",
        "--file",
        message_path.to_str().unwrap(),
    ]);
    let accepted = crate::git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
        .unwrap()
        .trim()
        .to_owned();

    check_commit_with_cutover(&repository.path, &accepted, "develop", &cutover).unwrap();
}

// accept 사전검증은 integration index를 만들기 전에도 전달받은 exact candidate diff의
// hash를 Review-Coverage trailer와 비교하여 오래된 검토 메시지를 먼저 거부합니다.
#[test]
fn candidate_diff_is_checked_before_the_index_is_mutated() {
    let repository = TestRepository::new("candidate-review-coverage");
    repository.write("base.txt", "base\n");
    repository.git(["add", "base.txt"]);
    repository.git(["commit", "--quiet", "-m", "test: coverage cutover"]);
    let cutover = crate::git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
        .unwrap()
        .trim()
        .to_owned();
    let candidate_diff = b"exact candidate diff\n";
    let message = coverage_message(&digest(candidate_diff));
    let message_path = repository.write("message", &message);
    let input = ImpactInput::load_from(
        &repository.path,
        message_path,
        None,
        Some("develop".to_owned()),
        true,
    )
    .unwrap();

    check_candidate_diff_with_cutover(&input, candidate_diff, &cutover).unwrap();
    assert!(
        check_candidate_diff_with_cutover(&input, b"different\n", &cutover)
            .unwrap_err()
            .contains("does not match the accepted review surface")
    );
}

// cutover 이후 accepted 브랜치에서는 amend/copy 동작을 거부하지만, 같은 저장소의
// Slice 워킹 브랜치와 일반 새 커밋은 계속 허용하여 정확한 리뷰 면만 보호한다.
#[test]
fn rejects_ambiguous_commit_reuse_only_on_accepted_history_after_cutover() {
    let repository = TestRepository::new("accepted-review-operation");
    repository.write("base.txt", "base\n");
    repository.git(["add", "base.txt"]);
    repository.git(["commit", "--quiet", "-m", "test: coverage cutover"]);
    let cutover = crate::git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
        .unwrap()
        .trim()
        .to_owned();
    let head = cutover.clone();

    check_prepare_commit_message_with_cutover(&repository.path, None, None, &cutover).unwrap();
    let error = check_prepare_commit_message_with_cutover(
        &repository.path,
        Some("commit"),
        Some(&head),
        &cutover,
    )
    .unwrap_err();
    assert!(error.contains("reject -m, -F, -t, -c, -C, and --amend"));

    repository.git(["switch", "--quiet", "-c", "slice/direct/rework"]);
    check_prepare_commit_message_with_cutover(
        &repository.path,
        Some("commit"),
        Some(&head),
        &cutover,
    )
    .unwrap();
}

#[cfg(unix)]
// xtask의 editor 경로는 source 없는 새 accepted commit을 만들지만, 뒤이은 실제
// Git --amend --file은 `message` source로 거부되어 HEAD와 staged 변경을 보존한다.
#[test]
fn accepted_commit_editor_allows_new_commit_but_amend_file_cannot_bypass_guard() {
    use std::os::unix::fs::PermissionsExt;

    let repository = TestRepository::new("accepted-review-amend-file");
    repository.write("base.txt", "base\n");
    repository.git(["add", "base.txt"]);
    repository.git(["commit", "--quiet", "-m", "test: coverage cutover"]);
    let cutover = crate::git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
        .unwrap()
        .trim()
        .to_owned();
    let hooks = repository.path.join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let hook = hooks.join("prepare-commit-msg");
    let executable = std::env::current_exe().unwrap();
    let script = format!(
        "#!/bin/sh\n\
         YO_XTASK_TEST_PREPARE_CHILD=1 \\\n         YO_XTASK_TEST_PREPARE_SOURCE=\"${{2-}}\" \\\n         YO_XTASK_TEST_PREPARE_COMMIT=\"${{3-}}\" \\\n         YO_XTASK_TEST_PREPARE_CUTOVER='{cutover}' \\\n         '{}' --exact \
         impact::review_coverage::tests::prepare_commit_message_hook_child --nocapture\n",
        executable.display()
    );
    std::fs::write(&hook, script).unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    repository.git(["config", "core.hooksPath", hooks.to_str().unwrap()]);
    let configured_template = repository.write("configured-template", "must not select template\n");
    repository.git([
        "config",
        "commit.template",
        configured_template.to_str().unwrap(),
    ]);
    let editor = repository.write(
        "accepted-message-editor",
        "#!/bin/sh\n\
         test \"$1\" = \"__accepted-commit-message-editor\" || exit 41\n\
         cp -- \"$YO_XTASK_ACCEPTED_COMMIT_MESSAGE\" \"$2\"\n",
    );
    std::fs::set_permissions(&editor, std::fs::Permissions::from_mode(0o755)).unwrap();
    repository.write("accepted.txt", "new accepted surface\n");
    repository.git(["add", "accepted.txt"]);
    let accepted_message = repository.write("accepted-message", "test: exact new commit\n");

    let template_error =
        super::commit::create_with_editor(&repository.path, &accepted_message, &editor)
            .unwrap_err();
    assert!(template_error.contains("requires commit.template to be unset"));
    repository.git(["config", "--unset", "commit.template"]);
    super::commit::create_with_editor(&repository.path, &accepted_message, &editor).unwrap();
    let accepted = crate::git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
        .unwrap()
        .trim()
        .to_owned();
    let parent = crate::git::output_in(&repository.path, &["rev-parse", "HEAD^"], false).unwrap();
    assert_eq!(parent.trim(), cutover);
    let committed_message = crate::git::output_in(
        &repository.path,
        &["show", "-s", "--format=%B", "HEAD"],
        false,
    )
    .unwrap();
    assert_eq!(committed_message.trim(), "test: exact new commit");

    repository.write("amended.txt", "must remain staged\n");
    repository.git(["add", "amended.txt"]);
    let message = repository.write("amend-message", "test: ambiguous amend\n");

    let output = crate::git::command_in(&repository.path, false)
        .args(["commit", "--amend", "--file"])
        .arg(&message)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("reject -m, -F, -t, -c, -C, and --amend")
            || stderr.contains("reject -m, -F, -t, -c, -C, and --amend"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    let head = crate::git::output_in(&repository.path, &["rev-parse", "HEAD"], false).unwrap();
    assert_eq!(head.trim(), accepted);
    let staged = crate::git::output_in(
        &repository.path,
        &["diff", "--cached", "--name-only"],
        false,
    )
    .unwrap();
    assert_eq!(staged.trim(), "amended.txt");
}

#[cfg(unix)]
// end-to-end Git hook의 자식 test process는 전달받은 실제 source/commit을 같은
// production guard로 검사하여 테스트 전용 shell 판단이 결과를 대신하지 않는다.
#[test]
fn prepare_commit_message_hook_child() {
    if std::env::var_os("YO_XTASK_TEST_PREPARE_CHILD").is_none() {
        return;
    }
    let optional = |name| std::env::var(name).ok().filter(|value| !value.is_empty());
    let source = optional("YO_XTASK_TEST_PREPARE_SOURCE");
    let commit = optional("YO_XTASK_TEST_PREPARE_COMMIT");
    let cutover = std::env::var("YO_XTASK_TEST_PREPARE_CUTOVER").unwrap();

    check_prepare_commit_message_with_cutover(
        &std::env::current_dir().unwrap(),
        source.as_deref(),
        commit.as_deref(),
        &cutover,
    )
    .unwrap();
}

fn coverage_message(hash: &str) -> String {
    format!(
        "feat: exact human review\n\n\
         Slice-Review: fresh-context - completed - human/yon - clear\n\
         Slice-Review: code-quality - completed - human/yon - clear\n\
         Review-Coverage: fresh-context - exact - human/yon - {hash}\n\
         Review-Coverage: code-quality - exact - human/yon - {hash}\n\
         Developer-Docs-Impact: none - runtime responsibilities remain unchanged\n"
    )
}
