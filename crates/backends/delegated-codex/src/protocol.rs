//! Codex app-server wire protocol façade.
//!
//! 세부 wire 해석은 책임별 하위 모듈에 두고, 이 파일은 기존 소비자가 계속
//! 사용할 수 있는 안정적인 `protocol::...` 경로만 다시 내보냅니다.

mod account;
mod bounds;
mod initialize;
mod message;
mod models;

pub(super) use account::{
    decode_account_capacity, decode_account_capacity_identity, decode_account_identity,
    decode_optional_account_identity,
};
pub(super) use bounds::{
    MAX_USER_AGENT_DISPLAY_BYTES, protocol_failure, safe_user_agent, string_at,
};
pub use initialize::{CodexCompatibilityWarning, CodexWarning};
pub(super) use initialize::{
    InitializeResult, decode_initialize, image_wire_version_supported,
    version_compatibility_warning,
};
pub(super) use message::{
    Incoming, classify, initialized_notification, request, server_error, server_response,
};
pub(super) use models::{ModelImageModality, ModelListModel, ModelListPage, decode_model_list};
#[cfg(test)]
use yo_core::BackendFailureKind;

#[cfg(test)]
mod tests;
