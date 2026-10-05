use std::path::Path;

use yo_backend_managed::{
    NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices,
};
use yo_connector_kimi::KimiChatCompletionsConnector;
use yo_connector_openai_chat_completions::OpenAiChatCompletionsConnector;
use yo_connector_openai_responses::OpenAiResponsesConnector;
use yo_core::{
    AgentBackend, ApiCredential, ApiDialect, BackendAdapter, ConnectorId, CredentialSnapshot,
    LocalConnectionOperationRepositories, LocalCredentialRepository, LocalModelRequestObservation,
    ModelConnector, ModelConnectorLimits, ModelRequestFailureKind, ModelRequestOutcome,
    SessionDescriptor, ToolRegistry, session_repository::StoredSessionContinuation,
};

use super::{
    PreparedNativeFork, StartupBackend,
    tokenizer::{TokenizerRegistry, require_supported_tokenizer},
};
use crate::{AppError, execution::tools as local_tools, state::config::Config};

pub(super) fn start_native(
    config: &Config,
    credentials: &CredentialSnapshot,
    selection: &StartupBackend,
    workspace: &Path,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<(Box<dyn AgentBackend + Send>, Option<String>), AppError> {
    create_native(config, credentials, selection, workspace, cancelled)
        .map(|(backend, digest)| (Box::new(backend) as Box<dyn AgentBackend + Send>, digest))
}

pub(super) fn start_native_for_fork(
    config: &Config,
    credentials: &CredentialSnapshot,
    selection: &StartupBackend,
    workspace: &Path,
    parent: &StoredSessionContinuation,
    child: SessionDescriptor,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<PreparedNativeFork, AppError> {
    let (mut backend, digest) =
        create_native(config, credentials, selection, workspace, cancelled)?;
    match backend.prepare_exact_fork(parent, child) {
        Ok(prepared) => Ok((Box::new(backend), prepared, digest)),
        Err(error) => {
            let primary = AppError::single("preparing exact native fork", error);
            match backend.shutdown() {
                Ok(()) => Err(primary),
                Err(cleanup) => Err(AppError::many([
                    primary.to_string(),
                    format!("fork candidate cleanup failed: {cleanup}"),
                ])),
            }
        },
    }
}

fn create_native(
    config: &Config,
    credentials: &CredentialSnapshot,
    selection: &StartupBackend,
    workspace: &Path,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<(NativeModelBackend, Option<String>), AppError> {
    let StartupBackend::Native {
        provider,
        account,
        model,
        registry_revision,
        execution_manifest_digest: expected_digest,
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
    let commands = if *registry_revision == local_tools::LocalToolRegistryRevision::CommandTools {
        if entry
            .explicit_profile()
            .is_some_and(|profile| profile.tool_capability_policy().as_str() == "no-tools/v1")
        {
            return Err(AppError::message(
                "saved command tools cannot be replaced by no-tools",
            ));
        }
        Some(
            local_tools::PreparedCommandTools::prepare(
                config.command_tools(),
                workspace,
                &credential_path,
                cancelled,
            )
            .map_err(|error| AppError::single("preparing command tools", error))?
            .ok_or_else(|| {
                AppError::message("saved command tools require explicit configuration")
            })?,
        )
    } else {
        None
    };
    let digest = commands
        .as_ref()
        .map(|commands| commands.digest().to_owned());
    if expected_digest
        .as_ref()
        .is_some_and(|expected| Some(expected) != digest.as_ref())
    {
        return Err(AppError::message(
            "configured command execution manifest does not match the saved Session",
        ));
    }
    let registry = match &commands {
        Some(commands) => commands.registry().clone(),
        None => runtime_registry(entry, *registry_revision)
            .map_err(|error| with_local_configuration_observation(error, observation.as_ref()))?,
    };
    let semantic_admission =
        local_tools::LocalSemanticAdmission::new(credentials.credentials().clone());
    let tool_host = local_tools::LocalToolHost::new(workspace, &credential_path)
        .map_err(|error| AppError::single("starting local workspace tools", error))?
        .with_commands(commands);
    let mut services = NativeModelBackendServices::new(
        Box::new(super::NativeBindingAdmission),
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
    let backend_config = native_backend_config(*registry_revision, digest.clone(), &registry);
    if cancelled() {
        return Err(AppError::message("native backend preparation cancelled"));
    }
    let connector = native_connector(entry, credential, ModelConnectorLimits::default())
        .map_err(|error| with_local_configuration_observation(error, observation.as_ref()))?;
    let backend = NativeModelBackend::new(entry, connector, registry, services, backend_config)
        .map_err(|error| AppError::single("starting native model backend", error));
    backend
        .map(|backend| (backend, digest))
        .map_err(|error| with_local_configuration_observation(error, observation.as_ref()))
}

fn native_backend_config(
    revision: local_tools::LocalToolRegistryRevision,
    execution_manifest_digest: Option<String>,
    registry: &yo_core::FrozenToolRegistry,
) -> NativeModelBackendConfig {
    NativeModelBackendConfig {
        maximum_tool_argument_bytes: revision.maximum_argument_bytes(),
        execution_manifest_digest,
        ask_user_enabled: !registry.is_empty(),
        ..NativeModelBackendConfig::default()
    }
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
            match entry.complete_binding() {
                Some(complete) => OpenAiChatCompletionsConnector::with_complete_binding(
                    complete, credential, limits,
                ),
                None => OpenAiChatCompletionsConnector::new(binding, credential, limits),
            }
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
        .map_err(|error| AppError::single("opening the credential repository", error))?
        .capture()
        .map_err(|error| AppError::single("reading model credentials", error))
}

#[cfg(test)]
mod tests {
    use std::{env, fs, process, time};

    use super::*;
    use crate::state::config;

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

    fn chat_entry() -> yo_core::ModelCatalogEntry {
        let complete = yo_core::CompleteModelBinding::from_durable_json(
            r#"{"provider":"qwencloud","account":"default","model":"model","connector":"openai-chat-completions","base_url":"https://example.test/v1","api_dialect":"openai-chat-completions","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{},"optional_request_parameters":{},"tool_capability_policy":"no-tools/v1"}"#,
        )
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

    fn kimi_entry() -> yo_core::ModelCatalogEntry {
        let complete = yo_core::CompleteModelBinding::from_durable_json(
            r#"{"provider":"kimi","account":"default","model":"kimi-k3","connector":"kimi-chat-completions","base_url":"https://api.moonshot.ai/v1","api_dialect":"kimi-chat-completions","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1048576,"max_output_tokens":131072,"reasoning_parameters":{"effort":"max"},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1","replay_profile":"kimi-private-local-plaintext/v1"}"#,
        )
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

    // CLI composition root는 exact Chat identity+dialect tuple만 외부 Chat crate에 연결하고
    // trait object를 통해 base URL 뒤에 정확한 chat/completions 경로를 관찰합니다.
    #[test]
    fn composes_the_external_chat_connector_for_the_exact_binding() {
        let connector = native_connector(
            &chat_entry(),
            ApiCredential::new("secret").unwrap(),
            ModelConnectorLimits::default(),
        )
        .unwrap();

        assert_eq!(
            connector.request_url(),
            "https://example.test/v1/chat/completions"
        );
    }

    // Kimi는 exact Connector identity+dialect와 complete profile이 모두 일치할 때만
    // 전용 crate로 조립하며 endpoint 뒤에 chat/completions를 정확히 한 번 붙입니다.
    #[test]
    fn composes_the_kimi_connector_for_the_exact_complete_binding() {
        let connector = native_connector(
            &kimi_entry(),
            ApiCredential::new("secret").unwrap(),
            ModelConnectorLimits::default(),
        )
        .unwrap();

        assert_eq!(
            connector.request_url(),
            "https://api.moonshot.ai/v1/chat/completions"
        );
    }

    // durable binding 경계는 Kimi dialect를 다른 Connector identity와 조합한 입력을
    // composition 전에 거절하므로 Provider 이름 추론이나 fallback이 실행되지 않습니다.
    #[test]
    fn rejects_a_non_exact_kimi_connector_dialect_tuple_without_fallback() {
        let error = yo_core::CompleteModelBinding::from_durable_json(
            r#"{"provider":"kimi","account":"default","model":"kimi-k3","connector":"openai-chat-completions","base_url":"https://api.moonshot.ai/v1","api_dialect":"kimi-chat-completions","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1048576,"max_output_tokens":131072,"reasoning_parameters":{"effort":"max"},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1","replay_profile":"kimi-private-local-plaintext/v1"}"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("does not match api_dialect"));
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

    // startup이 선택한 실제 registry가 backend config로 넘어갈 때만 질문을 켭니다.
    // 명시적 no-tools와 durable no-tools는 모두 빈 registry를 유지합니다.
    #[test]
    fn new_session_question_registry_config_uses_the_resolved_runtime_registry() {
        for (policy, no_tools, expected) in [
            ("local-tools/v1", false, true),
            ("local-tools/v1", true, false),
            ("no-tools/v1", false, false),
        ] {
            let entry = explicit_entry(policy);
            let mut config = Config::default();
            config.replace_model_catalog(yo_core::ModelCatalog::new(vec![entry.clone()]).unwrap());
            let selection =
                super::super::startup::resolve(&config, None, Some("model"), no_tools, false, None)
                    .unwrap();
            let revision = selection.registry_revision().unwrap();
            let registry = runtime_registry(&entry, revision).unwrap();
            let backend_config = native_backend_config(revision, None, &registry);
            assert_eq!(backend_config.ask_user_enabled, expected);
            assert_eq!(
                backend_config.maximum_tool_argument_bytes,
                revision.maximum_argument_bytes()
            );
        }
        let empty = runtime_registry(
            &explicit_entry("local-tools/v1"),
            local_tools::LocalToolRegistryRevision::NoTools,
        )
        .unwrap();
        assert!(
            !native_backend_config(
                local_tools::LocalToolRegistryRevision::BasicFiles,
                None,
                &empty,
            )
            .ask_user_enabled
        );
    }

    // configured 도구도 준비된 registry를 그대로 handoff하며 질문 활성화는
    // manifest digest나 로컬 실행 도구의 개수와 내용을 바꾸지 않습니다.
    #[test]
    fn new_session_question_registry_config_includes_prepared_command_tools() {
        use std::os::unix::fs::PermissionsExt;

        let root = env::temp_dir().canonicalize().unwrap().join(format!(
            "yo-native-question-command-{}",
            yo_core::SessionId::new().unwrap(),
        ));
        fs::create_dir(&root).unwrap();
        let executable = root.join("fixture");
        fs::write(&executable, b"fixture executable bytes\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let path = root.join("config.yaml");
        fs::write(&path, serde_json::json!({"tools":{"commands":[{
            "id":"configured", "name":"configured", "description":"Run an explicit command.",
            "executable":executable, "parameters":{"type":"object","properties":{},"additionalProperties":false}
        }]}}).to_string()).unwrap();
        let mut config = config::load_from(&path).unwrap();
        config.replace_model_catalog(
            yo_core::ModelCatalog::new(vec![explicit_entry("local-tools/v1")]).unwrap(),
        );
        let selection =
            super::super::startup::resolve(&config, None, Some("model"), false, false, None)
                .unwrap();
        let revision = selection.registry_revision().unwrap();
        assert_eq!(
            revision,
            local_tools::LocalToolRegistryRevision::CommandTools
        );
        let prepared = local_tools::PreparedCommandTools::prepare(
            config.command_tools(),
            &root,
            &config.credential_path(),
            &mut || false,
        )
        .unwrap()
        .unwrap();
        let backend_config = native_backend_config(
            revision,
            Some(prepared.digest().to_owned()),
            prepared.registry(),
        );
        assert!(backend_config.ask_user_enabled);
        assert_eq!(
            backend_config.execution_manifest_digest.as_deref(),
            Some(prepared.digest())
        );
        assert_eq!(prepared.registry().definitions().len(), 6);
        fs::remove_dir_all(root).unwrap();
    }

    // native startup은 Codex 선택을 catalog 해석이나 credential 조회로 보내지 않고,
    // backend 종류가 잘못된 호출이라는 고정 진단으로 즉시 거절한다.
    #[test]
    fn native_startup_rejects_host_backend_before_catalog_resolution() {
        let credentials = LocalCredentialRepository::new(env::temp_dir().join(format!(
            "yo-native-wrong-backend-{}-missing.yaml",
            process::id()
        )))
        .expect("fixture credential path must be non-empty and absolute")
        .capture()
        .unwrap();
        let error = match start_native(
            &Config::default(),
            &credentials,
            &StartupBackend::Host(yo_core::HostId::codex()),
            Path::new("."),
            &mut || false,
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
        let temp_dir = fs::canonicalize(env::temp_dir())
            .expect("the native startup fixture temp directory must resolve physically");
        let root = temp_dir.join(format!(
            "yo-native-missing-credential-{}-{}",
            process::id(),
            time::SystemTime::now()
                .duration_since(time::SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let config_path = root.join("config.yaml");
        let mut config = config::load_from(&config_path).unwrap();
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
            .expect("fixture credential path must be non-empty and absolute")
            .capture()
            .unwrap();
        let selection = StartupBackend::Native {
            provider: complete.binding().provider_id().clone(),
            account: complete.binding().account_id().clone(),
            model: complete.binding().model_id().clone(),
            replace_binding: false,
            registry_revision: local_tools::LocalToolRegistryRevision::BasicFiles,
            execution_manifest_digest: None,
        };

        let error = match start_native(&config, &credentials, &selection, &root, &mut || false) {
            Ok(_) => panic!("a missing credential must stop native startup"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("has no API credential"));
        let captured = repository.capture().unwrap();
        assert_eq!(
            captured.models()[0].last_failure().unwrap().kind(),
            ModelRequestFailureKind::LocalConfiguration
        );
        let _ = fs::remove_dir_all(root);
    }
}
