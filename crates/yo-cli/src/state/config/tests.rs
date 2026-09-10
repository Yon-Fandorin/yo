use std::{
    env, fs, io,
    os::unix::fs::{FileTypeExt, MetadataExt},
    sync::atomic::{AtomicU64, Ordering},
};

use super::{snapshot::MAX_CONFIG_BYTES, *};

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
    assert!(matches!(config.frame_rate_limit(), FrameRateLimit::Fps60));
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
    assert!(matches!(config.frame_rate_limit(), FrameRateLimit::Fps120));
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
    assert_eq!(config.frame_rate_limit(), FrameRateLimit::Fps120);
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

// 읽기 상한을 한 byte 넘는 파일은 YAML parser에 넘기기 전에 거절합니다.
#[test]
fn oversized_configuration_is_bounded_during_the_read() {
    let path = env::temp_dir().join(format!("yo-config-large-{}", std::process::id()));
    fs::write(&path, vec![b'a'; MAX_CONFIG_BYTES as usize + 1]).unwrap();

    let error = load_from(&path).unwrap_err();

    fs::remove_file(&path).unwrap();
    assert!(matches!(
        error,
        ConfigError::TooLarge { path: found, .. } if found == path
    ));
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

// 테마는 기존 tui 설정 안에서 영속 선택되고 생략 시 원래 팔레트를 유지한다.
#[test]
fn configured_theme_loads_with_existing_tui_settings() {
    let directory = TestDirectory::new("theme");
    let path = directory.path().join("config.yaml");
    for (name, expected) in [
        ("default", Theme::Default),
        ("light", Theme::Light),
        ("mono", Theme::Mono),
    ] {
        fs::write(&path, format!("tui:\n  theme: {name}\n  max_fps: 60\n")).unwrap();
        let config = load_from(&path).unwrap();
        assert_eq!(config.theme(), expected);
        assert_eq!(config.frame_rate_limit(), FrameRateLimit::Fps60);
        config.verify_unchanged().unwrap();
    }
    fs::write(&path, "tui:\n  max_fps: 120\n").unwrap();
    assert_eq!(load_from(&path).unwrap().theme(), Theme::Default);
    fs::remove_file(&path).unwrap();
    assert_eq!(load_from(&path).unwrap().theme(), Theme::Default);
}

// 잘못된 테마는 경로와 허용 값을 제시하며 조용히 다른 팔레트로 대체하지 않는다.
#[test]
fn invalid_theme_has_actionable_configuration_error() {
    let directory = TestDirectory::new("bad-theme");
    let path = directory.path().join("config.yaml");
    fs::write(&path, "tui:\n  theme: typo\n").unwrap();
    let error = load_from(&path).unwrap_err();
    assert!(
        matches!(&error, ConfigError::InvalidTheme { path: found, value } if found == &path && value == "typo")
    );
    assert!(
        error
            .to_string()
            .contains("tui.theme must be default, light, or mono")
    );
    for value in ["[light]", "{name: light}"] {
        fs::write(&path, format!("tui:\n  theme: {value}\n")).unwrap();
        assert!(load_from(&path).is_err());
    }
}

// 사용자 출력 설정은 세션에 전달할 타입으로 보존되고 0열·음수·범위 초과·오타는 거절한다.
#[test]
fn output_preferences_load_and_reject_invalid_dimensions() {
    use std::num::NonZeroU16;
    let config = parse(
        Path::new("/tmp/yo-config.yaml"),
        "tui:\n  max_body_width: 80\n  tool_head_rows: 4\n  shell_tail_rows: 7\n  diff_head_rows: 10\n  show_images: false\n  hyperlinks: false\n  show_reasoning: false\n  show_diagrams: false\n  image_max_width: 8\n  code_padding: 3\n",
    )
    .unwrap();
    assert_eq!(
        config.output_preferences(),
        OutputPreferences::default()
            .with_max_body_width(NonZeroU16::new(80))
            .with_tool_head_rows(4)
            .with_shell_tail_rows(7)
            .with_diff_head_rows(10)
            .with_images(false)
            .with_hyperlinks(false)
            .with_reasoning(false)
            .with_diagrams(false)
            .with_image_max_width(NonZeroU16::new(8).unwrap())
            .with_code_padding(3)
    );
    for field in [
        "max_body_width: 0",
        "image_max_width: 0",
        "image_max_width: 65536",
        "show_images: sometimes",
        "hyperlinks: sometimes",
        "show_reasoning: sometimes",
        "show_diagrams: sometimes",
        "max_body_width: 65536",
        "code_padding: -1",
        "code_padding: 65536",
        "code_padding: 1.5",
        "tool_head_rows: -1",
        "shell_tail_rows: -1",
        "shell_tail_rows: 65536",
        "diff_head_rows: 65536",
        "tool_head_row: 4",
    ] {
        assert!(
            parse(
                Path::new("/tmp/yo-config.yaml"),
                &format!("tui:\n  {field}\n")
            )
            .is_err(),
            "{field}"
        );
    }
    assert_eq!(
        parse(Path::new("/tmp/yo-config.yaml"), "tui: {}\n")
            .unwrap()
            .output_preferences(),
        OutputPreferences::default()
    );
}

// 팔레트 역할과 색상은 정확히 해석하고 오타·잘못된 RGB·제어 문자열을 설정 경로와 함께 거절한다.
#[test]
fn semantic_theme_colors_load_and_reject_invalid_values() {
    use yo_tui::{ThemeColor, ThemeRole};
    let config = parse(
        Path::new("/tmp/yo-config.yaml"),
        "tui:\n  colors:\n    code_background: '#5f87af'\n    accent: terminal\n    reasoning_text: '#987654'\n    chart: '#e6a028'\n    chart_2: '#010203'\n    chart_3: '#040506'\n    chart_4: '#070809'\n",
    )
    .unwrap();
    assert_eq!(
        config.theme_overrides(),
        &ThemeOverrides::default()
            .with_color(ThemeRole::CodeBackground, ThemeColor::Rgb(95, 135, 175))
            .with_color(ThemeRole::Accent, ThemeColor::Terminal)
            .with_color(ThemeRole::ReasoningText, ThemeColor::Rgb(152, 118, 84))
            .with_color(ThemeRole::Chart, ThemeColor::Rgb(230, 160, 40))
            .with_color(ThemeRole::Chart2, ThemeColor::Rgb(1, 2, 3))
            .with_color(ThemeRole::Chart3, ThemeColor::Rgb(4, 5, 6))
            .with_color(ThemeRole::Chart4, ThemeColor::Rgb(7, 8, 9))
    );
    for (role, value) in [
        ("typo", "#112233"),
        ("accent", "#123"),
        ("accent", "#xxxxxx"),
        ("accent", "#éabcd"),
        ("accent", "red"),
    ] {
        let error = parse(
            Path::new("/tmp/yo-config.yaml"),
            &format!("tui:\n  colors:\n    {role}: '{value}'\n"),
        )
        .unwrap_err();
        assert!(matches!(error, ConfigError::InvalidThemeColor { .. }));
        assert!(error.to_string().contains(&format!("tui.colors.{role}")));
    }
}

// 코드 여백은 0을 허용하고 첫 상한 초과와 최대 u16 값을 같은 8칸으로 제한한다.
#[test]
fn code_padding_config_preserves_zero_and_caps_excess() {
    for value in [0, 1, 8, 9, u16::MAX] {
        let config = parse(
            Path::new("/tmp/yo-config.yaml"),
            &format!("tui:\n  code_padding: {value}\n"),
        )
        .unwrap();
        assert_eq!(
            config.output_preferences(),
            OutputPreferences::default().with_code_padding(value.min(8))
        );
    }
}

// 템플릿은 치환 없이 Unicode·공백·줄바꿈과 입력 기호를 그대로 보존한다.
#[test]
fn prompt_templates_preserve_literal_text_and_default_to_empty() {
    let body = " 한글\t\r\n$name @path ${HOME} $(echo no)\n ";
    let yaml = format!(
        "prompts:\n  Review_1: {}\n",
        serde_json::to_string(body).unwrap()
    );
    let config = parse(Path::new("config.yaml"), &yaml).unwrap();
    assert_eq!(config.prompts().get("Review_1").unwrap(), body);
    assert!(
        parse(Path::new("config.yaml"), "{}")
            .unwrap()
            .prompts()
            .names()
            .next()
            .is_none()
    );
    assert!(Config::default().prompts().names().next().is_none());
}

// 개수·이름·본문 바이트 상한은 경계값을 허용하고 첫 초과를 거부한다.
#[test]
fn prompt_template_limits_reject_first_excess() {
    let path = Path::new("config.yaml");
    let body = "한".repeat(21845) + "x";
    let name = "a".repeat(64);
    let yaml = format!(
        "prompts:\n  {name}: {}\n",
        serde_json::to_string(&body).unwrap()
    );
    assert_eq!(
        parse(path, &yaml)
            .unwrap()
            .prompts()
            .get(&name)
            .unwrap()
            .len(),
        65536
    );
    for (name, body) in [(name.clone(), body + "x"), (name + "a", "body".into())] {
        let yaml = format!(
            "prompts:\n  {name}: {}\n",
            serde_json::to_string(&body).unwrap()
        );
        assert!(matches!(
            parse(path, &yaml),
            Err(ConfigError::InvalidPrompts { .. })
        ));
    }
    let mut yaml = "prompts:\n".to_owned();
    for index in 0..128 {
        yaml.push_str(&format!("  p{index}: body\n"));
    }
    assert_eq!(parse(path, &yaml).unwrap().prompts().names().count(), 128);
    yaml.push_str("  excess: body\n");
    assert!(matches!(
        parse(path, &yaml),
        Err(ConfigError::InvalidPrompts { .. })
    ));
}

// 중복 키·잘못된 이름·빈 본문·제어문자는 설정 경계에서 거부하여 숨은 실행 입력을 만들지 않는다.
#[test]
fn prompt_templates_reject_duplicates_invalid_names_and_controls() {
    let path = Path::new("config.yaml");
    let duplicate = parse(path, "prompts:\n  review: first\n  review: second\n").unwrap_err();
    assert!(matches!(duplicate, ConfigError::InvalidYaml { .. }));
    for name in ["", "한글", "a b", "a/b", "$name"] {
        let yaml = format!(
            "prompts:\n  {}: body\n",
            serde_json::to_string(name).unwrap()
        );
        assert!(parse(path, &yaml).is_err(), "{name:?}");
    }
    for body in ["", "a\0b", "a\u{1b}b", "a\u{7f}b", "a\u{85}b"] {
        let yaml = format!(
            "prompts:\n  review: {}\n",
            serde_json::to_string(body).unwrap()
        );
        assert!(parse(path, &yaml).is_err(), "{body:?}");
    }
    assert!(parse(path, "prompts:\n  spacing: '   '\n").is_ok());
}

// 설정은 명시한 root의 구문만 검증하고 저장 Session의 workspace가 정해질 때까지 해석하지 않는다.
#[test]
fn skill_roots_are_explicit_and_preserve_relative_paths_and_scopes() {
    let path = Path::new("/config/elsewhere/config.yaml");
    assert!(parse(path, "{}").unwrap().skill_roots().is_empty());
    let config = parse(path, "skills:\n  roots:\n    - path: .skills\n      scope: workspace\n    - path: /missing/user-skills\n      scope: user\n").unwrap();
    assert_eq!(config.skill_roots()[0].path, Path::new(".skills"));
    assert_eq!(
        config.skill_roots()[0].scope,
        SkillReferenceScope::Workspace
    );
    assert_eq!(
        config.skill_roots()[1].path,
        Path::new("/missing/user-skills")
    );
    assert_eq!(config.skill_roots()[1].scope, SkillReferenceScope::User);
}

// 잘못된 scope·알 수 없는 field·제어 문자·상한 초과는 filesystem I/O 전에 거부한다.
#[test]
fn skill_roots_reject_invalid_shapes_and_enforce_sixteen_root_limit() {
    let path = Path::new("config.yaml");
    for yaml in [
        "skills: {roots: [{path: '', scope: workspace}]}",
        "skills: {roots: [{path: x, scope: system}]}",
        "skills: {roots: [{path: x}]}",
        "skills: {roots: [{path: x, scope: user, extra: true}]}",
        "skills: {defaults: true}",
        "skills: {roots: [{path: \"bad\\u0000path\", scope: user}]}",
    ] {
        assert!(parse(path, yaml).is_err(), "{yaml}");
    }
    let mut yaml = "skills:\n  roots:\n".to_owned();
    for index in 0..16 {
        yaml.push_str(&format!("    - {{path: root-{index}, scope: workspace}}\n"));
    }
    assert_eq!(parse(path, &yaml).unwrap().skill_roots().len(), 16);
    yaml.push_str("    - {path: excess, scope: user}\n");
    assert!(matches!(
        parse(path, &yaml),
        Err(ConfigError::InvalidSkills { .. })
    ));
}

// 클립보드 설정은 생략할 수 있으며 명시한 각 source의 값은 호스트 I/O 없이 보존한다.
#[test]
fn clipboard_sources_parse_without_opening_target_paths() {
    let path = Path::new("config.yaml");
    assert_eq!(parse(path, "{}").unwrap().clipboard_source(), None);
    assert_eq!(
        parse(path, "clipboard: {source: native}")
            .unwrap()
            .clipboard_source(),
        Some(&ClipboardSource::Native)
    );
    assert_eq!(
        parse(
            path,
            "clipboard: {source: socket, path: /missing/private/clipboard.sock}"
        )
        .unwrap()
        .clipboard_source(),
        Some(&ClipboardSource::Socket {
            path: PathBuf::from("/missing/private/clipboard.sock")
        })
    );
    let config = parse(path, "clipboard:\n  source: ssh\n  host: user@[2001:db8::1]\n  reader: macos\n  identity_file: /missing/key\n  known_hosts_file: /missing/known_hosts\n  port: 2222\n").unwrap();
    assert_eq!(
        config.clipboard_source(),
        Some(&ClipboardSource::Ssh {
            host: "user@[2001:db8::1]".to_owned(),
            reader: ClipboardReader::Macos,
            identity_file: Some(PathBuf::from("/missing/key")),
            known_hosts_file: Some(PathBuf::from("/missing/known_hosts")),
            port: std::num::NonZeroU16::new(2222),
        })
    );
    for (reader, expected) in [
        ("wayland", ClipboardReader::Wayland),
        ("x11", ClipboardReader::X11),
    ] {
        let config = parse(
            path,
            &format!("clipboard: {{source: ssh, host: desktop-alias, reader: {reader}}}"),
        )
        .unwrap();
        assert_eq!(
            config.clipboard_source(),
            Some(&ClipboardSource::Ssh {
                host: "desktop-alias".to_owned(),
                reader: expected,
                identity_file: None,
                known_hosts_file: None,
                port: None,
            })
        );
    }
}

// source별 미등록 필드·필수 값 누락·임의 reader·잘못된 port를 구조 단계에서 거부한다.
#[test]
fn clipboard_config_rejects_unknown_or_ambiguous_shapes() {
    for yaml in [
        "clipboard: null",
        "clipboard: {}",
        "clipboard: {source: native, host: desktop}",
        "clipboard: {source: socket}",
        "clipboard: {source: socket, path: /private/c.sock, reader: macos}",
        "clipboard: {source: ssh, host: desktop}",
        "clipboard: {source: ssh, host: desktop, reader: shell}",
        "clipboard: {source: ssh, host: desktop, reader: macos, command: arbitrary}",
        "clipboard: {source: ssh, host: desktop, reader: macos, port: 0}",
        "clipboard: {source: ssh, host: desktop, reader: macos, port: 65536}",
        "clipboard: {source: native, source: ssh}",
    ] {
        assert!(parse(Path::new("config.yaml"), yaml).is_err(), "{yaml}");
    }
}

// SSH 옵션·셸 구문·공백·제어 문자와 상대 경로는 실행 단계에 전달하지 않는다.
#[test]
fn clipboard_config_rejects_unsafe_hosts_and_paths() {
    for host in [
        "",
        "-option",
        "two hosts",
        "host;command",
        "$(command)",
        "host`command`",
        "host\nother",
        "host/path",
        "host\\path",
    ] {
        let yaml = format!(
            "clipboard: {{source: ssh, host: {}, reader: macos}}",
            serde_json::to_string(host).unwrap()
        );
        assert!(
            matches!(
                parse(Path::new("config.yaml"), &yaml),
                Err(ConfigError::InvalidClipboard { .. })
            ),
            "{host:?}"
        );
    }
    for yaml in [
        "clipboard: {source: socket, path: relative.sock}",
        "clipboard: {source: socket, path: /private/../clipboard.sock}",
        "clipboard: {source: ssh, host: desktop, reader: macos, identity_file: key}",
        "clipboard: {source: ssh, host: desktop, reader: macos, known_hosts_file: known_hosts}",
        "clipboard: {source: ssh, host: desktop, reader: macos, identity_file: \"/key\\nfile\"}",
    ] {
        assert!(
            matches!(
                parse(Path::new("config.yaml"), yaml),
                Err(ConfigError::InvalidClipboard { .. })
            ),
            "{yaml}"
        );
    }
}

// host와 경로의 정확한 상한을 수용하고 첫 초과 바이트를 거부한다.
#[test]
fn clipboard_config_bounds_host_and_path_bytes() {
    let path = Path::new("config.yaml");
    for (size, accepted) in [(255, true), (256, false)] {
        let yaml = format!(
            "clipboard: {{source: ssh, host: {}, reader: macos}}",
            "a".repeat(size)
        );
        assert_eq!(parse(path, &yaml).is_ok(), accepted);
    }
    for (size, accepted) in [(4096, true), (4097, false)] {
        let absolute = format!("/{}", "a".repeat(size - 1));
        for field in ["identity_file", "known_hosts_file"] {
            let yaml = format!(
                "clipboard: {{source: ssh, host: desktop, reader: macos, {field}: {absolute}}}"
            );
            assert_eq!(parse(path, &yaml).is_ok(), accepted);
        }
        let yaml = format!("clipboard: {{source: socket, path: {absolute}}}");
        assert_eq!(parse(path, &yaml).is_ok(), accepted);
    }
}

// SSH의 token·환경 변수 확장으로 신뢰 파일이 바뀌지 않으며 일반 소켓 경로는 문자 그대로다.
#[test]
fn clipboard_ssh_paths_reject_openssh_expansion_tokens() {
    let path = Path::new("config.yaml");
    for field in ["identity_file", "known_hosts_file"] {
        for value in ["/private/%h/key", "/private/${USER}/key"] {
            let yaml = format!(
                "clipboard: {{source: ssh, host: desktop, reader: macos, {field}: {value:?}}}"
            );
            assert!(matches!(
                parse(path, &yaml),
                Err(ConfigError::InvalidClipboard { .. })
            ));
        }
    }
    assert!(
        parse(
            path,
            "clipboard: {source: socket, path: '/private/%h/${USER}.sock'}"
        )
        .is_ok()
    );
}
