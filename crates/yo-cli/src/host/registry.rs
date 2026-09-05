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
        assert!(from_backend_kind("future.backend").is_none());
        assert!(require_supported(&HostId::codex()).is_ok());
        assert!(require_supported(&HostId::grok()).is_ok());
        assert!(require_supported(&HostId::new("future").unwrap()).is_err());
    }
}
