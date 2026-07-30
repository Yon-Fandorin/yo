use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use super::{ImpactInput, developer_docs, slice_review};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

// staged deletion도 이름 목록에서 빠지지 않아 코드 변경으로 분류되고, 영향 trailer가
// 없으면 Developer Docs 검사가 실패하는 실제 Git 경로 수집을 검증한다.
#[test]
fn staged_deletion_remains_a_developer_docs_change() {
    let repository = TestRepository::new("docs-deletion");
    repository.write("crates/yo-core/src/obsolete.rs", "obsolete\n");
    repository.git(["add", "."]);
    repository.git(["commit", "--quiet", "-m", "test: seed deletion"]);
    std::fs::remove_file(repository.path.join("crates/yo-core/src/obsolete.rs")).unwrap();
    repository.git(["add", "-u"]);
    let message = repository.write("message", "refactor: remove obsolete code\n");

    let input = ImpactInput::load_from(
        &repository.path,
        message,
        None,
        Some("develop".to_owned()),
        false,
    )
    .unwrap();

    assert!(developer_docs::check(&input).is_err());
}

// amend처럼 index가 비어 있어도 현재 HEAD의 변경 경로를 다시 읽어, 기존 코드 커밋을
// 메시지만 바꾸면서 review 요구를 none으로 약화하지 못하게 한다.
#[test]
fn clean_index_uses_head_paths_for_message_amendment() {
    let repository = TestRepository::new("clean-amend");
    repository.write("tools/example/check.sh", "reviewed\n");
    repository.git(["add", "."]);
    repository.git(["commit", "--quiet", "-m", "test: seed reviewed tool"]);
    let message = repository.write(
        "message",
        "test: rewrite message\n\nSlice-Review: none - message-only amend\n",
    );

    let input = ImpactInput::load_from(
        &repository.path,
        message,
        None,
        Some("develop".to_owned()),
        true,
    )
    .unwrap();

    assert!(slice_review::check(&input).is_err());
}

// Developer Docs 검사는 기존 계약대로 staged 변경만 보며, 빈 index에서 직전 commit의
// 경로를 가져오는 Slice review 전용 amend 보호를 공유하지 않는다.
#[test]
fn developer_docs_does_not_use_slice_review_head_fallback() {
    let repository = TestRepository::new("docs-clean-index");
    repository.write("crates/yo-core/src/lib.rs", "reviewed\n");
    repository.git(["add", "."]);
    repository.git(["commit", "--quiet", "-m", "test: seed reviewed code"]);
    let message = repository.write("message", "test: allow empty follow-up\n");

    let input = ImpactInput::load_from(
        &repository.path,
        message,
        None,
        Some("develop".to_owned()),
        false,
    )
    .unwrap();

    assert!(developer_docs::check(&input).is_ok());
    assert!(input.changed_paths.is_empty());
}

// Git 저장소가 아닌 위치에서 staged 경로 조회가 실패하면 빈 변경으로 간주하지 않고
// 입력 로딩 자체를 실패시켜 검수 정책이 fail-closed로 동작하게 한다.
#[test]
fn changed_path_query_failure_is_not_treated_as_an_empty_change() {
    let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "yo-xtask-not-a-repository-{}-{sequence}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let message = directory.join("message");
    std::fs::write(&message, "test: invalid repository\n").unwrap();

    let result =
        ImpactInput::load_from(&directory, message, None, Some("develop".to_owned()), false);

    let _ = std::fs::remove_dir_all(&directory);
    assert!(result.is_err());
}

// Wave가 현재 develop의 이미 검수된 commit을 병합할 때만 별도 Slice trailer 없이
// 통과하며, MERGE_HEAD가 develop의 ancestor라는 실제 Git 관계를 확인한다.
#[test]
fn reviewed_develop_merge_into_wave_is_exempt() {
    let repository = TestRepository::new("reviewed-wave-merge");
    repository.write("README.md", "base\n");
    repository.git(["add", "."]);
    repository.git(["commit", "--quiet", "-m", "test: seed"]);
    repository.git(["branch", "wave/w1-review"]);
    repository.write("CONTRIBUTING.md", "reviewed change\n");
    repository.git(["add", "."]);
    repository.git(["commit", "--quiet", "-m", "test: advance develop"]);
    repository.git(["switch", "--quiet", "wave/w1-review"]);
    repository.git(["merge", "--quiet", "--no-ff", "--no-commit", "develop"]);
    let message = repository.write("message", "Merge develop into Wave\n");

    let input = ImpactInput::load_from(&repository.path, message, None, None, true).unwrap();

    assert!(slice_review::check(&input).is_ok());
}

// 아직 develop에 없는 Slice를 Wave로 병합하거나 develop이 직접 미검수 Slice를
// 병합하면 MERGE_HEAD가 있어도 review 면제가 적용되지 않는다.
#[test]
fn unreviewed_slice_merges_are_not_exempt() {
    for target in ["wave/w1-review", "develop"] {
        let repository = TestRepository::new(&target.replace('/', "-"));
        repository.write("README.md", "base\n");
        repository.git(["add", "."]);
        repository.git(["commit", "--quiet", "-m", "test: seed"]);
        repository.git(["branch", "wave/w1-review"]);
        repository.git(["switch", "--quiet", "-c", "slice/direct/unreviewed"]);
        repository.write("tools/example/change.sh", "unreviewed\n");
        repository.git(["add", "."]);
        repository.git(["commit", "--quiet", "-m", "test: unreviewed Slice"]);
        repository.git(["switch", "--quiet", target]);
        repository.git([
            "merge",
            "--quiet",
            "--no-ff",
            "--no-commit",
            "slice/direct/unreviewed",
        ]);
        let message = repository.write("message", "Merge unreviewed Slice\n");

        let input = ImpactInput::load_from(&repository.path, message, None, None, true).unwrap();

        assert!(
            slice_review::check(&input).is_err(),
            "{target} must not exempt an unreviewed Slice merge"
        );
    }
}

struct TestRepository {
    path: PathBuf,
}

impl TestRepository {
    fn new(label: &str) -> Self {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "yo-xtask-{label}-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        let repository = Self { path };
        repository.git(["init", "--quiet", "-b", "develop"]);
        repository.git(["config", "user.name", "xtask Test"]);
        repository.git(["config", "user.email", "xtask@example.invalid"]);
        repository
    }

    fn write(&self, relative: impl AsRef<Path>, content: &str) -> PathBuf {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, content).unwrap();
        path
    }

    fn git<I, S>(&self, arguments: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let status = Command::new("git")
            .current_dir(&self.path)
            .env_remove("GIT_DIR")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_WORK_TREE")
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    }
}

impl Drop for TestRepository {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
