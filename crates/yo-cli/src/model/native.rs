use std::path::Path;

use yo_connector_openai_responses::OpenAiResponsesConnector;
use yo_core::{
    AgentBackend, ApiCredential, ApiDialect, ConnectorId, CredentialSnapshot,
    KimiChatCompletionsConnector, LocalConnectionOperationRepositories, LocalCredentialRepository,
    LocalModelRequestObservation, ModelConnector, ModelConnectorLimits, ModelRequestFailureKind,
    ModelRequestOutcome, NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices,
    OpenAiChatCompletionsConnector, ToolRegistry,
};

use super::{
    StartupBackend,
    tokenizer::{TokenizerRegistry, require_supported_tokenizer},
};
use crate::{AppError, config::Config, local_tools};

pub(super) fn start_native(
    config: &Config,
    credentials: &CredentialSnapshot,
    selection: &StartupBackend,
    workspace: &Path,
) -> Result<Box<dyn AgentBackend + Send>, AppError> {
    let StartupBackend::Native {
        provider,
        account,
        model,
        registry_revision,
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
    let observation = entry.complete_binding().cloned().map(|complete| {
        let directory = config
            .connection_path()
            .parent()
            .expect("the connection path always has a parent")
            .to_owned();
        LocalConnectionOperationRepositories::in_directory(directory)
            .map(|repositories| {
                LocalModelRequestObservation::new(
                    repositories,
                    complete,
                    credentials.revision().clone(),
                )
            })
            .map_err(|error| error.to_string())
    });
    require_supported_tokenizer(entry)
        .map_err(|error| with_local_configuration_observation(error, observation.as_ref()))?;
    let credential_path = config.credential_path();
    let credential = match credentials.resolve(provider, account).cloned() {
        Some(credential) => credential,
        None => {
            let error = AppError::many([format!(
                "credentials.yaml has no API credential for Provider {provider} and Account {account}"
            )]);
            return Err(with_local_configuration_observation(
                error,
                observation.as_ref(),
            ));
        },
    };
    let registry = runtime_registry(entry, *registry_revision)
        .map_err(|error| with_local_configuration_observation(error, observation.as_ref()))?;
    let semantic_admission =
        local_tools::LocalSemanticAdmission::new(credentials.credentials().clone());
    let tool_host = local_tools::LocalToolHost::new(workspace, &credential_path)
        .map_err(|error| AppError::single("starting local workspace tools", error))?;
    let mut services = NativeModelBackendServices::new(
        Some(Box::new(semantic_admission)),
        Box::new(tool_host),
        Box::new(TokenizerRegistry),
    );
    if let Some(request_observation) = observation.clone() {
        services = services.with_model_request_observer(move |outcome| {
            request_observation
                .as_ref()
                .map_err(Clone::clone)?
                .record(outcome)
                .map(|_| ())
                .map_err(|error| error.to_string())
        });
    }
    let backend_config = NativeModelBackendConfig {
        maximum_tool_argument_bytes: registry_revision.maximum_argument_bytes(),
        ..NativeModelBackendConfig::default()
    };
    let connector = native_connector(entry, credential, ModelConnectorLimits::default())
        .map_err(|error| with_local_configuration_observation(error, observation.as_ref()))?;
    let backend = NativeModelBackend::new(entry, connector, registry, services, backend_config)
        .map_err(|error| AppError::single("starting native model backend", error));
    backend
        .map(|backend| Box::new(backend) as Box<dyn AgentBackend + Send>)
        .map_err(|error| with_local_configuration_observation(error, observation.as_ref()))
}

fn native_connector(
    entry: &yo_core::ModelCatalogEntry,
    credential: ApiCredential,
    limits: ModelConnectorLimits,
) -> Result<Box<dyn ModelConnector>, AppError> {
    let binding = entry.binding();
    let connector = match (binding.connector_id().as_str(), binding.api_dialect()) {
        (ConnectorId::OPENAI_RESPONSES, ApiDialect::OpenAiResponses) => {
            OpenAiResponsesConnector::new(binding, credential, limits)
                .map(|connector| Box::new(connector) as Box<dyn ModelConnector>)
        },
        (ConnectorId::OPENAI_CHAT_COMPLETIONS, ApiDialect::OpenAiChatCompletions) => {
            OpenAiChatCompletionsConnector::new(binding, credential, limits)
                .map(|connector| Box::new(connector) as Box<dyn ModelConnector>)
        },
        (ConnectorId::KIMI_CHAT_COMPLETIONS, ApiDialect::KimiChatCompletions) => {
            let complete = entry.complete_binding().ok_or_else(|| {
                AppError::many(["Kimi connector requires a complete explicit profile".to_owned()])
            })?;
            KimiChatCompletionsConnector::new(complete, credential, limits)
                .map(|connector| Box::new(connector) as Box<dyn ModelConnector>)
        },
        _ => {
            return Err(AppError::many([format!(
                "unsupported Connector identity {} for API dialect {}",
                binding.connector_id(),
                binding.api_dialect().as_str()
            )]));
        },
    };
    connector.map_err(|error| AppError::single("constructing the selected model connector", error))
}

fn with_local_configuration_observation(
    primary: AppError,
    observation: Option<&Result<LocalModelRequestObservation, String>>,
) -> AppError {
    let persistence_error = match observation {
        Some(Ok(observation)) => observation
            .record(ModelRequestOutcome::Failed(
                ModelRequestFailureKind::LocalConfiguration,
            ))
            .err()
            .map(|error| error.to_string()),
        Some(Err(error)) => Some(error.clone()),
        None => None,
    };
    match persistence_error {
        Some(error) => AppError::combine([
            primary,
            AppError::single("recording the local model configuration failure", error),
        ]),
        None => primary,
    }
}

fn runtime_registry(
    entry: &yo_core::ModelCatalogEntry,
    revision: local_tools::LocalToolRegistryRevision,
) -> Result<yo_core::FrozenToolRegistry, AppError> {
    if entry
        .explicit_profile()
        .is_some_and(|profile| profile.tool_capability_policy().as_str() == "no-tools/v1")
    {
        Ok(ToolRegistry::default().freeze())
    } else {
        let registry = local_tools::registry(revision)
            .map_err(|error| AppError::single("building the local tool registry", error))?
            .freeze();
        Ok(registry)
    }
}

pub(super) fn open_credentials(path: &Path) -> Result<CredentialSnapshot, AppError> {
    LocalCredentialRepository::new(path.to_owned())
        .capture()
        .map_err(|error| AppError::single("reading model credentials", error))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_complete() -> yo_core::CompleteModelBinding {
        yo_core::CompleteModelBinding::from_durable_json(
            r#"{"provider":"qwencloud","account":"default","model":"model","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1"}"#,
        )
        .unwrap()
    }

    fn explicit_entry(policy: &str) -> yo_core::ModelCatalogEntry {
        let complete = yo_core::CompleteModelBinding::from_durable_json(&format!(
            r#"{{"provider":"qwencloud","account":"default","model":"model","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{{}},"optional_request_parameters":{{}},"tool_capability_policy":"{policy}"}}"#
        ))
        .unwrap();
        yo_core::ModelCatalogEntry::with_explicit_profile(
            complete.binding().clone(),
            None,
            None,
            None,
            complete.profile().clone(),
        )
        .unwrap()
    }

    // CLI composition root는 이미 확정된 Responses identity와 dialect만 외부 Connector
    // crate에 연결하고 base URL에 정확한 responses endpoint를 구성합니다.
    #[test]
    fn composes_the_external_responses_connector_for_the_exact_binding() {
        let connector = native_connector(
            &explicit_entry("no-tools/v1"),
            ApiCredential::new("secret").unwrap(),
            ModelConnectorLimits::default(),
        )
        .unwrap();

        assert_eq!(connector.request_url(), "https://example.test/v1/responses");
    }

    // CLI startup의 authoritative registry handoff가 durable no-tools policy를 실제 empty
    // registry로 바꾸고 local-tools policy에는 현재 registry를 유지하는지 관찰합니다.
    #[test]
    fn startup_registry_matches_the_resolved_tool_policy() {
        assert!(
            runtime_registry(
                &explicit_entry("no-tools/v1"),
                local_tools::LocalToolRegistryRevision::BasicFiles,
            )
            .unwrap()
            .is_empty()
        );
        assert!(
            runtime_registry(
                &explicit_entry("local-tools/v1"),
                local_tools::LocalToolRegistryRevision::NoTools,
            )
            .unwrap()
            .is_empty()
        );
        assert!(
            !runtime_registry(
                &explicit_entry("local-tools/v1"),
                local_tools::LocalToolRegistryRevision::BasicFiles,
            )
            .unwrap()
            .is_empty()
        );
    }

    // native startup은 Codex 선택을 catalog 해석이나 credential 조회로 보내지 않고,
    // backend 종류가 잘못된 호출이라는 고정 진단으로 즉시 거절한다.
    #[test]
    fn native_startup_rejects_host_backend_before_catalog_resolution() {
        let credentials = LocalCredentialRepository::new(std::env::temp_dir().join(format!(
            "yo-native-wrong-backend-{}-missing.yaml",
            std::process::id()
        )))
        .capture()
        .unwrap();
        let error = match start_native(
            &Config::default(),
            &credentials,
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

    // 실제 native startup에서 credential이 없으면 remote connector를 만들기 전에 원래
    // startup 오류를 유지하면서 exact stored model에 local_configuration warning을 남깁니다.
    #[test]
    fn missing_startup_credential_records_local_configuration_failure() {
        let root = std::env::temp_dir().join(format!(
            "yo-native-missing-credential-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config_path = root.join("config.yaml");
        let mut config = crate::config::load_from(&config_path).unwrap();
        let complete = fixture_complete();
        let account = yo_core::ConnectionAccount::new(
            complete.binding().provider_id().clone(),
            complete.binding().account_id().clone(),
            None,
            None,
        )
        .unwrap();
        let stored = yo_core::StoredModelBinding::new(complete.clone(), None).unwrap();
        let repository = yo_core::LocalConnectionRepository::new(root.join("connections.yaml"));
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_model_upsert(account, stored)
            .unwrap()
            .unwrap();
        repository.commit(&mutation).unwrap();
        config.replace_model_catalog(repository.capture().unwrap().model_catalog().unwrap());
        let credentials = LocalCredentialRepository::new(root.join("credentials.yaml"))
            .capture()
            .unwrap();
        let selection = StartupBackend::Native {
            provider: complete.binding().provider_id().clone(),
            account: complete.binding().account_id().clone(),
            model: complete.binding().model_id().clone(),
            replace_binding: false,
            registry_revision: local_tools::LocalToolRegistryRevision::BasicFiles,
        };

        let error = match start_native(&config, &credentials, &selection, &root) {
            Ok(_) => panic!("a missing credential must stop native startup"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("has no API credential"));
        let captured = repository.capture().unwrap();
        assert_eq!(
            captured.models()[0].last_failure().unwrap().kind(),
            ModelRequestFailureKind::LocalConfiguration
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
