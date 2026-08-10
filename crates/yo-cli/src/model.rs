//! Startup model binding resolution and host-owned native backend services.

use std::path::Path;

use serde::Deserialize;
use yo_core::{
    AccountId, AgentBackend, ApiDialect, BackendResumeTarget, ConnectorId, CredentialStore,
    LocalCredentialStore, ModelCatalogEntry, ModelConnectorLimits, ModelId, ModelSelection,
    ModelSelectionController, ModelTokenCounter, ModelTokenCounterError, NativeModelBackend,
    NativeModelBackendConfig, NativeModelBackendServices, NormalizedEndpoint, ProviderId,
    StartupPolicy, StartupSelectionSources, StartupTarget, resolve_startup_target,
};

use crate::{AppError, config::Config, local_tools};

const O200K_PROFILE: &str = "o200k_base/v1";
const UTF8_BYTES_PROFILE: &str = "utf8-bytes/v1";

#[derive(Clone, Debug)]
pub(crate) enum StartupBackend {
    Codex,
    Native {
        provider: ProviderId,
        account: AccountId,
        model: ModelId,
        replace_binding: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DurableBackendKind {
    Codex,
    Native,
}

impl StartupBackend {
    pub(crate) fn label(&self) -> &str {
        match self {
            Self::Codex => "codex",
            Self::Native { model, .. } => model.as_str(),
        }
    }

    pub(crate) const fn replaces_binding(&self) -> bool {
        matches!(
            self,
            Self::Native {
                replace_binding: true,
                ..
            }
        )
    }

    pub(crate) fn model_selection(&self) -> Option<yo_core::ModelSelection> {
        match self {
            Self::Codex => None,
            Self::Native {
                provider,
                account,
                model,
                ..
            } => Some(yo_core::ModelSelection::new(
                provider.clone(),
                account.clone(),
                model.clone(),
            )),
        }
    }
}

pub(crate) fn replacement(selection: &yo_core::ModelSelection) -> StartupBackend {
    StartupBackend::Native {
        provider: selection.provider().clone(),
        account: selection.account().clone(),
        model: selection.model().clone(),
        replace_binding: true,
    }
}

pub(crate) fn resolve(
    config: &Config,
    override_model: Option<&str>,
    resume: Option<&BackendResumeTarget>,
) -> Result<StartupBackend, AppError> {
    if let Some(target) = resume {
        return resolve_resume(config, override_model, target);
    }
    let startup = config.startup_target().cloned();
    resolve_new_session(config.model_catalog(), startup, override_model)
}

fn resolve_new_session(
    catalog: &yo_core::ModelCatalog,
    operator: Option<StartupTarget>,
    reference: Option<&str>,
) -> Result<StartupBackend, AppError> {
    let target = resolve_startup_target(
        catalog,
        &StartupPolicy::initial(),
        StartupSelectionSources {
            invocation: reference,
            stored_preference: None,
            operator_target: operator,
        },
    )
    .map_err(|error| AppError::single("resolving startup target", error))?;
    let Some(target) = target else {
        return Err(AppError::many([
            "no startup target is selected; run `yo connect` to configure one, or start Local Codex explicitly with `yo --model host:codex`"
                .to_owned(),
        ]));
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

pub(crate) fn start_native(
    config: &Config,
    credentials: &CredentialStore,
    selection: &StartupBackend,
    workspace: &Path,
) -> Result<Box<dyn AgentBackend + Send>, AppError> {
    let StartupBackend::Native {
        provider,
        account,
        model,
        ..
    } = selection
    else {
        return Err(AppError::many([
            "native backend startup requires a native model selection".to_owned(),
        ]));
    };
    let entry = config
        .model_catalog()
        .resolve_model(provider, account, model)
        .map_err(|error| AppError::single("resolving native model binding", error))?;
    require_supported_tokenizer(entry)?;
    let credential_path = config.credential_path();
    let credential = credentials.resolve(provider, account).cloned().ok_or_else(|| {
        AppError::many([format!(
            "credentials.yaml has no API credential for Provider {provider} and Account {account}"
        )])
    })?;
    let registry = local_tools::registry()
        .map_err(|error| AppError::single("building the local tool registry", error))?
        .freeze();
    let semantic_admission = local_tools::LocalSemanticAdmission::new(credentials.clone());
    let tool_host = local_tools::LocalToolHost::new(workspace, &credential_path)
        .map_err(|error| AppError::single("starting local workspace tools", error))?;
    let services = NativeModelBackendServices::new(
        Some(Box::new(semantic_admission)),
        Box::new(tool_host),
        Box::new(TokenizerRegistry),
    );
    NativeModelBackend::new(
        entry,
        credential,
        ModelConnectorLimits::default(),
        registry,
        services,
        NativeModelBackendConfig::default(),
    )
    .map(|backend| Box::new(backend) as Box<dyn AgentBackend + Send>)
    .map_err(|error| AppError::single("starting native model backend", error))
}

pub(crate) fn open_credentials(path: &Path) -> Result<CredentialStore, AppError> {
    LocalCredentialStore::open(path)
        .map_err(|error| AppError::single("reading model credentials", error))
}

fn require_supported_tokenizer(entry: &ModelCatalogEntry) -> Result<(), AppError> {
    if TokenizerRegistry::supports(entry.context().tokenizer_profile()) {
        Ok(())
    } else {
        Err(AppError::many([format!(
            "unsupported tokenizer profile {:?}; this build supports {O200K_PROFILE} and {UTF8_BYTES_PROFILE}",
            entry.context().tokenizer_profile()
        )]))
    }
}

struct TokenizerRegistry;

impl TokenizerRegistry {
    fn supports(profile: &str) -> bool {
        matches!(profile, O200K_PROFILE | UTF8_BYTES_PROFILE)
    }
}

impl ModelTokenCounter for TokenizerRegistry {
    fn count_input_tokens(
        &self,
        tokenizer_profile: &str,
        request: &serde_json::Value,
    ) -> Result<u64, ModelTokenCounterError> {
        let encoded = serde_json::to_string(request)
            .map_err(|_| ModelTokenCounterError::new("request cannot be tokenized"))?;
        let count = match tokenizer_profile {
            O200K_PROFILE => tiktoken_rs::o200k_base_singleton()
                .encode_with_special_tokens(&encoded)
                .len(),
            // This profile deliberately admits one token per serialized UTF-8 byte. It is a
            // conservative, provider-neutral bound for byte-backed tokenizer families when an
            // exact built-in tokenizer is unavailable; the profile name makes that policy
            // explicit rather than claiming Qwen or another model's private tokenizer.
            UTF8_BYTES_PROFILE => encoded.len(),
            _ => return Err(ModelTokenCounterError::new("unsupported tokenizer profile")),
        };
        u64::try_from(count).map_err(|_| ModelTokenCounterError::new("token count exceeds u64"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection_catalog(entries: &[(&str, &str, &str)]) -> yo_core::ModelCatalog {
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
                        yo_core::ModelContextProfile::new(1_000, 100, UTF8_BYTES_PROFILE).unwrap(),
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

        let missing = resolve_new_session(&catalog, None, None)
            .unwrap_err()
            .to_string();
        assert!(missing.contains("no startup target is selected"));
        assert!(missing.contains("yo connect"));
        assert!(missing.contains("yo --model host:codex"));

        assert!(matches!(
            resolve_new_session(&catalog, None, Some("host:codex")).unwrap(),
            StartupBackend::Codex
        ));
        let bare = resolve_new_session(&catalog, None, Some("qwen3.8-max")).unwrap();
        let selected = bare.model_selection().unwrap();
        assert_eq!(selected.provider().as_str(), "qwencloud");
        assert_eq!(selected.account().as_str(), "default");
        assert_eq!(selected.model().as_str(), "qwen3.8-max");

        let complete =
            resolve_new_session(&catalog, None, Some("openrouter:default:openrouter/free"))
                .unwrap();
        assert_eq!(
            complete.model_selection().unwrap().model().as_str(),
            "openrouter/free"
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
            Some(StartupTarget::Model(startup)),
            Some("openrouter::same"),
        )
        .unwrap();
        assert_eq!(
            qualified.model_selection().unwrap().provider().as_str(),
            "openrouter"
        );

        let error = match resolve_new_session(&catalog, None, Some("same")) {
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

        let error = match resolve_new_session(&catalog, Some(StartupTarget::Model(stale)), None) {
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

    // tokenizer profile은 versioned allowlist로 해석하며 알 수 없는 이름은 추측하지 않는다.
    #[test]
    fn tokenizer_registry_is_versioned_and_fails_closed() {
        let payload = serde_json::json!({"input": "안녕", "tools": []});
        let registry = TokenizerRegistry;

        assert!(TokenizerRegistry::supports(O200K_PROFILE));
        assert!(TokenizerRegistry::supports(UTF8_BYTES_PROFILE));
        assert!(!TokenizerRegistry::supports("qwen/latest"));
        assert_eq!(
            registry
                .count_input_tokens(UTF8_BYTES_PROFILE, &payload)
                .unwrap(),
            serde_json::to_string(&payload).unwrap().len() as u64
        );
        assert!(
            registry
                .count_input_tokens("qwen/latest", &payload)
                .is_err()
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
}
