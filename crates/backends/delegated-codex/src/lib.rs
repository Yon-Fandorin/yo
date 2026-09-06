//! Local Codex app-server adapter, account observation, and skill catalog boundaries.

mod binding;
mod client;
mod config;
mod observation;
mod protocol;
mod runtime;
mod skill_catalog;
mod transport;

#[cfg(test)]
mod test_support;

pub use binding::{CodexNativeModelBinding, native_model_binding, native_model_binding_from_trace};
pub use config::CodexBackendConfig;
pub use observation::{
    CodexRead, read_account_capacity, read_model_catalog, read_model_catalog_with_warning_observer,
};
pub use protocol::CodexCompatibilityWarning;
pub use runtime::{CodexBackend, CodexWarningObserver};
pub use skill_catalog::CodexSkillReferenceProvider;

pub const HOST_ID: &str = "codex";
pub const BACKEND_KIND: &str = "codex-app-server";
pub const STANDARD_BINDING_SCHEMA: &str = "codex.app-server/thread-binding/v2";
pub const READ_ONLY_BINDING_SCHEMA: &str = "codex.app-server/thread-binding/v1alpha2";

pub(crate) const READ_ONLY_REVIEW_PROFILE: &str = "yo.delegated-review-execution/v1alpha1";
pub(crate) const LEGACY_STANDARD_BINDING_SCHEMA: &str = "codex.app-server/thread-binding/v1";
pub(crate) const LEGACY_READ_ONLY_BINDING_SCHEMA: &str = "codex.app-server/thread-binding/v1alpha1";
pub(crate) const MODEL_IDENTITY_SCHEMA: &str = "codex.app-server/model-and-provider/v1";
