use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use super::{
    Authority, GitEnvironment, StageReport, ValidationMode, classify_staged_paths,
    handle_staged_check_output, methexis_check_arguments,
    require_exact_semantic_worktree_with_environment, validation_mode_with_environment,
};

const DRAFT_REPORT: &str = r#"{
    "schema": "methexis.check/v1alpha1",
    "ok": true,
    "authority": "draft",
    "checks": [{"check": "records", "status": "passed"}],
    "units": [{"id": "secretly-large-unit", "body": "content must not escape"}],
    "diagnostics": []
}"#;

// Source와 Knowledge만 staged 된 집합만 의미 후보 경로를 선택하고, Projection이나
// 다른 authority 파일이 섞이면 기존 완전 검증 경로를 유지한다.
#[test]
fn semantic_candidate_mode_requires_an_exact_source_and_knowledge_path_cohort() {
    assert_eq!(
        classify_staged_paths(b"methexis/sources/decision/one.yaml\0methexis/knowledge/one.md\0"),
        ValidationMode::SemanticCandidate
    );
    assert_eq!(
        classify_staged_paths(b"methexis/knowledge/one.md\0methexis/review-projections/one.md\0"),
        ValidationMode::Complete
    );
    assert_eq!(classify_staged_paths(b""), ValidationMode::Complete);
}

// 의미 후보는 records와 relations만 실행하고, 그 밖의 staged 집합은 활성화 인식과
// 전체 authority 검증을 보존하는 기존 --staged-activation 진입점을 사용한다.
#[test]
fn validation_mode_selects_the_narrowest_complete_methexis_command() {
    assert_eq!(
        methexis_check_arguments(ValidationMode::SemanticCandidate),
        [
            "run",
            "--quiet",
            "--locked",
            "-p",
            "methexis",
            "--",
            "check",
            "--only",
            "records,relations",
        ]
    );
    assert_eq!(
        methexis_check_arguments(ValidationMode::Complete),
        [
            "run",
            "--quiet",
            "--locked",
            "-p",
            "methexis",
            "--",
            "check",
            "--staged-activation",
        ]
    );
}

// rename 감지를 끈 실제 staged path 관찰이 Projection 원본과 Knowledge 목적지를 모두
// 보므로 authority 파일을 의미 후보로 오분류하지 않는지 임시 Git 저장소로 확인한다.
#[test]
fn authority_to_semantic_rename_keeps_complete_validation() {
    let repository = TestRepository::new("rename");
    repository.write("methexis/review-projections/one.md", "projection\n");
    repository.commit_all("base");
    fs::create_dir_all(repository.path.join("methexis/knowledge")).unwrap();
    repository.git([
        "mv",
        "methexis/review-projections/one.md",
        "methexis/knowledge/one.md",
    ]);

    assert_eq!(
        validation_mode_with_environment(&repository.path, GitEnvironment::Isolated).unwrap(),
        ValidationMode::Complete
    );
}

// staged 의미 후보와 다른 tracked working bytes가 있으면 검사 대상이 index와 같지
// 않으므로 records·relations 실행 전에 닫힌 실패를 반환한다.
#[test]
fn semantic_candidate_rejects_unstaged_methexis_changes() {
    let repository = staged_semantic_repository("unstaged");
    repository.write("methexis/knowledge/one.md", "unstaged\n");

    let error = require_exact_semantic_worktree_with_environment(
        &repository.path,
        GitEnvironment::Isolated,
    )
    .unwrap_err();
    assert!(error.contains("unstaged tracked changes"), "{error}");
}

// 일반 untracked와 .gitignore가 숨기는 ignored 파일 모두 Methexis loader가 읽을 수
// 있으므로 exact staged 후보 검증에서는 둘 다 같은 닫힌 실패로 거부한다.
#[test]
fn semantic_candidate_rejects_untracked_and_ignored_methexis_paths() {
    let untracked = staged_semantic_repository("untracked");
    untracked.write("methexis/knowledge/extra.md", "extra\n");
    let error =
        require_exact_semantic_worktree_with_environment(&untracked.path, GitEnvironment::Isolated)
            .unwrap_err();
    assert!(error.contains("untracked or ignored"), "{error}");

    let ignored = staged_semantic_repository("ignored");
    ignored.write(".gitignore", "methexis/knowledge/ignored.md\n");
    ignored.git(["add", ".gitignore"]);
    ignored.git(["commit", "-qm", "ignore rule"]);
    ignored.write("methexis/knowledge/one.md", "candidate-again\n");
    ignored.git(["add", "methexis/knowledge/one.md"]);
    ignored.write("methexis/knowledge/ignored.md", "ignored\n");
    let error =
        require_exact_semantic_worktree_with_environment(&ignored.path, GitEnvironment::Isolated)
            .unwrap_err();
    assert!(error.contains("untracked or ignored"), "{error}");
}

#[cfg(unix)]
// Git의 NUL 구분 raw path를 UTF-8로 바꾸지 않으므로 Unix의 비 UTF-8 Knowledge
// 파일도 panic이나 누락 없이 의미 후보 cohort로 분류한다.
#[test]
fn non_utf8_semantic_path_is_classified_from_raw_git_bytes() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let repository = TestRepository::new("non-utf8");
    fs::create_dir_all(repository.path.join("methexis/knowledge")).unwrap();
    let path = repository
        .path
        .join("methexis/knowledge")
        .join(OsString::from_vec(b"unit-\xff.md".to_vec()));
    fs::write(path, b"body\n").unwrap();
    repository.git(["add", "methexis/knowledge"]);

    assert_eq!(
        validation_mode_with_environment(&repository.path, GitEnvironment::Isolated).unwrap(),
        ValidationMode::SemanticCandidate
    );
}

// 보고서의 authority 문자열이 draft와 prospective의 서로 다른 타입 값으로
// 역직렬화되는지 직접 비교해 호출자가 두 상태를 혼동하지 않게 한다.
#[test]
fn trusts_only_the_authority_in_the_single_methexis_report() {
    let prospective: StageReport = serde_json::from_str(
        r#"{"schema":"methexis.prospective-activation/v1alpha1","ok":true,"authority":"prospective","affected_ids":[]}"#,
    )
    .unwrap();
    let ordinary: StageReport = serde_json::from_str(DRAFT_REPORT).unwrap();

    assert_eq!(
        prospective.summary().unwrap().authority,
        Authority::Prospective
    );
    assert_eq!(ordinary.summary().unwrap().authority, Authority::Draft);
}

// 실제 활성화 보고서는 일반 검사 보고서의 checks/units/diagnostics 배열을 갖지
// 않는다. 전용 schema로 이를 구분해 affected_ids 개수만 bounded summary로 내보낸다.
#[test]
fn prospective_activation_uses_its_own_report_shape() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let authority = handle_staged_check_output(
        true,
        "exit status: 0",
        br#"{"schema":"methexis.prospective-activation/v1alpha1","ok":true,"operation":"check_staged_activation","authority":"prospective","affected_ids":["one","two"]}"#,
        b"",
        &mut stdout,
        &mut stderr,
    )
    .unwrap();

    assert_eq!(
        stdout,
        b"{\"schema\":\"yo.methexis-stage-summary/v1\",\"ok\":true,\"authority\":\"prospective\",\"checks\":1,\"units\":2,\"diagnostics\":0}\n"
    );
    assert!(stderr.is_empty());
    assert_eq!(authority, Authority::Prospective);
}

// schema와 authority 조합이 뒤바뀐 보고서는 성공 프로세스라도 신뢰하지 않아
// 도구 버전 불일치나 잘못 연결된 출력이 검증 통과로 오인되지 않게 한다.
#[test]
fn rejects_schema_authority_mismatch() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let error = handle_staged_check_output(
        true,
        "exit status: 0",
        br#"{"schema":"methexis.prospective-activation/v1alpha1","ok":true,"authority":"draft","affected_ids":[]}"#,
        b"",
        &mut stdout,
        &mut stderr,
    )
    .unwrap_err();

    assert_eq!(
        error,
        "staged Methexis report schema and authority disagree"
    );
}

// 성공 처리 경계가 전체 unit 본문과 식별자를 전달하지 않고 authority와 개수만
// stdout에 쓰며, 성공 stderr는 그대로 전달하는지 바이트 단위로 확인한다.
#[test]
fn successful_check_forwards_only_bounded_summary_and_stderr() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let authority = handle_staged_check_output(
        true,
        "exit status: 0",
        DRAFT_REPORT.as_bytes(),
        b"compiler warning\n",
        &mut stdout,
        &mut stderr,
    )
    .unwrap();

    assert_eq!(
        stdout,
        b"{\"schema\":\"yo.methexis-stage-summary/v1\",\"ok\":true,\"authority\":\"draft\",\"checks\":1,\"units\":1,\"diagnostics\":0}\n"
    );
    assert_eq!(stderr, b"compiler warning\n");
    assert_eq!(authority, Authority::Draft);
    assert!(
        !stdout
            .windows(b"secretly-large-unit".len())
            .any(|bytes| bytes == b"secretly-large-unit")
    );
}

// 실패 처리 경계가 Methexis의 stdout과 stderr 진단을 생략하거나 요약으로 바꾸지
// 않고 그대로 전달한 뒤 원래 종료 상태가 포함된 오류를 반환하는지 확인한다.
#[test]
fn failed_check_preserves_both_diagnostic_streams() {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let error = handle_staged_check_output(
        false,
        "exit status: 2",
        b"structured stdout\n",
        b"structured stderr\n",
        &mut stdout,
        &mut stderr,
    )
    .unwrap_err();

    assert_eq!(stdout, b"structured stdout\n");
    assert_eq!(stderr, b"structured stderr\n");
    assert_eq!(
        error,
        "staged Methexis validation failed with exit status: 2"
    );
}

fn staged_semantic_repository(name: &str) -> TestRepository {
    let repository = TestRepository::new(name);
    repository.write("methexis/knowledge/one.md", "base\n");
    repository.commit_all("base");
    repository.write("methexis/knowledge/one.md", "candidate\n");
    repository.git(["add", "methexis/knowledge/one.md"]);
    repository
}

struct TestRepository {
    path: PathBuf,
}

impl TestRepository {
    fn new(name: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock follows Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "yo-validation-stage-{name}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        let repository = Self { path };
        repository.git(["init", "-q"]);
        repository.git(["config", "user.email", "test@example.invalid"]);
        repository.git(["config", "user.name", "validation-stage-test"]);
        repository
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn commit_all(&self, message: &str) {
        self.git(["add", "."]);
        self.git(["commit", "-qm", message]);
    }

    fn git<const N: usize>(&self, arguments: [&str; N]) {
        let output = Command::new("git")
            .args(arguments)
            .current_dir(&self.path)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
            .env_remove("GIT_PREFIX")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

impl Drop for TestRepository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
