//! Secret-free durable intent and pure recovery planning for multi-repository connections.

mod connect;
mod error;
mod execution;
mod journal;
mod recovery;
mod repository;
mod storage;
mod wire;

pub use connect::{
    ExternalConnectionError, PreparedExternalConnection, VerifiedExternalConnection,
    verify_external_connection,
};
pub use error::ConnectionOperationError;
pub use execution::{
    ConnectionOperationExecutionError, ConnectionOperationExecutionOutcome,
    ConnectionOperationRepositoryKind, LocalConnectionOperationRepositories,
    LocalConnectionOperationSession,
};
pub use journal::{
    ConnectionCredentialAction, ConnectionOperationJournalEntry, ConnectionOperationKind,
    ConnectionOperationPhase,
};
pub use recovery::{ConnectionOperationRecovery, plan_connection_recovery};
pub use repository::{ConnectionOperationJournalRepository, LocalConnectionOperationJournal};

#[cfg(test)]
mod tests;
