//! Local Grok ACP adapter, runtime admission, and account/catalog observation boundaries.

mod admission;
mod billing_log;
mod client;
mod config;
mod observation;
mod protocol;
mod runtime;
mod transport;

pub use config::{
    GrokBackendConfig, NATIVE_SANDBOX_REVIEW_PROFILE, OUTER_SANDBOX_REVIEW_ENV,
    OUTER_SANDBOX_REVIEW_PROFILE, OUTER_SANDBOX_SENTINEL, REVIEW_RUNNER_CAPABILITIES,
};
pub use observation::{read_account_capacity, read_model_catalog};
pub use runtime::GrokBackend;

pub const HOST_ID: &str = "grok";
pub const BACKEND_KIND: &str = "grok-build-acp";

pub(crate) const READ_ONLY_REVIEW_PROFILE: &str = "yo.delegated-review-execution/v1alpha1";
pub(crate) const STANDARD_BINDING_SCHEMA: &str = "grok.acp/session-binding/v1";
pub(crate) const READ_ONLY_BINDING_SCHEMA: &str = "grok.acp/session-binding/v1alpha1";
