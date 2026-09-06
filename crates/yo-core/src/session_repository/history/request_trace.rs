//! Frontend-independent diagnostic records derived from validated Journal correlation state.

pub use crate::request_trace::{
    RequestTraceEntry as StoredRequestTraceEntry, RequestTraceRecord as StoredRequestTraceRecord,
    StoredBindingCacheState, StoredBindingCloseReason, StoredBindingTransition,
    StoredBindingTransitionMode, StoredContinuationStrategy, StoredExchangeDirection,
    StoredExchangeKind, StoredReplayExecutor, StoredRequestDetailAvailability,
};
pub(super) fn project(
    recovered: &crate::journal::codec::RecoveredJournal,
) -> Vec<StoredRequestTraceEntry> {
    crate::request_trace::project_recovered(recovered)
}

#[cfg(test)]
mod tests;
