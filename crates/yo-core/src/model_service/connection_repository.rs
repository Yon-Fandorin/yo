mod error;
mod local;
mod model;
mod mutation;
mod wire;

#[cfg(test)]
mod tests;

pub(crate) const MAX_CONNECTION_BYTES: u64 = 1024 * 1024;

#[cfg(test)]
pub(super) const PENDING_OPERATION_FILE: &str = local::PENDING_OPERATION_FILE;

pub use error::ConnectionRepositoryError;
#[cfg(test)]
pub(super) use local::{
    CONNECTION_TEMPORARY_ATTEMPTS_FOR_TEST, connection_temporary_path_for_test,
    create_connection_temporary_for_test,
};
pub use local::{LocalConnectionOperationGuard, LocalConnectionRepository};
pub use model::{
    ConnectionAccount, ConnectionCatalogSeed, ConnectionRevision, ConnectionSnapshot,
    ModelLastFailure, ModelRequestFailureKind, StoredModelBinding,
};
pub use mutation::{ConnectionCommit, ConnectionRepository, PreparedConnectionMutation};
