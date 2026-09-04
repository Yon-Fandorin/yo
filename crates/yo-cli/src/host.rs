use std::path::Path;

use yo_core::HostId;

use crate::AppError;

pub(crate) fn require_supported(host: &HostId) -> Result<(), AppError> {
    match host.as_str() {
        yo_backend_delegated_codex::HOST_ID | yo_backend_delegated_grok::HOST_ID => Ok(()),
        other => Err(AppError::message(format!(
            "unsupported agent host {other:?}; this yo build includes host:codex and host:grok"
        ))),
    }
}

pub(crate) fn from_backend_kind(kind: &str) -> Option<HostId> {
    match kind {
        yo_backend_delegated_codex::BACKEND_KIND => Some(HostId::codex()),
        yo_backend_delegated_grok::BACKEND_KIND => Some(HostId::grok()),
        _ => None,
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    // CLI host registry는 공개 target ID와 durable backend kind를 같은 Codex·Grok
    // 어댑터로 왕복시키고, 이 빌드에 없는 host는 startup 전에 거절합니다.
    #[test]
    fn registry_keeps_target_and_backend_identities_aligned() {
        assert_eq!(
            from_backend_kind(yo_backend_delegated_codex::BACKEND_KIND),
            Some(HostId::codex())
        );
        assert_eq!(
            from_backend_kind(yo_backend_delegated_grok::BACKEND_KIND),
            Some(HostId::grok())
        );
        assert!(require_supported(&HostId::codex()).is_ok());
        assert!(require_supported(&HostId::grok()).is_ok());
        assert!(require_supported(&HostId::new("future").unwrap()).is_err());
    }
}
