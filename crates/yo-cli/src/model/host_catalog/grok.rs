use std::path::PathBuf;

use yo_core::HostModelCatalog;

use super::super::DelegatedExecutionProfile;

pub(super) fn read_catalog(
    workspace: PathBuf,
    execution: DelegatedExecutionProfile,
) -> Result<HostModelCatalog, String> {
    let outer_sandboxed_review = should_use_outer_sandboxed_review(
        execution,
        std::env::var_os(yo_backend_delegated_grok::OUTER_SANDBOX_REVIEW_ENV).is_some(),
    );
    let config = yo_backend_delegated_grok::GrokBackendConfig::new(workspace)
        .with_read_only_review(execution.is_read_only_review())
        .with_outer_sandboxed_review(outer_sandboxed_review);
    yo_backend_delegated_grok::read_model_catalog(config).map_err(|error| error.to_string())
}

fn should_use_outer_sandboxed_review(
    execution: DelegatedExecutionProfile,
    outer_sandbox_available: bool,
) -> bool {
    outer_sandbox_available && execution.is_read_only_review()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Standard+available, read-only+unavailable, read-only+available 조합이 각각
    // false/false/true인지 확인해 Grok outer sandbox 정책이 공통 inventory 계획으로 새지 않고
    // Grok owner에서만 결정되도록 보호합니다.
    #[test]
    fn outer_sandbox_is_only_enabled_for_available_read_only_reviews() {
        assert!(!should_use_outer_sandboxed_review(
            DelegatedExecutionProfile::Standard,
            true,
        ));
        assert!(!should_use_outer_sandboxed_review(
            DelegatedExecutionProfile::ReadOnlyReview,
            false,
        ));
        assert!(should_use_outer_sandboxed_review(
            DelegatedExecutionProfile::ReadOnlyReview,
            true,
        ));
    }
}
