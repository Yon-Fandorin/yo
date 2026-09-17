//! 세션 연속성 파사드와 안정적인 공개/저장소 내보내기.

mod catalog;
mod model;
mod recovery;
mod validation;

pub(crate) use catalog::read_fork_catalog;
pub use model::{
    SessionForkLimits, StoredSessionForkBoundary, StoredSessionForkCatalog,
    StoredSessionForkSelection, StoredSessionForkSourceKind,
};
#[cfg(test)]
pub(crate) use recovery::build_continuation;
pub use recovery::{
    StoredSessionContinuation, StoredSessionContinuationError, read_stored_session_continuation,
    recover_stored_session_continuation,
};

#[cfg(test)]
mod tests;
