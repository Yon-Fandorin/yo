use std::path::Path;

use super::*;

// 설정 파일이 아직 없는 사용자는 파일이나 디렉터리를 새로 만들지 않아도 기본 날짜
// 형식을 사용할 수 있어, 읽기 전용 `yo session` 계약이 유지됩니다.
#[test]
fn missing_configuration_uses_defaults_without_creating_a_file() {
    let root = std::env::temp_dir().join(format!("yo-config-missing-{}", std::process::id()));
    let path = root.join("config.yaml");

    assert!(
        !path.exists(),
        "test path unexpectedly exists: {}",
        path.display()
    );
    let config = load_from(&path).unwrap();

    assert!(config.date_formatter().is_ok());
    assert_eq!(config.frame_rate_limit(), yo_tui::FrameRateLimit::Fps120);
    assert!(!path.exists());
    assert_eq!(
        config.snapshot_digest(),
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

// ConfigSnapshot digest는 exact bytes의 lowercase SHA-256 domain으로 안정적이어야 향후
// recovery journal이 같은 invocation의 공개 계획을 비밀 없이 식별할 수 있습니다.
#[test]
fn config_snapshot_digest_is_lowercase_sha256_of_exact_bytes() {
    let config = parse(Path::new("config.yaml"), "version: 1\n").unwrap();

    assert_eq!(
        config.snapshot_digest(),
        "sha256:09bfcc6a14b83e2192b8673677725c84883ee9cd0c70e45c9ec09daa8f2b2847"
    );
}

// 사용자가 60fps를 선택하면 설정을 시작 시의 typed frame 정책으로 해석합니다.
#[test]
fn tui_max_fps_accepts_60() {
    let config = parse(
        Path::new("config.yaml"),
        "version: 1\ntui:\n  max_fps: 60\n",
    )
    .unwrap();

    assert_eq!(config.frame_rate_limit(), yo_tui::FrameRateLimit::Fps60);
}

// 기본값과 같은 120도 명시할 수 있어 설정 파일이 실제 frame 정책을 온전히 표현합니다.
#[test]
fn tui_max_fps_accepts_120() {
    let config = parse(
        Path::new("config.yaml"),
        "version: 1\ntui:\n  max_fps: 120\n",
    )
    .unwrap();

    assert_eq!(config.frame_rate_limit(), yo_tui::FrameRateLimit::Fps120);
}

// startup namespace와 flat catalog entry가 하나의 완전한 Provider·Account·Model binding으로
// 검증되고, credential은 같은 디렉터리의 별도 파일에서만 찾는지 확인합니다.
#[test]
fn model_catalog_resolves_the_configured_startup_binding() {
    let path = Path::new("/tmp/yo/config.yaml");
    let config = parse(
        path,
        "version: 1\nmodel:\n  startup:\n    provider: qwencloud\n    account: token-plan\n    model: qwen3.8max\n  catalog:\n    - provider: qwencloud\n      provider_display_name: Qwen Cloud\n      account: token-plan\n      account_display_name: Token Plan\n      model: qwen3.8max\n      model_display_name: Qwen 3.8 Max\n      api_dialect: openai-responses\n      base_url: https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1\n      input_token_limit: 1000000\n      max_output_tokens: 65536\n      tokenizer_profile: utf8-bytes/v1\n",
    )
    .unwrap();

    let startup = config.startup_target().unwrap().model().unwrap();
    let selected = config
        .model_catalog()
        .resolve_model(startup.provider(), startup.account(), startup.model())
        .unwrap();
    assert_eq!(selected.binding().model_id().as_str(), "qwen3.8max");
    assert_eq!(
        selected.binding().connector_id().as_str(),
        "openai-responses"
    );
    assert_eq!(
        selected.binding().endpoint().as_str(),
        "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1"
    );
    assert_eq!(
        config.credential_path(),
        Path::new("/tmp/yo/credentials.yaml")
    );
}

// operator startup은 model 좌표뿐 아니라 exact HostTarget도 표현해야 Local Codex를
// implicit fallback이 아닌 명시적인 선택으로 유지할 수 있다.
#[test]
fn model_startup_accepts_exact_local_codex_host_target() {
    let config = parse(
        Path::new("/tmp/yo/config.yaml"),
        "version: 1\nmodel:\n  startup: host:codex\n",
    )
    .unwrap();

    assert_eq!(
        config.startup_target(),
        Some(&yo_core::StartupTarget::HostCodex)
    );
}

// 임의 문자열을 host target처럼 받아들이면 새 Host identity를 설정 오타로 만들 수 있어
// v1 operator 형식은 exact host:codex 외의 scalar를 명시적으로 거절한다.
#[test]
fn model_startup_rejects_unknown_host_target() {
    let error = parse(
        Path::new("/tmp/yo/config.yaml"),
        "version: 1\nmodel:\n  startup: host:other\n",
    )
    .unwrap_err();

    assert!(error.to_string().contains("must be exactly host:codex"));
}

// startup ModelId가 같은 Provider·Account catalog에 없으면 다른 계정이나 임의 첫 entry로
// 대체하지 않고 설정 경로를 포함한 오류로 실패합니다.
#[test]
fn model_startup_rejects_an_unconfigured_model() {
    let error = parse(
        Path::new("/tmp/yo/config.yaml"),
        "version: 1\nmodel:\n  startup:\n    provider: qwencloud\n    account: token-plan\n    model: absent\n",
    )
    .unwrap_err();

    assert!(error.to_string().contains("/tmp/yo/config.yaml"));
    assert!(
        error
            .to_string()
            .contains("does not name one configured entry")
    );
}

// 지원하지 않는 frame 비율은 조용히 보정하지 않고 정확한 설정 경로와 값으로 거절합니다.
#[test]
fn tui_max_fps_rejects_unsupported_values() {
    let error = parse(
        Path::new("/tmp/yo-config.yaml"),
        "version: 1\ntui:\n  max_fps: 30\n",
    )
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "/tmp/yo-config.yaml: tui.max_fps must be 60 or 120, not 30"
    );
}

// 사용자가 지정한 날짜 형식은 UPDATED와 STARTED가 공유할 하나의 검증된 formatter로
// 해석되어, 숫자 millisecond 대신 같은 규칙의 읽을 수 있는 시각을 만듭니다.
#[test]
fn custom_date_format_is_validated_and_applied() {
    let config = parse(
        Path::new("config.yaml"),
        "version: 1\nsession:\n  list:\n    date_format: '%Y'\n",
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

// 기본 위치를 정하는 HOME과 XDG_CONFIG_HOME은 현재 directory에 따라 의미가 바뀌는
// 상대경로를 거절해, 의도하지 않은 workspace 설정을 읽지 않습니다.
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

// 파일을 여는 시점의 크기와 무관하게 최대 크기보다 한 byte만 더 읽어 상한 초과를
// 판별하므로, 큰 설정을 YAML parser에 넘기거나 제한 없이 메모리에 올리지 않습니다.
#[test]
fn oversized_configuration_is_bounded_during_the_read() {
    let path = std::env::temp_dir().join(format!("yo-config-large-{}", std::process::id()));
    fs::write(&path, vec![b'a'; MAX_CONFIG_BYTES as usize + 1]).unwrap();

    let error = load_from(&path).unwrap_err();

    fs::remove_file(&path).unwrap();
    assert!(matches!(error, ConfigError::TooLarge(found) if found == path));
}

// FIFO를 regular file처럼 blocking open하면 writer가 나타날 때까지 `yo session`이
// 멈출 수 있으므로, nonblocking으로 연 descriptor의 타입을 확인해 즉시 거절합니다.
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

// config 최종 경로가 symlink이면 target의 내용과 권한이 정상이어도 no-follow open에서
// 거절되어 capture와 final guard 사이에 다른 파일로 바뀔 경로를 신뢰하지 않습니다.
#[test]
fn symlink_configuration_is_rejected_without_following_its_target() {
    let root = std::env::temp_dir().join(format!("yo-config-symlink-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let target = root.join("target.yaml");
    let alias = root.join("config.yaml");
    fs::write(&target, "version: 1\n").unwrap();
    std::os::unix::fs::symlink(&target, &alias).unwrap();

    let error = load_from(&alias).unwrap_err();

    fs::remove_dir_all(root).unwrap();
    assert!(matches!(error, ConfigError::Io { .. }));
}

// 한 invocation이 capture한 config handle의 bytes와 identity는 그대로면 guard가 통과하고,
// 같은 bytes를 다시 써도 새 identity metadata가 되면 stale 준비를 게시하지 않게 거절합니다.
#[test]
fn final_config_guard_detects_same_byte_replacement() {
    let path = std::env::temp_dir().join(format!("yo-config-guard-{}", std::process::id()));
    fs::write(&path, "version: 1\n").unwrap();
    let config = load_from(&path).unwrap();
    assert!(config.verify_unchanged().is_ok());

    let replacement = path.with_extension("replacement");
    fs::write(&replacement, "version: 1\n").unwrap();
    fs::rename(&replacement, &path).unwrap();
    let error = config.verify_unchanged().unwrap_err();

    fs::remove_file(path).unwrap();
    assert!(matches!(error, ConfigError::Changed(_)));
}

// 오타 난 설정 키를 무시하면 사용자는 형식이 적용됐다고 오해하므로, 정확한 파일
// 위치를 포함한 오류로 거절해 잘못된 설정을 즉시 고칠 수 있게 합니다.
#[test]
fn unknown_configuration_field_is_rejected() {
    let error = parse(
        Path::new("/tmp/yo-config.yaml"),
        "version: 1\nsession:\n  list:\n    date_formt: '%Y'\n",
    )
    .unwrap_err();

    assert!(error.to_string().contains("/tmp/yo-config.yaml"));
    assert!(error.to_string().contains("unknown field"));
}

// 공개 용어는 api_dialect 하나이므로 이전 임시 api_protocol key는 조용히 alias로
// 받아들이지 않고 unknown field로 거절합니다.
#[test]
fn obsolete_api_protocol_key_is_rejected() {
    let error = parse(
        Path::new("/tmp/yo-config.yaml"),
        "version: 1\nmodel:\n  catalog:\n    - provider: openrouter\n      account: default\n      model: openrouter/free\n      api_protocol: openai-responses\n      base_url: https://openrouter.ai/api/v1\n      input_token_limit: 100000\n      max_output_tokens: 8192\n      tokenizer_profile: o200k_base/v1\n",
    )
    .unwrap_err();

    assert!(error.to_string().contains("api_protocol"));
    assert!(error.to_string().contains("unknown field"));
}

// 공개 설정은 dialect만 선택하고 닫힌 runtime registry가 정확히 하나의 built-in
// connector identity를 파생합니다.
#[test]
fn chat_completions_dialect_derives_its_connector_without_a_public_selector() {
    let config = parse(
        Path::new("/tmp/yo/config.yaml"),
        "version: 1\nmodel:\n  catalog:\n    - provider: qwencloud\n      account: token-plan\n      model: deepseek-v4-flash-0731\n      api_dialect: openai-chat-completions\n      base_url: https://dashscope-intl.aliyuncs.com/compatible-mode/v1\n      input_token_limit: 65536\n      max_output_tokens: 8192\n      tokenizer_profile: utf8-bytes/v1\n",
    )
    .unwrap();
    let entry = &config.model_catalog().entries()[0];
    assert_eq!(
        entry.binding().api_dialect(),
        yo_core::ApiDialect::OpenAiChatCompletions
    );
    assert_eq!(
        entry.binding().connector_id().as_str(),
        "openai-chat-completions"
    );
}

// 끝나지 않은 `%`처럼 잘못된 strftime 문법은 실행 때 조용히 그대로 출력하지 않고
// 해당 설정 필드를 가리키는 명시적 오류로 막습니다.
#[test]
fn invalid_date_format_is_rejected() {
    let error = parse(
        Path::new("config.yaml"),
        "version: 1\nsession:\n  list:\n    date_format: '%Y %'\n",
    )
    .unwrap_err();

    assert!(error.to_string().contains("config.yaml"));
    assert!(error.to_string().contains("session.list.date_format"));
}
