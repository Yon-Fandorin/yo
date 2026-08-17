use super::*;

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

// 읽기 전용 명령은 설정 파일이 없어도 기본값을 사용하며 경로나 파일을 만들지 않습니다.
#[test]
fn missing_configuration_uses_defaults_without_creating_a_file() {
    let root = std::env::temp_dir().join(format!("yo-config-missing-{}", std::process::id()));
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
    let path = std::env::temp_dir().join(format!("yo-config-large-{}", std::process::id()));
    fs::write(&path, vec![b'a'; MAX_CONFIG_BYTES as usize + 1]).unwrap();

    let error = load_from(&path).unwrap_err();

    fs::remove_file(&path).unwrap();
    assert!(matches!(error, ConfigError::TooLarge(found) if found == path));
}

// FIFO는 nonblocking open 뒤 regular-file 검사를 받아 writer를 기다리지 않고 실패합니다.
#[test]
fn fifo_configuration_is_rejected_without_waiting_for_a_writer() {
    let path = std::env::temp_dir().join(format!("yo-config-fifo-{}", std::process::id()));
    nix::unistd::mkfifo(
        &path,
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )
    .unwrap();

    let error = load_from(&path).unwrap_err();

    fs::remove_file(&path).unwrap();
    assert!(matches!(error, ConfigError::UnsupportedFileType(found) if found == path));
}

// 최종 설정 경로가 symlink이면 대상 내용이 정상이어도 no-follow open에서 실패합니다.
#[test]
fn symlink_configuration_is_rejected_without_following_its_target() {
    let root = std::env::temp_dir().join(format!("yo-config-symlink-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let target = root.join("target.yaml");
    let alias = root.join("config.yaml");
    fs::write(&target, "").unwrap();
    std::os::unix::fs::symlink(&target, &alias).unwrap();

    let error = load_from(&alias).unwrap_err();

    fs::remove_dir_all(root).unwrap();
    assert!(matches!(error, ConfigError::Io { .. }));
}

// 같은 bytes로 파일을 교체해도 identity metadata가 달라지면 stale 계획을 막습니다.
#[test]
fn final_config_guard_detects_same_byte_replacement() {
    let path = std::env::temp_dir().join(format!("yo-config-guard-{}", std::process::id()));
    fs::write(&path, "session: {}\n").unwrap();
    let config = load_from(&path).unwrap();
    assert!(config.verify_unchanged().is_ok());

    let replacement = path.with_extension("replacement");
    fs::write(&replacement, "session: {}\n").unwrap();
    fs::rename(&replacement, &path).unwrap();
    let error = config.verify_unchanged().unwrap_err();

    fs::remove_file(path).unwrap();
    assert!(matches!(error, ConfigError::Changed(_)));
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
