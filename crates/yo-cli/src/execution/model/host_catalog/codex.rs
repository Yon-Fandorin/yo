use std::path::PathBuf;

use yo_core::HostModelCatalog;

use super::super::DelegatedExecutionProfile;

pub(super) fn read_catalog(
    workspace: PathBuf,
    execution: DelegatedExecutionProfile,
    warning_observer: Option<yo_backend_delegated_codex::CodexWarningObserver>,
) -> Result<HostModelCatalog, String> {
    let config = yo_backend_delegated_codex::CodexBackendConfig::new(workspace)
        .with_read_only_review(execution.is_read_only_review());
    yo_backend_delegated_codex::read_model_catalog_with_warning_observer(config, warning_observer)
        .map_err(|error| error.to_string())
}
