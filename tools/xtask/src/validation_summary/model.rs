use serde::Deserialize;

pub(crate) struct VerifiedSummary {
    pub(crate) status: String,
    pub(crate) log_path: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct SchemaEnvelope {
    pub(super) schema: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResourceLease {
    pub(super) schema: String,
    pub(super) class: String,
    pub(super) key: String,
    pub(super) status: String,
    pub(super) wait_attempts: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReuseContext {
    pub(super) schema: String,
    pub(super) platform_os: String,
    pub(super) platform_arch: String,
    pub(super) toolchain_hash: String,
    pub(super) external_state: String,
}
