use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;

fn selection_entry(
    provider: &str,
    account: &str,
    model: &str,
    provider_label: &str,
    account_label: &str,
) -> ModelCatalogEntry {
    ModelCatalogEntry::new(
        EffectiveModelBinding::new(
            ProviderId::new(provider).unwrap(),
            AccountId::new(account).unwrap(),
            ModelId::new(model).unwrap(),
            ApiDialect::OpenAiResponses,
            NormalizedEndpoint::parse("https://example.test/v1").unwrap(),
        ),
        Some(provider_label.to_owned()),
        Some(account_label.to_owned()),
        None,
        ModelContextProfile::new(1_000, 100, "utf8-bytes/v1").unwrap(),
    )
    .unwrap()
}

// picker는 완전한 좌표 순서를 보존하고 직접 Model 선택은 현재 Provider와 Account에 닫힌다.
#[test]
fn selection_controller_orders_complete_bindings_and_scopes_direct_resolution() {
    let catalog = ModelCatalog::new(vec![
        selection_entry("qwencloud", "default", "same", "Qwen Cloud", "Default"),
        selection_entry("openrouter", "team", "same", "OpenRouter", "Team"),
        selection_entry("openrouter", "default", "other", "OpenRouter", "Default"),
        selection_entry("openrouter", "default", "same", "OpenRouter", "Default"),
    ])
    .unwrap();
    let current = ModelSelection::new(
        ProviderId::new("openrouter").unwrap(),
        AccountId::new("default").unwrap(),
        ModelId::new("other").unwrap(),
    );
    let controller = ModelSelectionController::new(catalog, Some(current));

    let ordered = controller
        .choices()
        .iter()
        .map(|choice| choice.selection().row_identity())
        .collect::<Vec<_>>();
    assert_eq!(ordered.len(), 4);
    assert!(ordered[0].contains("openrouter"));
    assert!(ordered[0].contains("default"));
    assert!(ordered[3].contains("qwencloud"));

    let selected = controller
        .resolve_direct(&ModelId::new("same").unwrap())
        .unwrap();
    assert_eq!(selected.provider().as_str(), "openrouter");
    assert_eq!(selected.account().as_str(), "default");
    assert_eq!(selected.model().as_str(), "same");
    assert!(
        controller
            .resolve_direct(&ModelId::new("missing").unwrap())
            .is_err()
    );
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the system clock is after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "yo-model-service-{}-{name}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
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

fn write_credential_file(path: &Path, contents: &str, mode: u32) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

fn qwen_binding(account: &str, model: &str) -> EffectiveModelBinding {
    EffectiveModelBinding::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new(account).unwrap(),
        ModelId::new(model).unwrap(),
        "openai-responses".parse().unwrap(),
        NormalizedEndpoint::parse(
            "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1/",
        )
        .unwrap(),
    )
}

// public dialect는 connector를 하나로 파생하며 durable identity가 다른 pair를 주장하면 거부한다.
#[test]
fn api_dialect_derives_exactly_one_builtin_connector_and_durable_mismatch_fails() {
    let chat = EffectiveModelBinding::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("token-plan").unwrap(),
        ModelId::new("deepseek-v4-flash-0731").unwrap(),
        ApiDialect::OpenAiChatCompletions,
        NormalizedEndpoint::parse("https://dashscope-intl.aliyuncs.com/compatible-mode/v1")
            .unwrap(),
    );
    assert_eq!(
        chat.connector_id().as_str(),
        ConnectorId::OPENAI_CHAT_COMPLETIONS
    );
    assert!(
        EffectiveModelBinding::from_durable(
            chat.provider_id().clone(),
            chat.account_id().clone(),
            chat.model_id().clone(),
            ConnectorId::new(ConnectorId::OPENAI_RESPONSES).unwrap(),
            ApiDialect::OpenAiChatCompletions,
            chat.endpoint().clone(),
        )
        .is_err()
    );
}

// stable identity는 표시 이름과 분리되므로 유효한 ID를 그대로 보존하고 공백이나 control
// 문자가 섞인 설정 key는 서로 다른 routing authority가 되기 전에 거절하는지 검증합니다.
#[test]
fn validates_stable_model_service_identities() {
    assert_eq!(ProviderId::new("qwencloud").unwrap().as_str(), "qwencloud");
    assert_eq!(
        ModelId::new("qwen/qwen3.8max").unwrap().as_str(),
        "qwen/qwen3.8max"
    );
    assert!(AccountId::new(" account").is_err());
    assert!(ModelId::new("model\nname").is_err());
}

// QwenCloud Token Plan base URL은 HTTPS canonical endpoint 하나로 정규화하되 credential,
// query, fragment를 endpoint identity 안에 받아들이지 않는지 검증합니다.
#[test]
fn normalizes_the_qwencloud_endpoint_and_rejects_unsafe_components() {
    let endpoint = NormalizedEndpoint::parse(
        "https://TOKEN-PLAN.ap-southeast-1.maas.aliyuncs.com:443/compatible-mode/v1/",
    )
    .unwrap();

    assert_eq!(
        endpoint.as_str(),
        "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1"
    );
    assert!(NormalizedEndpoint::parse("http://example.com/v1").is_err());
    assert!(NormalizedEndpoint::parse("https://key@example.com/v1").is_err());
    assert!(NormalizedEndpoint::parse("https://example.com/v1?mode=x").is_err());
    assert!(NormalizedEndpoint::parse("https://example.com/v1#fragment").is_err());
}

// parse조차 할 수 없는 endpoint 입력에 secret-looking text가 포함되어도 URL parser
// 진단에는 원문이 남지 않아 잘못 배치한 credential을 로그로 유출하지 않는지 검증합니다.
#[test]
fn redacts_unparseable_endpoint_input_from_diagnostics() {
    let error = NormalizedEndpoint::parse("sk-sensitive-value://[").unwrap_err();
    let diagnostic = format!("{error:?} {error}");

    assert!(!diagnostic.contains("sk-sensitive-value"));
}

// direct model 선택은 현재 Provider와 Account namespace 안에서만 exact ModelId를 찾고,
// 다른 Account에 같은 ModelId가 있어도 fallback하거나 임의 선택하지 않는지 검증합니다.
#[test]
fn resolves_models_only_inside_the_selected_provider_and_account() {
    let context = || ModelContextProfile::new(262_144, 8_192, "utf8-bytes/v1").unwrap();
    let first = ModelCatalogEntry::new(
        qwen_binding("token-a", "qwen3.8max"),
        None,
        None,
        None,
        context(),
    )
    .unwrap();
    let second = ModelCatalogEntry::new(
        qwen_binding("token-b", "qwen3.8max"),
        None,
        None,
        None,
        context(),
    )
    .unwrap();
    let catalog = ModelCatalog::new(vec![first, second]).unwrap();

    let selected = catalog
        .resolve_model(
            &ProviderId::new("qwencloud").unwrap(),
            &AccountId::new("token-b").unwrap(),
            &ModelId::new("qwen3.8max").unwrap(),
        )
        .unwrap();
    assert_eq!(selected.binding().account_id().as_str(), "token-b");
    assert_eq!(selected.context().input_token_limit(), 262_144);
    assert_eq!(selected.context().max_output_tokens(), 8_192);
    assert_eq!(selected.context().tokenizer_profile(), "utf8-bytes/v1");
    assert!(
        catalog
            .resolve_model(
                &ProviderId::new("qwencloud").unwrap(),
                &AccountId::new("missing").unwrap(),
                &ModelId::new("qwen3.8max").unwrap(),
            )
            .is_err()
    );
}

// flat catalog의 반복 entry가 같은 stable Provider/Account에 서로 다른 표시 이름을 붙이면
// picker마다 이름이 달라질 수 있으므로 catalog 생성 단계에서 모순을 거절합니다.
#[test]
fn rejects_inconsistent_provider_and_account_display_names() {
    let context = || ModelContextProfile::new(100, 10, "test/v1").unwrap();
    let provider_mismatch = ModelCatalog::new(vec![
        ModelCatalogEntry::new(
            qwen_binding("default", "model-a"),
            Some("QwenCloud".to_owned()),
            Some("Default".to_owned()),
            None,
            context(),
        )
        .unwrap(),
        ModelCatalogEntry::new(
            qwen_binding("default", "model-b"),
            Some("Qwen Cloud".to_owned()),
            Some("Default".to_owned()),
            None,
            context(),
        )
        .unwrap(),
    ])
    .unwrap_err();
    assert!(provider_mismatch.to_string().contains("Provider qwencloud"));

    let account_mismatch = ModelCatalog::new(vec![
        ModelCatalogEntry::new(
            qwen_binding("default", "model-a"),
            Some("QwenCloud".to_owned()),
            Some("Primary".to_owned()),
            None,
            context(),
        )
        .unwrap(),
        ModelCatalogEntry::new(
            qwen_binding("default", "model-b"),
            Some("QwenCloud".to_owned()),
            Some("Default".to_owned()),
            None,
            context(),
        )
        .unwrap(),
    ])
    .unwrap_err();
    assert!(account_mismatch.to_string().contains("Account default"));
}

// context profile은 양수 token limit, 그보다 작은 양수 reserve, 식별 가능한 tokenizer profile을
// 모두 요구함을 검증합니다.
#[test]
fn rejects_incomplete_model_context_profiles() {
    assert!(ModelContextProfile::new(0, 1, "qwen/v1").is_err());
    assert!(ModelContextProfile::new(100, 0, "qwen/v1").is_err());
    assert!(ModelContextProfile::new(100, 100, "qwen/v1").is_err());
    assert!(ModelContextProfile::new(100, 10, "").is_err());
}

// resolved secret은 connector가 내부적으로 읽을 수 있어도 Debug와 Display projection에는
// 원문이나 일부 prefix가 나타나지 않고 store Debug도 계정 수만 보고하는지 검증합니다.
#[test]
fn redacts_api_credentials_from_diagnostics() {
    let secret = ApiCredential::new("sk-sensitive-value").unwrap();
    let store = CredentialStore::new([(
        (
            ProviderId::new("qwencloud").unwrap(),
            AccountId::new("token-plan").unwrap(),
        ),
        secret.clone(),
    )])
    .unwrap();

    assert_eq!(secret.expose_secret(), "sk-sensitive-value");
    assert_eq!(format!("{secret:?}"), "ApiCredential([REDACTED])");
    assert_eq!(secret.to_string(), "[REDACTED]");
    assert_eq!(
        format!("{store:?}"),
        "CredentialStore { credential_count: 1 }"
    );
    assert!(!format!("{store:?}").contains("sensitive"));
    assert!(store.contains_secret_material("prefix sk-sensitive-value suffix"));
    assert!(!store.contains_secret_material("ordinary tool output"));
}

// credential 파일이 아직 없는 첫 실행은 파일이나 부모 디렉터리를 만들지 않고 빈
// immutable snapshot을 반환해야 읽기 동작이 사용자 상태를 암묵적으로 바꾸지 않습니다.
#[test]
fn missing_local_credential_file_is_an_empty_non_creating_store() {
    let directory = TestDirectory::new("missing");
    let path = directory.path().join("nested/credentials.yaml");

    let store = LocalCredentialStore::open(&path).unwrap();

    assert!(store.is_empty());
    assert!(!path.exists());
    assert!(!path.parent().unwrap().exists());
}

// 현재 사용자가 소유한 0600 regular file의 version 1 Provider/Account mapping은 지정된
// exact pair에서만 원문 API key를 resolve하는 시작 시점 snapshot으로 읽히는지 검증합니다.
#[test]
fn loads_a_user_only_provider_account_credential_snapshot() {
    let directory = TestDirectory::new("valid");
    let path = directory.path().join("credentials.yaml");
    write_credential_file(
        &path,
        "version: 1\nproviders:\n  qwencloud:\n    token-plan:\n      api_key: sk-sensitive-value\n",
        0o600,
    );

    let store = LocalCredentialStore::open(&path).unwrap();
    let credential = store
        .resolve(
            &ProviderId::new("qwencloud").unwrap(),
            &AccountId::new("token-plan").unwrap(),
        )
        .unwrap();

    assert_eq!(credential.expose_secret(), "sk-sensitive-value");
    assert_eq!(store.len(), 1);
}

// 서로 다른 Provider는 같은 AccountId를 독립적으로 사용할 수 있어야 하며 exact pair가
// 없을 때 다른 Provider의 같은 이름 credential로 fall through하지 않는지 검증합니다.
#[test]
fn isolates_the_same_account_id_between_providers() {
    let directory = TestDirectory::new("provider-scope");
    let path = directory.path().join("credentials.yaml");
    write_credential_file(
        &path,
        "version: 1\nproviders:\n  openrouter:\n    default:\n      api_key: sk-openrouter\n  qwencloud:\n    default:\n      api_key: sk-qwencloud\n",
        0o600,
    );

    let store = LocalCredentialStore::open(&path).unwrap();
    let account = AccountId::new("default").unwrap();

    assert_eq!(
        store
            .resolve(&ProviderId::new("openrouter").unwrap(), &account)
            .unwrap()
            .expose_secret(),
        "sk-openrouter"
    );
    assert_eq!(
        store
            .resolve(&ProviderId::new("qwencloud").unwrap(), &account)
            .unwrap()
            .expose_secret(),
        "sk-qwencloud"
    );
    assert!(
        store
            .resolve(&ProviderId::new("missing-provider").unwrap(), &account)
            .is_none()
    );
}

// group 또는 other permission이 하나라도 열린 credential 파일은 내용이 올바르더라도
// 읽기 전에 거절해야 로컬의 다른 계정에 API key가 노출되는 설정을 허용하지 않습니다.
#[test]
fn rejects_a_credential_file_with_group_or_other_permissions() {
    let directory = TestDirectory::new("permissions");
    let path = directory.path().join("credentials.yaml");
    write_credential_file(&path, "version: 1\nproviders: {}\n", 0o640);

    let error = LocalCredentialStore::open(&path).unwrap_err();

    assert!(matches!(
        error,
        LocalCredentialStoreError::InsecurePermissions(ref rejected) if rejected == &path
    ));
}

// 최종 경로의 symlink는 target이 안전한 0600 regular file이어도 no-follow open에서
// 거절되어 경로 확인과 실제 credential read 사이의 대상을 바꿀 수 없음을 검증합니다.
#[test]
fn rejects_a_symlink_instead_of_following_it_to_credentials() {
    let directory = TestDirectory::new("symlink");
    let target = directory.path().join("actual.yaml");
    let alias = directory.path().join("credentials.yaml");
    write_credential_file(&target, "version: 1\nproviders: {}\n", 0o600);
    symlink(&target, &alias).unwrap();

    assert!(matches!(
        LocalCredentialStore::open(&alias),
        Err(LocalCredentialStoreError::Io { .. })
    ));
}

// regular file이 아닌 directory는 nonblocking handle로 열릴 수 있는 플랫폼에서도
// 같은 handle의 metadata 검증에서 거절되어 무제한 또는 특수 입력으로 읽히지 않습니다.
#[test]
fn rejects_non_regular_credential_sources() {
    let directory = TestDirectory::new("directory");

    assert!(matches!(
        LocalCredentialStore::open(directory.path()),
        Err(LocalCredentialStoreError::UnsupportedFileType(ref rejected))
            if rejected == directory.path()
    ));
}

// 허용 한도보다 큰 regular file은 YAML parse 전에 typed size failure로 거절되어
// credential 입력이 시작 시점 메모리를 무제한 소비하지 않는지 검증합니다.
#[test]
fn rejects_an_oversized_credential_file_before_parsing() {
    let directory = TestDirectory::new("oversized");
    let path = directory.path().join("credentials.yaml");
    write_credential_file(&path, &"x".repeat(64 * 1024 + 1), 0o600);

    assert!(matches!(
        LocalCredentialStore::open(&path),
        Err(LocalCredentialStoreError::TooLarge(ref rejected)) if rejected == &path
    ));
}

// malformed 또는 중복 Provider/Account YAML에 secret-looking 원문이 들어 있어도 외부 오류에는
// 경로와 일반 사유만 남겨 credential 값이나 parser 문맥이 유출되지 않는지 검증합니다.
#[test]
fn redacts_secret_material_from_invalid_credential_diagnostics() {
    let directory = TestDirectory::new("invalid");
    let path = directory.path().join("credentials.yaml");
    write_credential_file(
        &path,
        "version: 1\nproviders:\n  qwencloud:\n    duplicate:\n      api_key: sk-first-secret\n    duplicate:\n      api_key: sk-second-secret\n",
        0o600,
    );

    let error = LocalCredentialStore::open(&path).unwrap_err();
    let diagnostic = format!("{error:?} {error}");

    assert!(matches!(
        error,
        LocalCredentialStoreError::InvalidContents(_)
    ));
    assert!(!diagnostic.contains("sk-first-secret"));
    assert!(!diagnostic.contains("sk-second-secret"));
}
