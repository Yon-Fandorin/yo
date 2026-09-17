use std::path::Path;

mod aggregate;
mod binding;
mod model;
mod projection;

pub(super) const PROVIDER_USAGE_SCHEMA: &str = "yo.external-review-provider-usage/v1alpha2";

pub(super) use model::{UsageBinding, UsageTarget};

pub(super) fn project(
    session_root: &Path,
    binding: UsageBinding,
) -> Result<model::ProviderUsageDocument, String> {
    projection::project(session_root, binding)
}
