use std::path::Path;

use yo_core::{
    AgentBackend, CredentialStore, LocalCredentialStore, ModelConnectorLimits, NativeModelBackend,
    NativeModelBackendConfig, NativeModelBackendServices, ToolRegistry,
};

use super::{
    StartupBackend,
    tokenizer::{TokenizerRegistry, require_supported_tokenizer},
};
use crate::{AppError, config::Config, local_tools};

pub(super) fn start_native(
    config: &Config,
    credentials: &CredentialStore,
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
    require_supported_tokenizer(entry)?;
    let credential_path = config.credential_path();
    let credential = credentials.resolve(provider, account).cloned().ok_or_else(|| {
        AppError::many([format!(
            "credentials.yaml has no API credential for Provider {provider} and Account {account}"
        )])
    })?;
    let registry = runtime_registry(entry, *registry_revision)?;
    let semantic_admission = local_tools::LocalSemanticAdmission::new(credentials.clone());
    let tool_host = local_tools::LocalToolHost::new(workspace, &credential_path)
        .map_err(|error| AppError::single("starting local workspace tools", error))?;
    let services = NativeModelBackendServices::new(
        Some(Box::new(semantic_admission)),
        Box::new(tool_host),
        Box::new(TokenizerRegistry),
    );
    let backend_config = NativeModelBackendConfig {
        maximum_tool_argument_bytes: registry_revision.maximum_argument_bytes(),
        ..NativeModelBackendConfig::default()
    };
    NativeModelBackend::new(
        entry,
        credential,
        ModelConnectorLimits::default(),
        registry,
        services,
        backend_config,
    )
    .map(|backend| Box::new(backend) as Box<dyn AgentBackend + Send>)
    .map_err(|error| AppError::single("starting native model backend", error))
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

pub(super) fn open_credentials(path: &Path) -> Result<CredentialStore, AppError> {
    LocalCredentialStore::open(path)
        .map_err(|error| AppError::single("reading model credentials", error))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn explicit_entry(policy: &str) -> yo_core::ModelCatalogEntry {
        let complete = yo_core::CompleteModelBinding::from_durable_json(&format!(
            r#"{{"provider":"qwencloud","account":"default","model":"model","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{{}},"optional_request_parameters":{{}},"tool_capability_policy":"{policy}","verification_profile":"semantic-terminal/v1"}}"#
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
}
