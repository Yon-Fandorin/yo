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
    assert!(selected.explicit_profile().is_none());
}

// 새 bindings 형식은 Provider·Account의 endpoint와 base profile을 두 모델이 공유하고,
// 두 번째 모델이 명시한 출력 한도와 reasoning mapping만 완전 교체해 catalog를 만듭니다.
#[test]
fn model_bindings_resolve_base_profile_and_model_overrides() {
    let config = parse(
        Path::new("/tmp/yo/config.yaml"),
        r#"version: 1
model:
  startup:
    provider: qwencloud
    account: token-plan
    model: qwen-flash
  bindings:
    - provider: qwencloud
      provider_display_name: Qwen Cloud
      account: token-plan
      account_display_name: Token Plan
      base_url: https://example.test/v1/
      profile:
        api_dialect: openai-responses
        tokenizer_profile: utf8-bytes/v1
        input_token_limit: 1000000
        max_output_tokens: 65536
        reasoning_parameters:
          effort: medium
        optional_request_parameters: {}
        tool_capability_policy: local-tools/v1
        verification_profile: semantic-terminal/v1
      models:
        - model: qwen-max
          model_display_name: Qwen Max
        - model: qwen-flash
          model_display_name: Qwen Flash
          profile:
            max_output_tokens: 8192
            reasoning_parameters:
              effort: high
"#,
    )
    .unwrap();

    let max = &config.model_catalog().entries()[0];
    let flash = &config.model_catalog().entries()[1];
    assert_eq!(max.binding().endpoint(), flash.binding().endpoint());
    assert_eq!(max.binding().endpoint().as_str(), "https://example.test/v1");
    assert_eq!(max.context().max_output_tokens(), 65_536);
    assert_eq!(flash.context().max_output_tokens(), 8_192);
    let max_profile = max.explicit_profile().unwrap();
    let flash_profile = flash.explicit_profile().unwrap();
    assert_eq!(
        max_profile.reasoning_parameters(),
        &serde_norway::from_str::<yo_core::ModelProfileParameters>("{effort: medium}").unwrap()
    );
    assert_eq!(
        flash_profile.reasoning_parameters(),
        &serde_norway::from_str::<yo_core::ModelProfileParameters>("{effort: high}").unwrap()
    );
    assert_eq!(
        flash_profile.tool_capability_policy().as_str(),
        "local-tools/v1"
    );
    assert_eq!(
        config
            .startup_target()
            .unwrap()
            .model()
            .unwrap()
            .model()
            .as_str(),
        "qwen-flash"
    );
}

// 완전한 base profile을 가진 빈 OpenRouter binding은 routable catalog entry를 만들지
// 않으면서 정확한 Provider·Account discovery seed로 남고 다른 Provider의 빈 목록은 거절됩니다.
#[test]
fn empty_openrouter_binding_is_a_discovery_seed_only() {
    let config = parse(
        Path::new("/tmp/yo/config.yaml"),
        r#"version: 1
model:
  bindings:
    - provider: openrouter
      provider_display_name: OpenRouter
      account: team
      account_display_name: Team
      base_url: https://openrouter.ai/api/v1
      profile:
        api_dialect: openai-responses
        tokenizer_profile: o200k_base/v1
        input_token_limit: 200000
        max_output_tokens: 16000
        reasoning_parameters: {}
        optional_request_parameters: {}
        tool_capability_policy: local-tools/v1
        verification_profile: semantic-terminal/v1
      models: []
"#,
    )
    .unwrap();
    assert!(config.model_catalog().entries().is_empty());
    let provider = yo_core::ProviderId::new("openrouter").unwrap();
    let account = yo_core::AccountId::new("team").unwrap();
    let seed = config
        .openrouter_discovery_seed(&provider, &account)
        .unwrap();
    assert_eq!(seed.provider(), &provider);
    assert_eq!(seed.account(), &account);

    let error = parse(
        Path::new("config.yaml"),
        "version: 1\nmodel:\n  bindings:\n    - provider: qwencloud\n      account: team\n      base_url: https://example.test/v1\n      models: []\n",
    )
    .unwrap_err();
    assert!(error.to_string().contains("requires at least one model"));
}

// QwenCloud catalog binding은 Account와 표시 이름만 설정에서 받아 로컬 registry seed를
// 만들고, bundled 행 전체를 startup용 수동 ModelCatalog에 미리 확장하지 않습니다.
#[test]
fn qwencloud_catalog_binding_is_a_non_routable_local_seed() {
    let config = parse(
        Path::new("/tmp/yo/config.yaml"),
        r#"version: 1
model:
  bindings:
    - provider: qwencloud
      account: team
      account_display_name: Coding Team
      catalog: qwencloud-coding-plan-intl/v1
"#,
    )
    .unwrap();

    assert!(config.model_catalog().entries().is_empty());
    let provider = yo_core::ProviderId::new("qwencloud").unwrap();
    let account = yo_core::AccountId::new("team").unwrap();
    let seed = config.qwencloud_catalog_seed(&provider, &account).unwrap();
    assert_eq!(seed.profile().as_str(), "qwencloud-coding-plan-intl/v1");
    assert_eq!(seed.models().len(), 10);
    let row = seed
        .model(&yo_core::ModelId::new("qwen3-coder-next").unwrap())
        .unwrap();
    assert!(row.entry().is_some());
    assert_eq!(
        row.entry().unwrap().account_display_name(),
        Some("Coding Team")
    );
}

// catalog는 endpoint·profile·models를 하나의 registry 원자 단위로 소유하므로 같은
// binding에서 어느 수동 필드도 함께 쓰지 못하고 unknown profile/provider도 실패합니다.
#[test]
fn qwencloud_catalog_binding_rejects_mixed_or_unknown_shapes() {
    for extra in [
        "      base_url: https://example.test/v1\n",
        "      profile: {}\n",
        "      models: []\n",
    ] {
        let error = parse(
            Path::new("config.yaml"),
            &format!(
                "version: 1\nmodel:\n  bindings:\n    - provider: qwencloud\n      account: team\n      catalog: qwencloud-coding-plan-intl/v1\n{extra}"
            ),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("cannot author base_url, profile, or models"),
            "{error}"
        );
    }

    for (provider, catalog, expected) in [
        (
            "qwencloud",
            "qwencloud-future/v1",
            "unknown built-in catalog profile",
        ),
        (
            "vendor",
            "qwencloud-coding-plan-intl/v1",
            "requires ProviderId qwencloud",
        ),
    ] {
        let error = parse(
            Path::new("config.yaml"),
            &format!(
                "version: 1\nmodel:\n  bindings:\n    - provider: {provider}\n      account: team\n      catalog: {catalog}\n"
            ),
        )
        .unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
}

// 같은 v1 문서에서 legacy catalog와 새 bindings를 함께 쓰면 어느 쪽이 우선인지
// 추측하지 않고 정확한 상호배타 오류로 거절합니다.
#[test]
fn model_catalog_and_bindings_are_mutually_exclusive() {
    let error = parse(
        Path::new("config.yaml"),
        r#"version: 1
model:
  catalog: []
  bindings:
    - provider: explicit
      account: default
      base_url: https://explicit.test/v1
      models: []
"#,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("model.catalog and model.bindings cannot be authored together")
    );
}

// catalog와 bindings는 배열만 받으므로 null을 누락으로 축약하지 않으며, 단독 null과
// 다른 키를 동반한 null 모두 닫힌 authored shape 경계에서 실패합니다.
#[test]
fn model_catalog_and_bindings_reject_null_instead_of_treating_it_as_absent() {
    for model in [
        "catalog: null",
        "bindings: null",
        "catalog: null\n  bindings: []",
        "catalog: []\n  bindings: null",
    ] {
        let yaml = format!("version: 1\nmodel:\n  {model}\n");
        let error = parse(Path::new("config.yaml"), &yaml).unwrap_err();
        assert!(
            error.to_string().contains("expected a sequence"),
            "{model}: {error}"
        );
    }
}

// structured null은 필드 누락과 다르므로 모델 override가 상위 mapping을 상속하지 않고
// exact Null 값으로 교체하며, native runtime이 지원 여부를 별도로 판단할 수 있습니다.
#[test]
fn model_profile_structured_null_is_an_explicit_whole_field_override() {
    let config = parse(
        Path::new("config.yaml"),
        r#"version: 1
model:
  bindings:
    - provider: qwencloud
      account: default
      base_url: https://example.test/v1
      profile:
        api_dialect: openai-responses
        tokenizer_profile: utf8-bytes/v1
        input_token_limit: 1000
        max_output_tokens: 100
        reasoning_parameters: {effort: medium}
        optional_request_parameters: {}
        tool_capability_policy: local-tools/v1
        verification_profile: semantic-terminal/v1
      models:
        - model: null-reasoning
          profile:
            reasoning_parameters: null
"#,
    )
    .unwrap();

    assert_eq!(
        config.model_catalog().entries()[0]
            .explicit_profile()
            .unwrap()
            .reasoning_parameters()
            .to_json_value(),
        serde_json::Value::Null
    );
}

// profile의 문자열·정수 같은 shape 필드는 null을 상속 표시로 사용하지 않으므로,
// structured Null과 달리 authored type 경계에서 즉시 거절합니다.
#[test]
fn model_profile_scalar_null_is_not_treated_as_an_inherited_field() {
    let error = parse(
        Path::new("config.yaml"),
        r#"version: 1
model:
  bindings:
    - provider: qwencloud
      account: default
      base_url: https://example.test/v1
      profile:
        api_dialect: openai-responses
        tokenizer_profile: utf8-bytes/v1
        input_token_limit: 1000
        max_output_tokens: null
        reasoning_parameters: {}
        optional_request_parameters: {}
        tool_capability_policy: local-tools/v1
        verification_profile: semantic-terminal/v1
      models:
        - model: null-limit
"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("expected u64"));
}

// YAML plain 숫자는 spelling 기준 variant를 정하기 전에 범위를 검증하여 큰 정수가
// float/string으로 바뀌지 않으며, 유한 exponent와 따옴표 문자열은 구분해 허용합니다.
#[test]
fn model_profile_plain_numbers_reject_retyping_but_preserve_quoted_strings() {
    let prefix = r#"version: 1
model:
  bindings:
    - provider: qwencloud
      account: default
      base_url: https://example.test/v1
      profile:
        api_dialect: openai-responses
        tokenizer_profile: utf8-bytes/v1
        input_token_limit: 1000
        max_output_tokens: 100
        optional_request_parameters: {}
        tool_capability_policy: local-tools/v1
        verification_profile: semantic-terminal/v1
        reasoning_parameters:
          value: "#;
    let suffix = "\n      models:\n        - model: numeric\n";
    for invalid in [
        "18446744073709551616",
        "340282366920938463463374607431768211456",
        "1e400",
    ] {
        let error = parse(
            Path::new("config.yaml"),
            &format!("{prefix}{invalid}{suffix}"),
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("plain profile")
                || error.to_string().contains("profile integer"),
            "{invalid}: {error}"
        );
    }

    for invalid in [".5e400", "-.5e400", "0x10000000000000000"] {
        let error = parse(
            Path::new("config.yaml"),
            &format!("{prefix}{invalid}{suffix}"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("profile"), "{invalid}: {error}");
    }

    for valid in [
        "1e2",
        ".5e2",
        "\"1e400\"",
        "'1e400'",
        "!!str 1e400",
        "plain 1e400 text",
        "0x32",
        "1_000",
    ] {
        parse(
            Path::new("config.yaml"),
            &format!("{prefix}{valid}{suffix}"),
        )
        .unwrap();
    }

    for (authored, expected) in [
        (".5e2", serde_json::json!(50.0)),
        ("0x32", serde_json::json!(50)),
        ("1_000", serde_json::json!("1_000")),
    ] {
        let config = parse(
            Path::new("config.yaml"),
            &format!("{prefix}{authored}{suffix}"),
        )
        .unwrap();
        assert_eq!(
            config.model_catalog().entries()[0]
                .explicit_profile()
                .unwrap()
                .reasoning_parameters()
                .to_json_value()["value"],
            expected,
            "{authored}"
        );
    }

    for block_string in ["|\n            1e400", ">\n            1e400"] {
        parse(
            Path::new("config.yaml"),
            &format!("{prefix}{block_string}{suffix}"),
        )
        .unwrap();
    }

    parse(
        Path::new("config.yaml"),
        &format!("{prefix}plain\n            1e400{suffix}"),
    )
    .unwrap();

    let error = parse(
        Path::new("config.yaml"),
        r#"version: 1
model:
  bindings:
    - provider: &overflow 1e400
      account: default
      base_url: https://example.test/v1
      profile:
        api_dialect: openai-responses
        tokenizer_profile: utf8-bytes/v1
        input_token_limit: 1000
        max_output_tokens: 100
        reasoning_parameters:
          value: *overflow
        optional_request_parameters: {}
        tool_capability_policy: local-tools/v1
        verification_profile: semantic-terminal/v1
      models:
        - model: numeric
"#,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("plain profile number"),
        "{error}"
    );
}

// structured map의 key도 같은 lexical 경계를 통과하므로 범위 밖 plain 숫자와
// 그 alias는 문자열 key로 재해석되지 않고, 명시적으로 quote한 key만 허용합니다.
#[test]
fn model_profile_plain_number_map_keys_cannot_bypass_range_validation() {
    let prefix = r#"version: 1
model:
  bindings:
    - provider: qwencloud
      account: default
      base_url: https://example.test/v1
      profile:
        api_dialect: openai-responses
        tokenizer_profile: utf8-bytes/v1
        input_token_limit: 1000
        max_output_tokens: 100
        reasoning_parameters: "#;
    let suffix = r#"
        optional_request_parameters: {}
        tool_capability_policy: local-tools/v1
        verification_profile: semantic-terminal/v1
      models:
        - model: numeric-key
"#;

    for parameters in [
        "{1e400: value}",
        "{0x100000000000000000000000000000000: value}",
    ] {
        let error = parse(
            Path::new("config.yaml"),
            &format!("{prefix}{parameters}{suffix}"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("plain profile"), "{error}");
    }

    let error = parse(
        Path::new("config.yaml"),
        r#"version: 1
model:
  bindings:
    - provider: &overflow 1e400
      account: default
      base_url: https://example.test/v1
      profile:
        api_dialect: openai-responses
        tokenizer_profile: utf8-bytes/v1
        input_token_limit: 1000
        max_output_tokens: 100
        reasoning_parameters:
          ? *overflow
          : value
        optional_request_parameters: {}
        tool_capability_policy: local-tools/v1
        verification_profile: semantic-terminal/v1
      models:
        - model: numeric-key
"#,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("plain profile number"),
        "{error}"
    );

    let config = parse(
        Path::new("config.yaml"),
        &format!("{prefix}{{\"1e400\": value}}{suffix}"),
    )
    .unwrap();
    assert_eq!(
        config.model_catalog().entries()[0]
            .explicit_profile()
            .unwrap()
            .reasoning_parameters()
            .to_json_value()["1e400"],
        "value"
    );
}

// 명시적 YAML 숫자 tag도 spelling이 정한 integer/float variant를 바꿀 수 없으며,
// 일치하는 tag만 같은 typed profile 값으로 decode됩니다.
#[test]
fn model_profile_numeric_tags_must_match_the_authored_spelling() {
    let prefix = r#"version: 1
model:
  bindings:
    - provider: qwencloud
      account: default
      base_url: https://example.test/v1
      profile:
        api_dialect: openai-responses
        tokenizer_profile: utf8-bytes/v1
        input_token_limit: 1000
        max_output_tokens: 100
        reasoning_parameters: {value: "#;
    let suffix = r#"}
        optional_request_parameters: {}
        tool_capability_policy: local-tools/v1
        verification_profile: semantic-terminal/v1
      models:
        - model: tagged-number
"#;

    for mismatched in ["!!float 1", "!!int 1.0"] {
        let error = parse(
            Path::new("config.yaml"),
            &format!("{prefix}{mismatched}{suffix}"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("numeric tag"), "{error}");
    }

    for matching in ["!!int 1", "!!float 1.0"] {
        parse(
            Path::new("config.yaml"),
            &format!("{prefix}{matching}{suffix}"),
        )
        .unwrap();
    }
}

// profile 밖의 typed String은 숫자 모양이어도 그대로 식별자이므로 구조화 parameter의
// lexical 숫자 검사가 vendor ModelId까지 넓어지지 않습니다.
#[test]
fn model_profile_number_validation_does_not_retype_numeric_looking_model_ids() {
    for model in ["1e400", "18446744073709551616"] {
        let config = parse(
            Path::new("config.yaml"),
            &format!(
                r#"version: 1
model:
  bindings:
    - provider: qwencloud
      account: default
      base_url: https://example.test/v1
      profile:
        api_dialect: openai-responses
        tokenizer_profile: utf8-bytes/v1
        input_token_limit: 1000
        max_output_tokens: 100
        reasoning_parameters: {{}}
        optional_request_parameters: {{}}
        tool_capability_policy: local-tools/v1
        verification_profile: semantic-terminal/v1
      models:
        - model: {model}
"#
            ),
        )
        .unwrap();

        assert_eq!(
            config.model_catalog().entries()[0]
                .binding()
                .model_id()
                .as_str(),
            model
        );
    }
}

// 상위와 모델 profile을 합쳐도 필수 필드가 빠진 새 binding은 legacy 기본값을 끌어오지
// 않고 누락된 첫 필드를 지목해 실패합니다.
#[test]
fn model_bindings_require_a_complete_resolved_profile() {
    let error = parse(
        Path::new("config.yaml"),
        r#"version: 1
model:
  bindings:
    - provider: qwencloud
      account: default
      base_url: https://example.test/v1
      profile:
        api_dialect: openai-responses
      models:
        - model: incomplete
"#,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("resolved model profile is missing tokenizer_profile")
    );
}

// 한 Provider·Account가 두 binding 블록에서 서로 다른 endpoint나 profile을 갖지 않도록
// 좌표 중복을 catalog 평탄화 전에 거절합니다.
#[test]
fn model_bindings_reject_duplicate_provider_account_blocks() {
    let error = parse(
        Path::new("config.yaml"),
        r#"version: 1
model:
  bindings:
    - provider: qwencloud
      account: default
      base_url: https://one.test/v1
      profile:
        api_dialect: openai-responses
        tokenizer_profile: utf8-bytes/v1
        input_token_limit: 1000
        max_output_tokens: 100
        reasoning_parameters: {}
        optional_request_parameters: {}
        tool_capability_policy: local-tools/v1
        verification_profile: semantic-terminal/v1
      models:
        - model: one
    - provider: qwencloud
      account: default
      base_url: https://two.test/v1
      models:
        - model: two
"#,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("repeats Provider qwencloud and Account default")
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
