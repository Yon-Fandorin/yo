use std::{collections::BTreeSet, ffi::OsStr, fs, path::Path};

use super::{
    super::{classify_git_workspace, git_command, is_git_workspace},
    support::TempFixture,
};

// Git 미설치·권한·safe.directory 오류를 일반 폴더로 오인해 ignore 파일을 노출하지 않는다.
#[test]
fn git_detection_distinguishes_non_repositories_from_operational_failures() {
    assert!(classify_git_workspace(true, b"true\n", b"").unwrap());
    assert!(!classify_git_workspace(true, b"false\n", b"").unwrap());
    assert!(classify_git_workspace(false, b"", b"fatal: not a git repository\n").is_err());
    assert!(classify_git_workspace(false, b"", b"fatal: detected dubious ownership\n").is_err());
    assert!(
        classify_git_workspace(false, b"", "치명적: 깃 저장소가 아닙니다\n".as_bytes()).is_err()
    );
}

// Git probe가 성공했지만 boolean이 아니거나 stderr 없이 실패한 경우에도 workspace 선택을 조용히
// 바꾸지 않고 안정적인 진단을 보존하는지 확인한다.
#[test]
fn git_probe_diagnostics_cover_unexpected_success_and_empty_failure() {
    let unexpected = classify_git_workspace(true, b"maybe\n", b"").unwrap_err();
    let empty_failure = classify_git_workspace(false, b"", b"").unwrap_err();
    assert!(!unexpected.trim().is_empty());
    assert!(unexpected.contains("maybe"));
    assert!(!empty_failure.trim().is_empty());
    assert!(empty_failure.to_ascii_lowercase().contains("git"));
    assert_ne!(unexpected, empty_failure);
}

// 깨진 `.git` 표식은 ignore 보호를 끈 일반 폴더로 강등하지 않고 명시적으로 실패한다.
#[test]
fn invalid_git_marker_fails_closed() {
    let fixture = TempFixture::new("invalid-git-marker");
    let root = fixture.path();
    fs::write(root.join(".git"), "gitdir: missing\n").unwrap();

    let error = is_git_workspace(root).unwrap_err();
    assert!(!error.trim().is_empty());
    assert!(error.contains(&root.join(".git").display().to_string()));
}

// provider 소유 Git 명령은 caller의 대체 index를 제거해 실제 workspace index만 읽는다.
#[test]
fn provider_git_commands_remove_an_inherited_alternate_index() {
    let command = git_command(Path::new("."));
    assert!(
        command
            .get_envs()
            .any(|(name, value)| { name == OsStr::new("GIT_INDEX_FILE") && value.is_none() })
    );
}

// provider 명령이 상속된 Git 환경 변수 중 현재 제거하도록 정의된 모든 이름을 지우며, alternate
// index만 확인하는 기존 회귀 테스트에 머물지 않는지 확인한다.
#[test]
fn provider_git_commands_remove_every_cleared_git_environment_override() {
    let expected = [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_IMPLICIT_WORK_TREE",
        "GIT_GRAFT_FILE",
        "GIT_NO_REPLACE_OBJECTS",
        "GIT_REPLACE_REF_BASE",
        "GIT_PREFIX",
        "GIT_SHALLOW_FILE",
        "GIT_CEILING_DIRECTORIES",
        "GIT_DISCOVERY_ACROSS_FILESYSTEM",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    let cleared = git_command(Path::new("."))
        .get_envs()
        .filter_map(|(name, value)| {
            if name.to_string_lossy().starts_with("GIT_") {
                assert!(value.is_none(), "Git override {name:?} was not removed");
                Some(name.to_string_lossy().into_owned())
            } else {
                None
            }
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(cleared, expected);
}

// Git 설정이 FIFO 읽기에 멈추어도 제한 시간 뒤 자식 프로세스를 회수하고 오류를 반환한다.
// 해당 설정을 제거하면 같은 작업 공간을 다시 정상적으로 탐색할 수 있다.
#[test]
fn git_discovery_timeout_reaps_a_process_blocked_on_configuration() {
    use std::time::{Duration, Instant};

    use rustix::fs::{CWD, Mode, mkfifoat};
    let fixture = TempFixture::new("git-discovery-timeout");
    let root = fixture.path();
    assert!(
        git_command(root)
            .args(["init", "-q"])
            .status()
            .unwrap()
            .success()
    );
    let config_path = root.join(".git/config");
    let original = fs::read_to_string(&config_path).unwrap();
    let fifo = root.join("blocked-config");
    mkfifoat(CWD, &fifo, Mode::RUSR | Mode::WUSR).unwrap();
    fs::write(
        &config_path,
        format!("{original}\n[include]\n    path = {}\n", fifo.display()),
    )
    .unwrap();
    let started = Instant::now();
    let error = is_git_workspace(root).unwrap_err();
    assert!(error.contains("exceeded 5 seconds"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(10));
    fs::write(&config_path, original).unwrap();
    assert!(is_git_workspace(root).unwrap());
}

// Git stdout은 정확히 16 MiB까지 받으며 다음 경로 record부터 검색 실패로 알린다.
// 실제 파일을 대량 생성하지 않고 임시 index에 경로만 넣어 출력 경계를 검증한다.
#[test]
fn tracked_output_accepts_its_byte_limit_and_rejects_the_next_record() {
    use std::{io::Write, process::Stdio};

    use super::{super::build_inventory, support::host_id};
    let fixture = TempFixture::new("tracked-output-limit");
    let root = fixture.path().join("workspace");
    fs::create_dir(&root).unwrap();
    assert!(
        git_command(&root)
            .args(["init", "-q", "--object-format=sha1"])
            .status()
            .unwrap()
            .success()
    );
    let index_input = fixture.path().join("index-input");
    let mut input = fs::File::create(&index_input).unwrap();
    for index in 0..16_384 {
        let path = format!("{index:05}/{}", "x".repeat(1017));
        assert_eq!(path.len() + 1, 1024);
        write!(
            input,
            "100644 e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\t{path}\0"
        )
        .unwrap();
    }
    drop(input);
    assert!(
        git_command(&root)
            .args(["update-index", "-z", "--index-info"])
            .stdin(Stdio::from(fs::File::open(&index_input).unwrap()))
            .status()
            .unwrap()
            .success()
    );
    let inventory = build_inventory(&root, host_id()).unwrap();
    assert!(inventory.entries.is_empty());
    assert!(matches!(
        inventory.status,
        crate::WorkspaceReferenceSearchStatus::Incomplete(_)
    ));
    fs::write(
        &index_input,
        b"100644 e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\tz\0",
    )
    .unwrap();
    assert!(
        git_command(&root)
            .args(["update-index", "-z", "--index-info"])
            .stdin(Stdio::from(fs::File::open(&index_input).unwrap()))
            .status()
            .unwrap()
            .success()
    );
    let error = match build_inventory(&root, host_id()) {
        Ok(_) => panic!("the first excess output record must fail discovery"),
        Err(error) => error,
    };
    assert!(error.contains("output exceeds its byte limit"), "{error}");
}
