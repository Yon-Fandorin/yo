//! 저장된 Session snapshot 하나에서 복구한 검증된 의미 history입니다.

mod discovery;
mod inherited;
mod model;
mod normalizer;
mod read;
mod request_trace;
mod session_usage;

// 내부 복구 소비자는 이 facade를 통해서만 history 조립 연산에 접근합니다.
pub(in crate::session_repository) use discovery::validate_discovery;
pub use discovery::{
    StoredDiscoveryMismatch, StoredDiscoveryMismatchKind, StoredDiscoveryValidation,
};
pub(in crate::session_repository) use inherited::project_inherited;
pub use inherited::{InheritedHistorySection, InheritedHistorySource, InheritedSessionHistory};
pub use model::{StoredSessionContinuity, StoredSessionHistory, StoredSessionRecovery};
pub(in crate::session_repository) use read::normalize_recovered;
pub use read::{StoredSessionReadError, read_stored_session};
pub use request_trace::{
    StoredBindingCacheState, StoredBindingCloseReason, StoredBindingTransition,
    StoredBindingTransitionMode, StoredContinuationStrategy, StoredExchangeDirection,
    StoredExchangeKind, StoredReplayExecutor, StoredRequestDetailAvailability,
    StoredRequestTraceEntry, StoredRequestTraceRecord,
};
pub use session_usage::{
    CODEX_USAGE_SCHEMA, CacheReadShare, CacheReadSummary, GROK_USAGE_SCHEMA, MANAGED_USAGE_SCHEMA,
    SessionUsage, SessionUsageAggregates, SessionUsageError, SessionUsageProjection,
    SessionUsageProvider, SessionUsageReceipt, SessionUsageSource, UsageAggregate, UsageCoverage,
    UsageValue,
};

#[cfg(test)]
mod tests;
