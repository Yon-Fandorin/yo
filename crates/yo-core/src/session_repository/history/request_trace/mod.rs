//! Frontend-independent diagnostic records derived from validated Journal correlation state.

mod model;
mod projection;

pub use model::{
    StoredBindingCacheState, StoredBindingCloseReason, StoredBindingTransition,
    StoredBindingTransitionMode, StoredExchangeDirection, StoredExchangeKind,
    StoredRequestDetailAvailability, StoredRequestTraceEntry, StoredRequestTraceRecord,
};
pub(super) fn project(
    recovered: &crate::journal::codec::RecoveredJournal,
) -> Vec<StoredRequestTraceEntry> {
    projection::project(recovered)
}

#[cfg(test)]
mod tests;
