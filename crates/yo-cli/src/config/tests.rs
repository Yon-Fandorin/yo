use std::{
    env, io,
    os::unix::fs::{FileTypeExt, MetadataExt},
    sync::atomic::{AtomicU64, Ordering},
};

use super::*;

static NEXT_TEST_DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        Self::new_in(&env::temp_dir(), label)
    }

    fn new_in(parent: &Path, label: &str) -> Self {
        loop {
            let fixture_id = NEXT_TEST_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(
                "yo-config-{label}-{}-{fixture_id}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {},
                Err(error) => panic!(
                    "creating exclusive config test directory {} failed: {error}",
                    path.display()
                ),
            }
        }
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn expect_config_error(result: Result<(), ConfigError>) {
    let _ = result.expect_err("the injected operation must fail");
}

fn assert_fifo_rejection(directory: &TestDirectory) {
    let path = directory.path().join("config.yaml");
    nix::unistd::mkfifo(
        &path,
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )
    .unwrap();

    let error = load_from(&path).unwrap_err();

    assert!(matches!(error, ConfigError::UnsupportedFileType(found) if found == path));
}

fn assert_symlink_rejection(directory: &TestDirectory) {
    let target = directory.path().join("target.yaml");
    let alias = directory.path().join("config.yaml");
    fs::write(&target, "").unwrap();
    std::os::unix::fs::symlink(&target, &alias).unwrap();

    let error = load_from(&alias).unwrap_err();

    assert!(matches!(error, ConfigError::Io { .. }));
}

#[derive(Clone, Copy)]
enum LegacyEntryShape {
    Symlink,
    Fifo,
    Regular,
    Directory,
}

#[derive(Debug, Eq, PartialEq)]
enum LegacyEntryPayload {
    Symlink(PathBuf),
    Fifo,
    Regular(Vec<u8>),
    Directory(Vec<u8>),
}

#[derive(Debug, Eq, PartialEq)]
struct LegacyEntrySnapshot {
    device: u64,
    inode: u64,
    mode: u32,
    length: u64,
    payload: LegacyEntryPayload,
}

fn create_legacy_entry(path: &Path, shape: LegacyEntryShape) {
    match shape {
        LegacyEntryShape::Symlink => {
            std::os::unix::fs::symlink("retired-config-target.yaml", path).unwrap();
        },
        LegacyEntryShape::Fifo => {
            nix::unistd::mkfifo(
                path,
                nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
            )
            .unwrap();
        },
        LegacyEntryShape::Regular => fs::write(path, b"retired config fixture\n").unwrap(),
        LegacyEntryShape::Directory => {
            fs::create_dir(path).unwrap();
            fs::write(path.join("sentinel"), b"retired directory fixture\n").unwrap();
        },
    }
}

fn snapshot_legacy_entry(path: &Path) -> LegacyEntrySnapshot {
    let metadata = fs::symlink_metadata(path).unwrap();
    let file_type = metadata.file_type();
    let payload = if file_type.is_symlink() {
        LegacyEntryPayload::Symlink(fs::read_link(path).unwrap())
    } else if file_type.is_file() {
        LegacyEntryPayload::Regular(fs::read(path).unwrap())
    } else if file_type.is_dir() {
        LegacyEntryPayload::Directory(fs::read(path.join("sentinel")).unwrap())
    } else if file_type.is_fifo() {
        LegacyEntryPayload::Fifo
    } else {
        panic!(
            "legacy fixture {} has an unexpected file type",
            path.display()
        );
    };
    LegacyEntrySnapshot {
        device: metadata.dev(),
        inode: metadata.ino(),
        mode: metadata.mode(),
        length: metadata.len(),
        payload,
    }
}

fn remove_legacy_entry(path: &Path, shape: LegacyEntryShape) {
    match shape {
        LegacyEntryShape::Directory => fs::remove_dir_all(path).unwrap(),
        LegacyEntryShape::Symlink | LegacyEntryShape::Fifo | LegacyEntryShape::Regular => {
            fs::remove_file(path).unwrap();
        },
    }
}

// config.yaml에서 계속 소유하는 Session·TUI 일반 설정은 모델 정의 제거와 무관하게 읽힙니다.
#[test]
fn general_configuration_remains_supported() {
    let config = parse(
        Path::new("/tmp/config.yaml"),
        "session:\n  list:\n    date_format: '%Y'\ntui:\n  max_fps: 60\n",
    )
    .unwrap();
    assert!(matches!(
        config.frame_rate_limit(),
        yo_tui::FrameRateLimit::Fps60
    ));
    assert!(config.model_catalog().entries().is_empty());
}

// 모델 정의의 durable owner는 하나뿐이므로 config.yaml의 model field를 변환하지 않고 거절합니다.
#[test]
fn model_is_an_unknown_top_level_field() {
    let error = parse(Path::new("/tmp/config.yaml"), "model:\n  bindings: []\n")
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown field"));
    assert!(error.contains("model"));
}

// TUI frame rate는 기존의 닫힌 60·120 값만 허용하고 다른 숫자를 기본값으로 축약하지 않습니다.
#[test]
fn max_fps_remains_closed() {
    let error = parse(Path::new("/tmp/config.yaml"), "tui:\n  max_fps: 90\n")
        .unwrap_err()
        .to_string();
    assert!(error.contains("must be 60 or 120"));
}

// 빈 일반 설정은 모델 target을 만들지 않으면서 기존 Session·TUI 기본값만 제공합니다.
#[test]
fn empty_config_uses_defaults() {
    let config = parse(Path::new("/tmp/config.yaml"), "{}\n").unwrap();
    assert!(matches!(
        config.frame_rate_limit(),
        yo_tui::FrameRateLimit::Fps120
    ));
    assert!(config.date_formatter().is_ok());
}

// 파일명만 있는 상대 YO_CONFIG도 빈 parent를 state root로 노출하지 않고 현재 디렉터리를
// 기준으로 credentials, connections, account-capacity cache를 함께 둡니다.
#[test]
fn relative_config_filename_uses_the_current_state_directory() {
    let config = parse(Path::new("config.yaml"), "{}\n").unwrap();

    assert_eq!(config.state_directory(), PathBuf::from("."));
    assert_eq!(
        config.account_capacity_path(),
        PathBuf::from("./account-capacity.yaml")
    );
    assert_eq!(
        config.credential_path(),
        PathBuf::from("./credentials.yaml")
    );
    assert_eq!(
        config.connection_path(),
        PathBuf::from("./connections.yaml")
    );
}

// 읽기 전용 명령은 설정 파일이 없어도 기본값을 사용하며 경로나 파일을 만들지 않습니다.
#[test]
fn missing_configuration_uses_defaults_without_creating_a_file() {
    let root = env::temp_dir().join(format!("yo-config-missing-{}", std::process::id()));
    let path = root.join("config.yaml");

    assert!(!path.exists());
    let config = load_from(&path).unwrap();

    assert!(config.date_formatter().is_ok());
    assert_eq!(config.frame_rate_limit(), yo_tui::FrameRateLimit::Fps120);
    assert!(!path.exists());
}

// 사용자가 지정한 날짜 형식은 시작 때 검증한 동일 formatter로 실제 시각에 적용됩니다.
#[test]
fn custom_date_format_is_validated_and_applied() {
    let config = parse(
        Path::new("config.yaml"),
        "session:\n  list:\n    date_format: '%Y'\n",
    )
    .unwrap();

    assert_eq!(
        config
            .date_formatter()
            .unwrap()
            .format_unix_millis(15_724_800_000)
            .unwrap(),
        "1970"
    );
}

// 끝나지 않은 `%` 같은 잘못된 strftime 문법은 사용 시점까지 미루지 않습니다.
#[test]
fn invalid_date_format_is_rejected() {
    let error = parse(
        Path::new("config.yaml"),
        "session:\n  list:\n    date_format: '%Y %'\n",
    )
    .unwrap_err();

    assert!(error.to_string().contains("config.yaml"));
    assert!(error.to_string().contains("session.list.date_format"));
}

// 기본 설정 root는 현재 디렉터리에 따라 뜻이 바뀌는 상대경로를 허용하지 않습니다.
#[test]
fn default_configuration_roots_require_absolute_paths() {
    assert!(environment_root("HOME", OsString::from("")).is_err());
    assert!(environment_root("HOME", OsString::from("relative")).is_err());
    assert!(environment_root("XDG_CONFIG_HOME", OsString::from("config")).is_err());
    assert_eq!(
        environment_root("HOME", OsString::from("/home/user")).unwrap(),
        PathBuf::from("/home/user")
    );
}

// 읽기 상한을 한 byte 넘는 파일은 YAML parser에 넘기기 전에 거절합니다.
#[test]
fn oversized_configuration_is_bounded_during_the_read() {
    let path = env::temp_dir().join(format!("yo-config-large-{}", std::process::id()));
    fs::write(&path, vec![b'a'; MAX_CONFIG_BYTES as usize + 1]).unwrap();

    let error = load_from(&path).unwrap_err();

    fs::remove_file(&path).unwrap();
    assert!(matches!(error, ConfigError::TooLarge(found) if found == path));
}

// FIFO는 nonblocking open 뒤 regular-file 검사를 받아 writer를 기다리지 않고 실패합니다.
#[test]
fn fifo_configuration_is_rejected_without_waiting_for_a_writer() {
    let directory = TestDirectory::new("fifo");
    assert_fifo_rejection(&directory);
}

// 최종 설정 경로가 symlink이면 대상 내용이 정상이어도 no-follow open에서 실패합니다.
#[test]
fn symlink_configuration_is_rejected_without_following_its_target() {
    let directory = TestDirectory::new("symlink");
    assert_symlink_rejection(&directory);
}

// 격리 parent의 이전 PID-only FIFO·symlink basename에 symlink, FIFO, regular file,
// directory가 남아 있어도 새 전용 root에서 본래 거절 동작까지 도달하고 기존 entry는 바꾸지
// 않습니다.
#[test]
fn stale_legacy_entries_do_not_block_owned_config_fixtures() {
    let sandbox = TestDirectory::new("legacy-sandbox");
    for label in ["fifo", "symlink"] {
        let legacy_path = sandbox
            .path()
            .join(format!("yo-config-{label}-{}", std::process::id()));
        for shape in [
            LegacyEntryShape::Symlink,
            LegacyEntryShape::Fifo,
            LegacyEntryShape::Regular,
            LegacyEntryShape::Directory,
        ] {
            create_legacy_entry(&legacy_path, shape);
            let before = snapshot_legacy_entry(&legacy_path);

            let directory = TestDirectory::new_in(sandbox.path(), label);
            let owned_root = directory.path().to_owned();
            if label == "fifo" {
                assert_fifo_rejection(&directory);
            } else {
                assert_symlink_rejection(&directory);
            }
            drop(directory);

            assert!(!owned_root.exists());
            assert_eq!(snapshot_legacy_entry(&legacy_path), before);
            remove_legacy_entry(&legacy_path, shape);
        }
    }
}

// 같은 parent에서 FIFO·symlink fixture를 병렬 생성해도 atomic ID와 exclusive create가
// root를 공유하지 않고, 각 Drop은 자기 root만 제거하며 parent sentinel은 보존합니다.
#[test]
fn parallel_config_fixtures_keep_unique_scoped_roots() {
    let sandbox = TestDirectory::new("parallel-sandbox");
    let sentinel = sandbox.path().join("sentinel");
    fs::write(&sentinel, b"outside every child fixture\n").unwrap();
    let parent = sandbox.path();

    let mut roots = std::thread::scope(|scope| {
        let handles = (0..8)
            .map(|index| {
                scope.spawn(move || {
                    let label = if index % 2 == 0 {
                        "parallel-fifo"
                    } else {
                        "parallel-symlink"
                    };
                    let directory = TestDirectory::new_in(parent, label);
                    let root = directory.path().to_owned();
                    if index % 2 == 0 {
                        assert_fifo_rejection(&directory);
                    } else {
                        assert_symlink_rejection(&directory);
                    }
                    drop(directory);
                    assert!(!root.exists());
                    root
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    roots.sort();
    roots.dedup();

    assert_eq!(roots.len(), 8);
    assert_eq!(
        fs::read(sentinel).unwrap(),
        b"outside every child fixture\n"
    );
}

// 같은 bytes로 파일을 교체해도 identity metadata가 달라지면 stale 계획을 막습니다.
#[test]
fn final_config_guard_detects_same_byte_replacement() {
    let directory = TestDirectory::new("guard");
    let path = directory.path().join("config.yaml");
    fs::write(&path, "session: {}\n").unwrap();
    let config = load_from(&path).unwrap();
    assert!(config.verify_unchanged().is_ok());

    let replacement = directory.path().join("replacement.yaml");
    fs::write(&replacement, "session: {}\n").unwrap();
    fs::rename(&replacement, &path).unwrap();
    let error = config.verify_unchanged().unwrap_err();

    assert!(matches!(error, ConfigError::Changed(_)));
}

// config fixture 안에 FIFO·symlink·교체 파일이 함께 있어도 unexpected Ok의 unwrap panic은
// guard를 unwind하며 전체 전용 root를 제거하므로 다음 테스트에 잔여물을 남기지 않습니다.
#[test]
fn config_fixture_cleanup_survives_unexpected_success_panics() {
    let directory = TestDirectory::new("panic-cleanup");
    let root = directory.path().to_owned();

    let outcome = std::panic::catch_unwind(move || {
        let fifo = directory.path().join("config.fifo");
        nix::unistd::mkfifo(
            &fifo,
            nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
        )
        .unwrap();
        let target = directory.path().join("target.yaml");
        fs::write(&target, "session: {}\n").unwrap();
        std::os::unix::fs::symlink(&target, directory.path().join("config.link")).unwrap();
        fs::write(directory.path().join("replacement.yaml"), "session: {}\n").unwrap();

        expect_config_error(Ok(()));
    });

    assert!(outcome.is_err());
    assert!(!root.exists());
}

// version은 migration 신호가 아니라 다른 알 수 없는 설정 키와 같은 typed YAML 오류입니다.
#[test]
fn version_and_unknown_configuration_fields_are_rejected() {
    for contents in [
        "version: 1\nsession: {}\n",
        "session:\n  list:\n    version: 1\n",
        "session:\n  list:\n    date_formt: '%Y'\n",
    ] {
        let error = parse(Path::new("/tmp/yo-config.yaml"), contents).unwrap_err();
        assert!(matches!(error, ConfigError::InvalidYaml { .. }));
        assert!(error.to_string().contains("unknown field"));
    }
}
