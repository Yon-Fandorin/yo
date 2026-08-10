//! Startup model binding resolution and host-owned native backend services.

use std::path::Path;

use yo_core::{AccountId, AgentBackend, BackendResumeTarget, CredentialStore, ModelId, ProviderId};
#[cfg(test)]
use yo_core::{ApiDialect, ModelSelection, ModelTokenCounter, NormalizedEndpoint, StartupTarget};

use crate::{AppError, config::Config};

mod native;
mod startup;
mod tokenizer;

#[cfg(test)]
use startup::{
    DurableBackendKind, classify_durable_backend, parse_durable_binding, resolve_codex_resume,
    resolve_native_resume, resolve_new_session,
};
#[cfg(test)]
use tokenizer::{
    O200K_PROFILE, TokenizerRegistry, UTF8_BYTES_PROFILE, require_supported_tokenizer,
};

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
    startup::replacement(selection)
}

pub(crate) fn resolve(
    config: &Config,
    override_model: Option<&str>,
    resume: Option<&BackendResumeTarget>,
) -> Result<StartupBackend, AppError> {
    startup::resolve(config, override_model, resume)
}

pub(crate) fn start_native(
    config: &Config,
    credentials: &CredentialStore,
    selection: &StartupBackend,
    workspace: &Path,
) -> Result<Box<dyn AgentBackend + Send>, AppError> {
    native::start_native(config, credentials, selection, workspace)
}

pub(crate) fn open_credentials(path: &Path) -> Result<CredentialStore, AppError> {
    native::open_credentials(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection_catalog(entries: &[(&str, &str, &str)]) -> yo_core::ModelCatalog {
        selection_catalog_with_tokenizer(entries, UTF8_BYTES_PROFILE)
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

    // backend facade는 host와 native selection의 label·좌표·replacement flag를 각각
    // 보존하고, replacement helper도 같은 좌표를 durable binding 교체로 표시한다.
    #[test]
    fn startup_backend_metadata_preserves_host_and_native_selection_semantics() {
        let selection = ModelSelection::new(
            ProviderId::new("qwencloud").unwrap(),
            AccountId::new("token-plan").unwrap(),
            ModelId::new("qwen3.8max").unwrap(),
        );

        let codex = StartupBackend::Codex;
        assert_eq!(codex.label(), "codex");
        assert!(!codex.replaces_binding());
        assert!(codex.model_selection().is_none());

        let native = StartupBackend::Native {
            provider: selection.provider().clone(),
            account: selection.account().clone(),
            model: selection.model().clone(),
            replace_binding: false,
        };
        assert_eq!(native.label(), "qwen3.8max");
        assert!(!native.replaces_binding());
        assert_eq!(native.model_selection(), Some(selection.clone()));

        let replacement = replacement(&selection);
        assert_eq!(replacement.label(), "qwen3.8max");
        assert!(replacement.replaces_binding());
        assert_eq!(replacement.model_selection(), Some(selection));
    }

    // native startup은 Codex 선택을 catalog 해석이나 credential 조회로 보내지 않고,
    // backend 종류가 잘못된 호출이라는 고정 진단으로 즉시 거절한다.
    #[test]
    fn native_startup_rejects_host_backend_before_catalog_resolution() {
        let error = match start_native(
            &Config::default(),
            &yo_core::CredentialStore::default(),
            &StartupBackend::Codex,
            std::path::Path::new("."),
        ) {
            Ok(_) => panic!("host backend must be rejected before native startup"),
            Err(error) => error,
        };

        assert_eq!(
            error.to_string(),
            "native backend startup requires a native model selection"
        );
    }

    // startup source가 모두 비면 setup guidance로 실패하고, Local Codex도 host target으로
    // 명시해야 한다. unique bare ModelId와 완전한 좌표는 operator startup 없이 선택한다.
    #[test]
    fn new_session_requires_a_target_and_accepts_host_unique_or_complete_references() {
        let catalog = selection_catalog(&[
            ("qwencloud", "default", "qwen3.8-max"),
            ("openrouter", "default", "openrouter/free"),
        ]);

        let missing = resolve_new_session(&catalog, None, None).unwrap_err();
        assert_eq!(missing.to_string(), "no startup target is selected");
        assert_eq!(missing.help(), ["yo connect", "yo --model host:codex"]);

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

    // catalog entry가 지원하지 않는 profile을 선언하면 실제 profile과 현재 build의
    // 전체 allowlist를 함께 노출하는 exact diagnostic으로 fail closed 한다.
    #[test]
    fn unsupported_tokenizer_profile_reports_exact_profile_and_allowlist() {
        let catalog = selection_catalog_with_tokenizer(
            &[("qwencloud", "default", "qwen3.8-max")],
            "qwen/latest",
        );

        let error = require_supported_tokenizer(&catalog.entries()[0]).unwrap_err();

        assert_eq!(
            error.to_string(),
            "unsupported tokenizer profile \"qwen/latest\"; this build supports o200k_base/v1 and utf8-bytes/v1"
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
