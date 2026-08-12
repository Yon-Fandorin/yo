use serde::Deserialize;
use yo_core::{
    AccountId, ApiDialect, BackendResumeTarget, ConnectorId, ModelId, ModelSelection,
    ModelSelectionController, NormalizedEndpoint, ProviderId, StartupPolicy,
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
    if binding_identity.schema() != "yo.model-binding/v1" {
        return Err(AppError::many([
            "the durable native binding has an unsupported identity schema".to_owned(),
        ]));
    }
    let durable_binding = parse_durable_binding(binding_identity.value())?;
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
    durable_binding: yo_core::EffectiveModelBinding,
    reference: Option<&str>,
) -> Result<StartupBackend, AppError> {
    let durable_selection = ModelSelection::new(
        durable_binding.provider_id().clone(),
        durable_binding.account_id().clone(),
        durable_binding.model_id().clone(),
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
    let replace_binding = entry.binding() != &durable_binding;
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

fn parse_durable_binding(value: &str) -> Result<yo_core::EffectiveModelBinding, AppError> {
    let durable: DurableBinding = serde_json::from_str(value).map_err(|_| {
        AppError::many(["the durable native binding identity is malformed".to_owned()])
    })?;
    let durable_provider = ProviderId::new(durable.provider)
        .map_err(|error| AppError::single("validating durable Provider", error))?;
    let durable_account = AccountId::new(durable.account)
        .map_err(|error| AppError::single("validating durable Account", error))?;
    let durable_model = ModelId::new(durable.model)
        .map_err(|error| AppError::single("validating durable Model", error))?;
    let durable_connector = ConnectorId::new(durable.connector)
        .map_err(|error| AppError::single("validating durable connector", error))?;
    let durable_dialect = durable
        .api_dialect
        .parse::<ApiDialect>()
        .map_err(|error| AppError::single("validating durable API dialect", error))?;
    let durable_endpoint = NormalizedEndpoint::parse(&durable.base_url)
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

        let same = resolve_native_resume(&catalog, durable.clone(), Some("same")).unwrap();
        assert!(!same.replaces_binding());
        assert_eq!(
            same.model_selection().unwrap().provider().as_str(),
            "qwencloud"
        );

        let replacement =
            resolve_native_resume(&catalog, durable, Some("openrouter::same")).unwrap();
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
        let binding = parse_durable_binding(
            r#"{"provider":"qwencloud","account":"token-plan","model":"qwen3.8max","connector":"openai-responses","api_dialect":"openai-responses","base_url":"https://old.example/v1"}"#,
        )
        .unwrap();
        let changed = yo_core::EffectiveModelBinding::new(
            binding.provider_id().clone(),
            binding.account_id().clone(),
            binding.model_id().clone(),
            ApiDialect::OpenAiResponses,
            NormalizedEndpoint::parse("https://new.example/v1").unwrap(),
        );

        assert_ne!(binding, changed);
        assert_eq!(binding.endpoint().as_str(), "https://old.example/v1");
    }

    // durable identity parser는 v1에서 알 수 없는 JSON 필드를 현재 무시하므로, 같은
    // canonical binding을 extra field 없이 읽거나 붙여 읽어도 complete identity가 같다.
    #[test]
    fn durable_binding_parser_ignores_unknown_json_fields() {
        let canonical = r#"{"provider":"qwencloud","account":"token-plan","model":"qwen3.8max","connector":"openai-responses","api_dialect":"openai-responses","base_url":"https://old.example/v1"}"#;
        let with_unknown_field = r#"{"provider":"qwencloud","account":"token-plan","model":"qwen3.8max","connector":"openai-responses","api_dialect":"openai-responses","base_url":"https://old.example/v1","unknown_field":"ignored"}"#;

        let binding = parse_durable_binding(canonical).unwrap();
        let binding_with_unknown_field = parse_durable_binding(with_unknown_field).unwrap();

        assert_eq!(binding_with_unknown_field, binding);
    }
}
