use serde::Deserialize;
use yo_core::{
    AccountId, ApiDialect, BackendResumeTarget, CompleteModelBinding, ConnectorId, ModelId,
    ModelSelection, ModelSelectionController, NormalizedEndpoint, ProviderId, StartupPolicy,
    StartupSelectionSources, StartupTarget, resolve_startup_target,
};

use super::StartupBackend;
use crate::{AppError, config::Config};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DurableBackendKind {
    Codex,
    Native,
}

pub(super) fn replacement(selection: &yo_core::ModelSelection) -> StartupBackend {
    StartupBackend::Native {
        provider: selection.provider().clone(),
        account: selection.account().clone(),
        model: selection.model().clone(),
        replace_binding: true,
    }
}

pub(super) fn resolve(
    config: &Config,
    stored_preference: Option<StartupTarget>,
    override_model: Option<&str>,
    resume: Option<&BackendResumeTarget>,
) -> Result<StartupBackend, AppError> {
    if let Some(target) = resume {
        return resolve_resume(config, override_model, target);
    }
    let startup = config.startup_target().cloned();
    resolve_new_session(
        config.model_catalog(),
        stored_preference,
        startup,
        override_model,
    )
}

fn resolve_new_session(
    catalog: &yo_core::ModelCatalog,
    stored_preference: Option<StartupTarget>,
    operator: Option<StartupTarget>,
    reference: Option<&str>,
) -> Result<StartupBackend, AppError> {
    let target = resolve_startup_target(
        catalog,
        &StartupPolicy::initial(),
        StartupSelectionSources {
            invocation: reference,
            stored_preference,
            operator_target: operator,
        },
    )
    .map_err(|error| AppError::single("resolving startup target", error))?;
    let Some(target) = target else {
        return Err(AppError::message("no startup target is selected")
            .with_help(["yo connect", "yo --model host:codex"]));
    };
    match target {
        StartupTarget::HostCodex => Ok(StartupBackend::Codex),
        StartupTarget::Model(selection) => Ok(native_selection(selection, false)),
    }
}

fn resolve_resume(
    config: &Config,
    override_model: Option<&str>,
    target: &BackendResumeTarget,
) -> Result<StartupBackend, AppError> {
    match classify_durable_backend(target.binding().backend_kind())? {
        DurableBackendKind::Codex => return resolve_codex_resume(override_model),
        DurableBackendKind::Native => {},
    }
    let binding_identity = target.binding().binding_identity();
    let durable_binding =
        parse_durable_binding(binding_identity.schema(), binding_identity.value())?;
    resolve_native_resume(config.model_catalog(), durable_binding, override_model)
}

fn classify_durable_backend(kind: &str) -> Result<DurableBackendKind, AppError> {
    match kind {
        "codex-app-server" => Ok(DurableBackendKind::Codex),
        "yo-managed-model" => Ok(DurableBackendKind::Native),
        other => Err(AppError::many([format!(
            "unsupported durable backend kind {other:?}; the saved Session can only be opened read-only"
        )])),
    }
}

fn resolve_codex_resume(override_model: Option<&str>) -> Result<StartupBackend, AppError> {
    match override_model {
        None | Some(StartupTarget::HOST_CODEX_REFERENCE) => Ok(StartupBackend::Codex),
        Some(_) => Err(AppError::many([
            "a different model target cannot replace a Codex Session; cross-backend handoff is not supported"
                .to_owned(),
        ])),
    }
}

fn resolve_native_resume(
    catalog: &yo_core::ModelCatalog,
    durable_binding: DurableNativeBinding,
    reference: Option<&str>,
) -> Result<StartupBackend, AppError> {
    let binding = durable_binding.binding();
    let durable_selection = ModelSelection::new(
        binding.provider_id().clone(),
        binding.account_id().clone(),
        binding.model_id().clone(),
    );
    let selection = match reference {
        Some(reference) => {
            match ModelSelectionController::new(catalog.clone(), Some(durable_selection.clone()))
                .resolve_target_reference(reference)
                .map_err(|error| AppError::single("resolving resumed model", error))?
            {
                StartupTarget::HostCodex => {
                    return Err(AppError::many([
                    "Local Codex cannot replace a Yo-managed Session; cross-backend handoff is not supported"
                        .to_owned(),
                ]));
                },
                StartupTarget::Model(selection) => selection,
            }
        },
        None => durable_selection,
    };
    let entry = catalog
        .resolve_model(selection.provider(), selection.account(), selection.model())
        .map_err(|error| AppError::single("resolving resumed model", error))?;
    let replace_binding = !durable_binding.matches(entry);
    Ok(native_selection(selection, replace_binding))
}

fn native_selection(selection: ModelSelection, replace_binding: bool) -> StartupBackend {
    StartupBackend::Native {
        provider: selection.provider().clone(),
        account: selection.account().clone(),
        model: selection.model().clone(),
        replace_binding,
    }
}

fn parse_durable_binding(schema: &str, value: &str) -> Result<DurableNativeBinding, AppError> {
    match schema {
        "yo.model-binding/v1" => {
            let durable: DurableBinding = serde_json::from_str(value).map_err(|_| {
                AppError::many(["the durable native binding identity is malformed".to_owned()])
            })?;
            parse_legacy_binding(durable).map(DurableNativeBinding::Legacy)
        },
        "yo.complete-model-binding/v1" => CompleteModelBinding::from_durable_json(value)
            .map(DurableNativeBinding::Complete)
            .map_err(|error| AppError::single("validating durable complete binding", error)),
        _ => Err(AppError::many([
            "the durable native binding has an unsupported identity schema".to_owned(),
        ])),
    }
}

fn parse_legacy_binding(
    durable: DurableBinding,
) -> Result<yo_core::EffectiveModelBinding, AppError> {
    parse_binding_coordinates(
        durable.provider,
        durable.account,
        durable.model,
        durable.connector,
        durable.api_dialect,
        durable.base_url,
    )
}

fn parse_binding_coordinates(
    provider: String,
    account: String,
    model: String,
    connector: String,
    api_dialect: String,
    base_url: String,
) -> Result<yo_core::EffectiveModelBinding, AppError> {
    let durable_provider = ProviderId::new(provider)
        .map_err(|error| AppError::single("validating durable Provider", error))?;
    let durable_account = AccountId::new(account)
        .map_err(|error| AppError::single("validating durable Account", error))?;
    let durable_model =
        ModelId::new(model).map_err(|error| AppError::single("validating durable Model", error))?;
    let durable_connector = ConnectorId::new(connector)
        .map_err(|error| AppError::single("validating durable connector", error))?;
    let durable_dialect = api_dialect
        .parse::<ApiDialect>()
        .map_err(|error| AppError::single("validating durable API dialect", error))?;
    let durable_endpoint = NormalizedEndpoint::parse(&base_url)
        .map_err(|error| AppError::single("validating durable endpoint", error))?;
    yo_core::EffectiveModelBinding::from_durable(
        durable_provider,
        durable_account,
        durable_model,
        durable_connector,
        durable_dialect,
        durable_endpoint,
    )
    .map_err(|error| AppError::single("validating durable model binding", error))
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum DurableNativeBinding {
    Legacy(yo_core::EffectiveModelBinding),
    Complete(CompleteModelBinding),
}

impl DurableNativeBinding {
    fn binding(&self) -> &yo_core::EffectiveModelBinding {
        match self {
            Self::Legacy(binding) => binding,
            Self::Complete(binding) => binding.binding(),
        }
    }

    fn matches(&self, entry: &yo_core::ModelCatalogEntry) -> bool {
        match self {
            Self::Legacy(binding) => {
                entry.explicit_profile().is_none() && entry.binding() == binding
            },
            Self::Complete(binding) => entry.complete_binding() == Some(binding),
        }
    }
}

#[derive(Deserialize)]
struct DurableBinding {
    provider: String,
    account: String,
    model: String,
    connector: String,
    api_dialect: String,
    base_url: String,
}

#[cfg(test)]
mod tests {
    use yo_core::{EffectiveModelProfile, ModelProfileParameters, VersionedProfileId};

    use super::*;

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
        assert_eq!(missing.help(), ["yo connect", "yo --model host:codex"]);

        assert!(matches!(
            resolve_new_session(&catalog, None, None, Some("host:codex")).unwrap(),
            StartupBackend::Codex
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
            Some(StartupTarget::HostCodex),
            Some(operator.clone()),
            None,
        )
        .unwrap();
        assert!(matches!(stored, StartupBackend::Codex));

        let invoked = resolve_new_session(
            &catalog,
            Some(StartupTarget::HostCodex),
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

        let error =
            match resolve_new_session(&catalog, None, Some(StartupTarget::Model(stale)), None) {
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
            resolve_codex_resume(None).unwrap(),
            StartupBackend::Codex
        ));
        assert!(matches!(
            resolve_codex_resume(Some("host:codex")).unwrap(),
            StartupBackend::Codex
        ));
        assert!(resolve_codex_resume(Some("qwen3.8-max")).is_err());
    }

    // 저장된 backend kind를 추측해 Codex로 보내면 locator identity를 바꾸므로 알려진 두
    // runtime kind만 분류하고 다른 값은 read-only guidance와 함께 fail closed 한다.
    #[test]
    fn durable_backend_classification_rejects_unknown_kinds() {
        assert_eq!(
            classify_durable_backend("codex-app-server").unwrap(),
            DurableBackendKind::Codex
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
        )
        .unwrap();
        assert!(!same.replaces_binding());
        assert_eq!(
            same.model_selection().unwrap().provider().as_str(),
            "qwencloud"
        );

        let replacement = resolve_native_resume(
            &catalog,
            DurableNativeBinding::Legacy(durable),
            Some("openrouter::same"),
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

    // 새 complete binding은 endpoint·connector와 여덟 profile 필드를 모두 복원하고,
    // structured integer/float 구분을 포함한 exact profile이 같을 때만 같은 epoch입니다.
    #[test]
    fn complete_durable_binding_preserves_every_resolved_profile_field() {
        let durable = parse_durable_binding(
            "yo.complete-model-binding/v1",
            r#"{"provider":"qwencloud","account":"token-plan","model":"qwen3.8max","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000000,"max_output_tokens":65536,"reasoning_parameters":{"effort":"medium","integer":1,"float":1.0},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1","verification_profile":"semantic-terminal/v1"}"#,
        )
        .unwrap();
        let DurableNativeBinding::Complete(complete) = durable else {
            panic!("complete schema must produce a complete binding");
        };
        let binding = complete.binding();
        let profile = complete.profile();

        assert_eq!(binding.endpoint().as_str(), "https://example.test/v1");
        assert_eq!(profile.context().input_token_limit(), 1_000_000);
        assert_eq!(profile.context().max_output_tokens(), 65_536);
        assert_eq!(profile.context().tokenizer_profile(), "utf8-bytes/v1");
        assert_eq!(
            profile.reasoning_parameters(),
            &serde_json::from_str::<ModelProfileParameters>(
                r#"{"effort":"medium","integer":1,"float":1.0}"#
            )
            .unwrap()
        );
        assert_eq!(profile.tool_capability_policy().as_str(), "local-tools/v1");
        assert_eq!(
            profile.verification_profile().as_str(),
            "semantic-terminal/v1"
        );
    }

    // resume은 좌표와 endpoint가 같아도 현재 resolved profile이 달라지면 replacement를
    // 요구하고, exact complete binding일 때만 기존 epoch를 그대로 재사용합니다.
    #[test]
    fn native_resume_compares_the_complete_explicit_profile() {
        let durable = parse_durable_binding(
            "yo.complete-model-binding/v1",
            r#"{"provider":"qwencloud","account":"default","model":"model","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{"effort":"medium"},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1","verification_profile":"semantic-terminal/v1"}"#,
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
            !resolve_native_resume(&exact_catalog, durable.clone(), None)
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
            Some(VersionedProfileId::new("semantic-terminal/v1").unwrap()),
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
            resolve_native_resume(&changed_catalog, durable, None)
                .unwrap()
                .replaces_binding()
        );
    }

    // complete schema는 full-binding attribution이므로 알 수 없는 필드를 legacy v1처럼
    // 무시하지 않고 거절해 새 profile 의미가 소실되는 것을 막습니다.
    #[test]
    fn complete_durable_binding_rejects_unknown_fields() {
        let error = parse_durable_binding(
            "yo.complete-model-binding/v1",
            r#"{"provider":"qwencloud","account":"default","model":"model","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1","verification_profile":"semantic-terminal/v1","unknown":true}"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("malformed"));
    }

    // durable JSON 숫자는 serde가 큰 정수를 float로 바꾸기 전에 spelling대로 범위를
    // 검사하고, 유한 exponent와 따옴표로 명시한 숫자 모양 문자열은 구분합니다.
    #[test]
    fn complete_durable_binding_rejects_out_of_range_number_spellings() {
        let prefix = r#"{"provider":"qwencloud","account":"default","model":"model","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{"value":"#;
        let suffix = r#"},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1","verification_profile":"semantic-terminal/v1"}"#;
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
}
