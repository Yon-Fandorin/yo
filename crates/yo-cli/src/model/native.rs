use std::path::Path;

use yo_core::{
    AgentBackend, CredentialStore, LocalCredentialStore, ModelConnectorLimits, NativeModelBackend,
    NativeModelBackendConfig, NativeModelBackendServices,
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

pub(super) fn open_credentials(path: &Path) -> Result<CredentialStore, AppError> {
    LocalCredentialStore::open(path)
        .map_err(|error| AppError::single("reading model credentials", error))
}
