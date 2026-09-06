use std::path::Path;

use yo_core::{
    CompleteModelBinding, LocalConnectionOperationRepositories, LocalConnectionRepository,
    ModelSelection, StartupPolicy, StartupSelectionSources, StartupTarget, resolve_startup_target,
};

use crate::{AppError, state::config::Config};

pub(crate) fn absolute_config_path(
    path: std::path::PathBuf,
) -> Result<std::path::PathBuf, AppError> {
    if path.is_absolute() {
        return Ok(path);
    }
    std::env::current_dir()
        .map(|directory| directory.join(path))
        .map_err(|error| AppError::single("resolving the Yo configuration path", error))
}

pub(crate) fn operation_repositories(
    config_path: &Path,
) -> Result<LocalConnectionOperationRepositories, AppError> {
    let directory = config_path.parent().ok_or_else(|| {
        AppError::message("Yo configuration must have an absolute parent directory")
    })?;
    LocalConnectionOperationRepositories::in_directory(directory)
        .map_err(|error| AppError::single("opening connection repositories", error))
}

pub(crate) fn repository(config: &Config) -> LocalConnectionRepository {
    LocalConnectionRepository::new(config.connection_path())
}

pub(crate) fn admit_target(config: &Config, reference: &str) -> Result<StartupTarget, AppError> {
    let target = resolve_startup_target(
        config.model_catalog(),
        &StartupPolicy::initial(),
        StartupSelectionSources {
            invocation: Some(reference),
            stored_preference: None,
            operator_target: None,
        },
    )
    .map_err(|error| AppError::single("admitting the startup target", error))?
    .ok_or_else(|| AppError::message("target admission returned no startup target"))?;
    if let StartupTarget::Host(host) = &target {
        crate::execution::host::require_supported(host)?;
    }
    Ok(target)
}

pub(crate) fn display_target(target: Option<&StartupTarget>) -> String {
    match target {
        None => "unset".to_owned(),
        Some(StartupTarget::Host(host)) => host.reference(),
        Some(StartupTarget::Model(selection)) => {
            crate::interaction::connection::escape_remote_text(&selection.canonical_reference())
        },
    }
}

pub(crate) fn selection_for_binding(binding: &yo_core::EffectiveModelBinding) -> ModelSelection {
    ModelSelection::new(
        binding.provider_id().clone(),
        binding.account_id().clone(),
        binding.model_id().clone(),
    )
}

pub(crate) fn complete_binding_details(
    complete: &CompleteModelBinding,
) -> crate::interaction::connection::BindingDetails {
    crate::interaction::connection::BindingDetails::from(complete)
}

#[cfg(test)]
pub(crate) fn canonical_test_temp_dir() -> std::path::PathBuf {
    std::fs::canonicalize(std::env::temp_dir())
        .expect("the connection test temp directory must resolve to its physical path")
}

#[cfg(test)]
mod tests {
    use super::*;

    // 사용자에게 반환하는 ModelTarget은 예약 문자를 canonical escape로 보존해 다음 exact 명령에
    // 재사용합니다.
    #[test]
    fn model_target_output_uses_the_shared_canonical_reference() {
        let target = StartupTarget::Model(ModelSelection::new(
            yo_core::ProviderId::new("vendor:edge").unwrap(),
            yo_core::AccountId::new("team%blue").unwrap(),
            yo_core::ModelId::new("model:latest/v1").unwrap(),
        ));
        assert_eq!(
            display_target(Some(&target)),
            "vendor%3Aedge:team%25blue:model:latest/v1"
        );
    }
}
