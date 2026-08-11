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

#[cfg(test)]
mod tests {
    use super::*;

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
