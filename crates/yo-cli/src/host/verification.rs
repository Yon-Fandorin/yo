use std::path::Path;

use yo_core::HostId;

use crate::{AppError, host::registry::require_supported};

pub(crate) fn verify_at_with_codex_warning_observer(
    host: &HostId,
    workspace: &Path,
    warning_observer: Option<yo_backend_delegated_codex::CodexWarningObserver>,
) -> Result<(), AppError> {
    require_supported(host)?;
    match host.as_str() {
        yo_backend_delegated_codex::HOST_ID => {
            yo_backend_delegated_codex::CodexBackend::verify_with_warning_observer(
                yo_backend_delegated_codex::CodexBackendConfig::new(workspace),
                warning_observer,
            )
            .map_err(|error| AppError::single("verifying Local Codex", error))
        },
        yo_backend_delegated_grok::HOST_ID => yo_backend_delegated_grok::GrokBackend::verify(
            yo_backend_delegated_grok::GrokBackendConfig::new(workspace),
        )
        .map_err(|error| AppError::single("verifying Local Grok", error)),
        _ => unreachable!("supported hosts are exhaustively dispatched above"),
    }
}
