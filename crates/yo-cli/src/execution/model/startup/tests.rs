use std::{fs, path::PathBuf, time::SystemTime};

use yo_core::{EffectiveModelProfile, ModelProfileParameters, VersionedProfileId};

use super::*;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "yo-cli-model-startup-{}-{name}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn complete_binding(effort: &str) -> CompleteModelBinding {
    CompleteModelBinding::from_durable_json(&format!(
        r#"{{"provider":"qwencloud","account":"default","model":"model","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{{"effort":"{effort}"}},"optional_request_parameters":{{}},"tool_capability_policy":"local-tools/v1"}}"#
    ))
    .unwrap()
}

fn disabled_complete_catalog() -> (TestDirectory, yo_core::ModelCatalog, CompleteModelBinding) {
    let directory = TestDirectory::new("disabled-catalog");
    let repository = yo_core::LocalConnectionRepository::new(directory.0.join("connections.yaml"));
    let complete = complete_binding("medium");
    let account = yo_core::ConnectionAccount::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("default").unwrap(),
        None,
        None,
    )
    .unwrap();
    let binding = yo_core::StoredModelBinding::new(complete.clone(), None).unwrap();
    let selection = binding.selection();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(account, binding)
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_activation(&selection, false)
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let catalog = repository.capture().unwrap().model_catalog().unwrap();
    (directory, catalog, complete)
}

fn selection_catalog(entries: &[(&str, &str, &str)]) -> yo_core::ModelCatalog {
    selection_catalog_with_tokenizer(entries, "utf8-bytes/v1")
}

fn selection_catalog_with_tokenizer(
    entries: &[(&str, &str, &str)],
    tokenizer_profile: &str,
) -> yo_core::ModelCatalog {
    yo_core::ModelCatalog::new(
        entries
            .iter()
            .map(|(provider, account, model)| {
                yo_core::ModelCatalogEntry::new(
                    yo_core::EffectiveModelBinding::new(
                        ProviderId::new(*provider).unwrap(),
                        AccountId::new(*account).unwrap(),
                        ModelId::new(*model).unwrap(),
                        ApiDialect::OpenAiResponses,
                        NormalizedEndpoint::parse("https://example.test/v1").unwrap(),
                    ),
                    None,
                    None,
                    None,
                    yo_core::ModelContextProfile::new(1_000, 100, tokenizer_profile).unwrap(),
                )
                .unwrap()
            })
            .collect(),
    )
    .unwrap()
}

fn codex_binding(schema: &str) -> yo_core::BackendBindingEvidence {
    yo_core::BackendBindingEvidence::new(
        "codex-app-server",
        "codex/test",
        yo_core::BackendIdentity::new(
            schema,
            serde_json::json!({ "sessionId": "session", "threadId": "thread" }).to_string(),
        ),
        yo_core::BackendIdentity::new("codex/model/v1", "model"),
        yo_core::BackendIdentity::new("codex.app-server/thread-locator/v1", "thread"),
        yo_core::ContinuationStrategy::BackendManagedState,
    )
}

fn grok_binding(schema: &str) -> yo_core::BackendBindingEvidence {
    yo_core::BackendBindingEvidence::new(
        "grok-build-acp",
        "grok/test",
        yo_core::BackendIdentity::new(
            schema,
            serde_json::json!({ "sessionId": "session" }).to_string(),
        ),
        yo_core::BackendIdentity::new("grok/model/v1", "backend-managed"),
        yo_core::BackendIdentity::new("grok.acp/session-locator/v1", "session"),
        yo_core::ContinuationStrategy::BackendManagedState,
    )
}

// startup source가 모두 비면 setup guidance로 실패하고, Local Codex도 host target으로
// 명시해야 한다. unique bare ModelId와 완전한 좌표는 operator startup 없이 선택한다.
#[test]
fn new_session_requires_a_target_and_accepts_host_unique_or_complete_references() {
    let catalog = selection_catalog(&[
        ("qwencloud", "default", "qwen3.8-max"),
        ("openrouter", "default", "openrouter/free"),
    ]);

    let missing = resolve_new_session(&catalog, None, None, None).unwrap_err();
    assert_eq!(missing.to_string(), "no startup target is selected");
    assert_eq!(
        missing.help(),
        [
            "yo connect",
            "yo --model host:codex",
            "yo --model host:grok"
        ]
    );

    assert!(matches!(
        resolve_new_session(&catalog, None, None, Some("host:codex")).unwrap(),
        StartupBackend::Host(host) if host.as_str() == HostId::CODEX
    ));
    assert!(matches!(
        resolve_new_session(&catalog, None, None, Some("host:grok")).unwrap(),
        StartupBackend::Host(host) if host.as_str() == HostId::GROK
    ));
    let bare = resolve_new_session(&catalog, None, None, Some("qwen3.8-max")).unwrap();
    let selected = bare.model_selection().unwrap();
    assert_eq!(selected.provider().as_str(), "qwencloud");
    assert_eq!(selected.account().as_str(), "default");
    assert_eq!(selected.model().as_str(), "qwen3.8-max");

    let complete = resolve_new_session(
        &catalog,
        None,
        None,
        Some("openrouter:default:openrouter/free"),
    )
    .unwrap();
    assert_eq!(
        complete.model_selection().unwrap().model().as_str(),
        "openrouter/free"
    );
}

// 새 Session은 explicit invocation이 저장 기본값보다 우선하고, invocation이 없을 때는
// 저장 기본값이 operator startup보다 먼저 선택되어 영속화한 사용자 의도가 적용됩니다.
#[test]
fn stored_preference_sits_between_invocation_and_operator_startup() {
    let catalog = selection_catalog(&[("qwencloud", "default", "operator")]);
    let operator = StartupTarget::Model(ModelSelection::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("default").unwrap(),
        ModelId::new("operator").unwrap(),
    ));

    let stored = resolve_new_session(
        &catalog,
        Some(StartupTarget::host_codex()),
        Some(operator.clone()),
        None,
    )
    .unwrap();
    assert!(matches!(
        stored,
        StartupBackend::Host(host) if host == HostId::codex()
    ));

    let invoked = resolve_new_session(
        &catalog,
        Some(StartupTarget::host_codex()),
        Some(operator),
        Some("qwencloud:default:operator"),
    )
    .unwrap();
    assert_eq!(
        invoked.model_selection().unwrap().model().as_str(),
        "operator"
    );
}

// configured startup이 있으면 bare form은 그 Provider·Account에 머물고 qualified form은
// 다른 정확한 좌표로 갈 수 있으며, 중복 bare form은 namespace 없이 모호하게 실패한다.
#[test]
fn startup_reference_scope_is_contextual_and_ambiguity_is_explicit() {
    let catalog = selection_catalog(&[
        ("qwencloud", "default", "same"),
        ("qwencloud", "team", "same"),
        ("openrouter", "default", "same"),
    ]);
    let startup = ModelSelection::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("team").unwrap(),
        ModelId::new("same").unwrap(),
    );

    let contextual = resolve_new_session(
        &catalog,
        None,
        Some(StartupTarget::Model(startup.clone())),
        Some("same"),
    )
    .unwrap();
    assert_eq!(
        contextual.model_selection().unwrap().account().as_str(),
        "team"
    );
    let qualified = resolve_new_session(
        &catalog,
        None,
        Some(StartupTarget::Model(startup)),
        Some("openrouter::same"),
    )
    .unwrap();
    assert_eq!(
        qualified.model_selection().unwrap().provider().as_str(),
        "openrouter"
    );

    let error = match resolve_new_session(&catalog, None, None, Some("same")) {
        Ok(_) => panic!("duplicate bare ModelId must remain ambiguous"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("is ambiguous"));
    assert!(error.contains("openrouter:default:same"));
    assert!(error.contains("qwencloud:default:same"));
}

// option을 생략해 configured startup을 그대로 쓰더라도 catalog에서 제거된 낡은
// 좌표를 backend 시작 경계 밖으로 통과시키지 않는지 검증한다.
#[test]
fn unchanged_startup_must_still_resolve_in_the_current_catalog() {
    let catalog = selection_catalog(&[("qwencloud", "default", "current")]);
    let stale = ModelSelection::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("default").unwrap(),
        ModelId::new("removed").unwrap(),
    );

    let error = match resolve_new_session(&catalog, None, Some(StartupTarget::Model(stale)), None) {
        Ok(_) => panic!("a stale configured startup must fail closed"),
        Err(error) => error.to_string(),
    };

    assert!(error.contains("resolving startup target"));
    assert!(error.contains("Model removed"));
}

// Codex durable binding은 exact host target으로 같은 binding을 확인할 수 있지만 다른
// model target은 cross-backend handoff로 해석하지 않는다.
#[test]
fn codex_resume_accepts_only_absent_or_exact_host_override() {
    assert!(matches!(
        resolve_host_resume(
            HostId::codex(),
            None,
            &codex_binding("codex.app-server/thread-binding/v1")
        )
        .unwrap(),
        StartupBackend::Host(host) if host.as_str() == HostId::CODEX
    ));
    assert!(matches!(
        resolve_host_resume(
            HostId::codex(),
            Some("host:codex"),
            &codex_binding("codex.app-server/thread-binding/v1")
        )
        .unwrap(),
        StartupBackend::Host(host) if host.as_str() == HostId::CODEX
    ));
    let binding = codex_binding("codex.app-server/thread-binding/v1");
    assert!(resolve_host_resume(HostId::codex(), Some("host:grok"), &binding).is_err());
    assert!(resolve_host_resume(HostId::codex(), Some("qwen3.8-max"), &binding).is_err());
}

// 읽기 전용 리뷰는 새 delegated host에만 선택할 수 있고 Codex·Grok 모두 같은
// 실행 프로필로 해석되며, native model에 제한이 조용히 누락되지 않습니다.
#[test]
fn read_only_review_selects_only_delegated_hosts() {
    let catalog = selection_catalog(&[("qwencloud", "default", "model")]);
    for reference in ["host:codex", "host:grok"] {
        assert!(matches!(
            resolve_new_session_with_tool_restriction(
                &catalog,
                None,
                None,
                Some(reference),
                false,
                true,
            )
            .unwrap(),
            StartupBackend::ReadOnlyHost(_)
        ));
    }

    let error =
        resolve_new_session_with_tool_restriction(&catalog, None, None, Some("model"), false, true)
            .unwrap_err();
    assert!(error.to_string().contains("delegated host Sessions"));
}

// resume 호출에는 `--sandbox`를 다시 받지 않고 저장된 binding schema에서 제한
// 프로필을 복원하며, 모르는 schema는 권한 완화 위험 때문에 시작 전에 거절합니다.
#[test]
fn delegated_resume_restores_the_durable_execution_profile() {
    for (host, binding) in [
        (
            HostId::codex(),
            codex_binding("codex.app-server/thread-binding/v1alpha2"),
        ),
        (
            HostId::codex(),
            codex_binding("codex.app-server/thread-binding/v1alpha1"),
        ),
        (
            HostId::grok(),
            grok_binding("grok.acp/session-binding/v1alpha1"),
        ),
    ] {
        assert!(matches!(
            resolve_host_resume(host, None, &binding).unwrap(),
            StartupBackend::ReadOnlyHost(_)
        ));
    }

    assert!(matches!(
        resolve_host_resume(
            HostId::codex(),
            None,
            &codex_binding("codex.app-server/thread-binding/v2")
        )
        .unwrap(),
        StartupBackend::Host(_)
    ));

    let unknown = codex_binding("codex.app-server/thread-binding/v3");
    let error = resolve_host_resume(HostId::codex(), None, &unknown).unwrap_err();
    assert!(error.to_string().contains("permission downgrade"));
}

// 저장된 backend kind를 추측해 Codex로 보내면 locator identity를 바꾸므로 알려진 두
// runtime kind만 분류하고 다른 값은 read-only guidance와 함께 fail closed 한다.
#[test]
fn durable_backend_classification_rejects_unknown_kinds() {
    assert_eq!(
        classify_durable_backend("codex-app-server").unwrap(),
        DurableBackendKind::Host(HostId::codex())
    );
    assert_eq!(
        classify_durable_backend("grok-build-acp").unwrap(),
        DurableBackendKind::Host(HostId::grok())
    );
    assert_eq!(
        classify_durable_backend("yo-managed-model").unwrap(),
        DurableBackendKind::Native
    );
    let error = classify_durable_backend("delegated-provider")
        .unwrap_err()
        .to_string();
    assert!(error.contains("unsupported durable backend kind"));
    assert!(error.contains("read-only"));
}

// native resume의 bare form은 durable Provider·Account에 남고 qualified form만 다른
// configured coordinate를 골라 기존 exact-replay replacement flag를 세운다.
#[test]
fn native_resume_reference_uses_the_durable_namespace_or_an_exact_qualified_replacement() {
    let catalog = selection_catalog(&[
        ("qwencloud", "default", "same"),
        ("openrouter", "default", "same"),
    ]);
    let durable = catalog
        .resolve_model(
            &ProviderId::new("qwencloud").unwrap(),
            &AccountId::new("default").unwrap(),
            &ModelId::new("same").unwrap(),
        )
        .unwrap()
        .binding()
        .clone();

    let same = resolve_native_resume(
        &catalog,
        DurableNativeBinding::Legacy(durable.clone()),
        Some("same"),
        crate::execution::tools::LocalToolRegistryRevision::BasicFiles,
    )
    .unwrap();
    assert!(!same.replaces_binding());
    assert_eq!(
        same.model_selection().unwrap().provider().as_str(),
        "qwencloud"
    );

    let handoff = resolve_native_resume(
        &catalog,
        DurableNativeBinding::Legacy(durable.clone()),
        Some("host:codex"),
        crate::execution::tools::LocalToolRegistryRevision::BasicFiles,
    )
    .unwrap_err()
    .to_string();
    assert!(handoff.contains("cannot replace a native model Session"));
    assert!(handoff.contains("cross-backend handoff is not supported"));

    let replacement = resolve_native_resume(
        &catalog,
        DurableNativeBinding::Legacy(durable),
        Some("openrouter::same"),
        crate::execution::tools::LocalToolRegistryRevision::BasicFiles,
    )
    .unwrap();
    assert!(replacement.replaces_binding());
    assert_eq!(
        replacement.model_selection().unwrap().provider().as_str(),
        "openrouter"
    );
}

// durable identity는 좌표뿐 아니라 connector·dialect·endpoint까지 복원하므로 같은
// Provider·Account·Model 아래 endpoint 변경도 새 binding으로 구분된다.
#[test]
fn durable_binding_parser_preserves_the_complete_effective_identity() {
    let durable = parse_durable_binding(
        "yo.model-binding/v1",
        r#"{"provider":"qwencloud","account":"token-plan","model":"qwen3.8max","connector":"openai-responses","api_dialect":"openai-responses","base_url":"https://old.example/v1"}"#,
    )
    .unwrap();
    let binding = durable.binding();
    let changed = yo_core::EffectiveModelBinding::new(
        binding.provider_id().clone(),
        binding.account_id().clone(),
        binding.model_id().clone(),
        ApiDialect::OpenAiResponses,
        NormalizedEndpoint::parse("https://new.example/v1").unwrap(),
    );

    assert_ne!(binding, &changed);
    assert_eq!(binding.endpoint().as_str(), "https://old.example/v1");
}

// durable identity parser는 v1에서 알 수 없는 JSON 필드를 현재 무시하므로, 같은
// canonical binding을 extra field 없이 읽거나 붙여 읽어도 complete identity가 같다.
#[test]
fn durable_binding_parser_ignores_unknown_json_fields() {
    let canonical = r#"{"provider":"qwencloud","account":"token-plan","model":"qwen3.8max","connector":"openai-responses","api_dialect":"openai-responses","base_url":"https://old.example/v1"}"#;
    let with_unknown_field = r#"{"provider":"qwencloud","account":"token-plan","model":"qwen3.8max","connector":"openai-responses","api_dialect":"openai-responses","base_url":"https://old.example/v1","unknown_field":"ignored"}"#;

    let binding = parse_durable_binding("yo.model-binding/v1", canonical).unwrap();
    let binding_with_unknown_field =
        parse_durable_binding("yo.model-binding/v1", with_unknown_field).unwrap();

    assert_eq!(binding_with_unknown_field, binding);
}

// 새 complete binding은 endpoint·connector와 현재 profile 필드를 모두 복원하고,
// structured integer/float 구분을 포함한 exact profile이 같을 때만 같은 epoch입니다.
#[test]
fn complete_durable_binding_preserves_every_resolved_profile_field() {
    let durable = parse_durable_binding(
        "yo.complete-model-binding/v1",
        r#"{"provider":"qwencloud","account":"token-plan","model":"qwen3.8max","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000000,"max_output_tokens":65536,"reasoning_parameters":{"effort":"medium","integer":1,"float":1.0},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1"}"#,
    )
    .unwrap();
    let DurableNativeBinding::Complete(complete) = durable else {
        panic!("complete schema must produce a complete binding");
    };
    let binding = complete.binding();
    let profile = complete.profile();

    assert_eq!(binding.endpoint().as_str(), "https://example.test/v1");
    assert_eq!(profile.context().input_token_limit(), 1_000_000);
    assert_eq!(profile.context().max_output_tokens(), Some(65_536));
    assert_eq!(profile.context().tokenizer_profile(), "utf8-bytes/v1");
    assert_eq!(
        profile.reasoning_parameters(),
        &serde_json::from_str::<ModelProfileParameters>(
            r#"{"effort":"medium","integer":1,"float":1.0}"#
        )
        .unwrap()
    );
    assert_eq!(profile.tool_capability_policy().as_str(), "local-tools/v1");
}

// resume은 좌표와 endpoint가 같아도 현재 resolved profile이 달라지면 replacement를
// 요구하고, exact complete binding일 때만 기존 epoch를 그대로 재사용합니다.
#[test]
fn native_resume_compares_the_complete_explicit_profile() {
    let durable = parse_durable_binding(
        "yo.complete-model-binding/v1",
        r#"{"provider":"qwencloud","account":"default","model":"model","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{"effort":"medium"},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1"}"#,
    )
    .unwrap();
    let DurableNativeBinding::Complete(complete) = &durable else {
        panic!("complete schema must produce a complete binding");
    };
    let binding = complete.binding();
    let profile = complete.profile();
    let exact_catalog = yo_core::ModelCatalog::new(vec![
        yo_core::ModelCatalogEntry::with_explicit_profile(
            binding.clone(),
            None,
            None,
            None,
            profile.clone(),
        )
        .unwrap(),
    ])
    .unwrap();
    assert!(
        !resolve_native_resume(
            &exact_catalog,
            durable.clone(),
            None,
            crate::execution::tools::LocalToolRegistryRevision::BasicFiles,
        )
        .unwrap()
        .replaces_binding()
    );

    let changed_layer = yo_core::ModelProfileLayer::new(
        Some(ApiDialect::OpenAiResponses),
        Some(VersionedProfileId::new("utf8-bytes/v1").unwrap()),
        Some(1_000),
        Some(50),
        Some(serde_json::from_str(r#"{"effort":"medium"}"#).unwrap()),
        Some(serde_json::from_str("{}").unwrap()),
        Some(VersionedProfileId::new("local-tools/v1").unwrap()),
    );
    let changed_profile = EffectiveModelProfile::resolve(None, &changed_layer).unwrap();
    let changed_catalog = yo_core::ModelCatalog::new(vec![
        yo_core::ModelCatalogEntry::with_explicit_profile(
            binding.clone(),
            None,
            None,
            None,
            changed_profile,
        )
        .unwrap(),
    ])
    .unwrap();
    assert!(
        resolve_native_resume(
            &changed_catalog,
            durable,
            None,
            crate::execution::tools::LocalToolRegistryRevision::BasicFiles,
        )
        .unwrap()
        .replaces_binding()
    );
}

// 이미 admit된 native Session은 exact complete binding을 그대로 resume할 때만 disabled
// 상태를 통과하고, 같은 좌표의 명시 override나 달라진 durable binding은 새 선택으로
// 간주되어 시작 전에 거절됩니다.
#[test]
fn disabled_native_binding_resumes_only_without_override_and_with_exact_identity() {
    let (_directory, catalog, exact) = disabled_complete_catalog();
    let resumed = resolve_native_resume(
        &catalog,
        DurableNativeBinding::Complete(exact.clone()),
        None,
        crate::execution::tools::LocalToolRegistryRevision::BasicFiles,
    )
    .unwrap();
    assert!(!resumed.replaces_binding());

    let explicit = resolve_native_resume(
        &catalog,
        DurableNativeBinding::Complete(exact),
        Some("model"),
        crate::execution::tools::LocalToolRegistryRevision::BasicFiles,
    )
    .unwrap_err();
    assert!(explicit.to_string().contains("disabled by operator"));

    let changed = resolve_native_resume(
        &catalog,
        DurableNativeBinding::Complete(complete_binding("high")),
        None,
        crate::execution::tools::LocalToolRegistryRevision::BasicFiles,
    )
    .unwrap_err();
    assert!(changed.to_string().contains("disabled by operator"));
}

// complete schema는 full-binding attribution이므로 알 수 없는 필드를 legacy v1처럼
// 무시하지 않고 거절해 새 profile 의미가 소실되는 것을 막습니다.
#[test]
fn complete_durable_binding_rejects_unknown_fields() {
    let error = parse_durable_binding(
        "yo.complete-model-binding/v1",
        r#"{"provider":"qwencloud","account":"default","model":"model","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1","unknown":true}"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("malformed"));
}

// 새 no-tools binding은 empty registry revision을 startup state에 남겨 이후 model
// replacement가 그 Session을 basic tools로 조용히 upgrade하지 않습니다.
#[test]
fn new_no_tools_session_freezes_the_empty_registry_revision() {
    let complete = CompleteModelBinding::from_durable_json(
        r#"{"provider":"qwencloud","account":"default","model":"model","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{},"optional_request_parameters":{},"tool_capability_policy":"no-tools/v1"}"#,
    )
    .unwrap();
    let catalog = yo_core::ModelCatalog::new(vec![
        yo_core::ModelCatalogEntry::with_explicit_profile(
            complete.binding().clone(),
            None,
            None,
            None,
            complete.profile().clone(),
        )
        .unwrap(),
    ])
    .unwrap();
    let startup = resolve_new_session(&catalog, None, None, Some("model")).unwrap();

    assert_eq!(
        startup.registry_revision(),
        Some(crate::execution::tools::LocalToolRegistryRevision::NoTools)
    );
}

// 명시적 Session restriction은 local-tools/v1 complete binding을 바꾸지 않고 empty
// registry만 선택하며, delegated HostTarget에는 Yo가 강제할 surface가 없어 거절합니다.
#[test]
fn explicit_no_tools_restricts_only_a_new_native_session() {
    let catalog = selection_catalog(&[("qwencloud", "default", "model")]);
    let startup =
        resolve_new_session_with_tool_restriction(&catalog, None, None, Some("model"), true, false)
            .unwrap();
    assert_eq!(
        startup.registry_revision(),
        Some(crate::execution::tools::LocalToolRegistryRevision::NoTools)
    );
    assert_eq!(startup.model_selection().unwrap().model().as_str(), "model");

    let error = resolve_new_session_with_tool_restriction(
        &catalog,
        None,
        None,
        Some("host:codex"),
        true,
        false,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("--no-tools"));
    assert!(error.contains(SESSION_TOOL_EXPOSURE_PROFILE));
    assert!(error.contains("owns its tool surface"));
}

// durable JSON 숫자는 serde가 큰 정수를 float로 바꾸기 전에 spelling대로 범위를
// 검사하고, 유한 exponent와 따옴표로 명시한 숫자 모양 문자열은 구분합니다.
#[test]
fn complete_durable_binding_rejects_out_of_range_number_spellings() {
    let prefix = r#"{"provider":"qwencloud","account":"default","model":"model","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{"value":"#;
    let suffix = r#"},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1"}"#;
    for invalid in [
        "18446744073709551616",
        "340282366920938463463374607431768211456",
        "1e400",
    ] {
        let error = parse_durable_binding(
            "yo.complete-model-binding/v1",
            &format!("{prefix}{invalid}{suffix}"),
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("out-of-range number"),
            "{invalid}: {error}"
        );
    }

    let exponent = parse_durable_binding(
        "yo.complete-model-binding/v1",
        &format!("{prefix}1e2{suffix}"),
    )
    .unwrap();
    let decimal = parse_durable_binding(
        "yo.complete-model-binding/v1",
        &format!("{prefix}100.0{suffix}"),
    )
    .unwrap();
    assert_eq!(exponent, decimal);
    parse_durable_binding(
        "yo.complete-model-binding/v1",
        &format!(r#"{prefix}"1e400"{suffix}"#),
    )
    .unwrap();
}
