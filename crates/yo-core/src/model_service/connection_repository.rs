mod local;
mod model;
mod mutation;

#[cfg(test)]
mod tests;

pub(crate) const MAX_CONNECTION_BYTES: u64 = 1024 * 1024;
const FILE_MODE: u32 = 0o600;
const DIRECTORY_MODE: u32 = 0o700;
#[cfg(target_vendor = "apple")]
const REGULAR_FILE_MODE: u32 = libc::S_IFREG as u32;
#[cfg(not(target_vendor = "apple"))]
const REGULAR_FILE_MODE: u32 = libc::S_IFREG;
#[cfg(target_vendor = "apple")]
const FILE_TYPE_MASK: u32 = libc::S_IFMT as u32;
#[cfg(not(target_vendor = "apple"))]
const FILE_TYPE_MASK: u32 = libc::S_IFMT;
const REPOSITORY_LOCK_FILE: &str = ".connections.lock";
const OPERATION_LOCK_FILE: &str = ".connection-operation.lock";
const PENDING_OPERATION_FILE: &str = "connection-operation.yaml";
// One retry distinguishes an occupied candidate from a persistently abnormal name source.
const CONNECTION_TEMPORARY_ATTEMPTS: usize = 2;

#[cfg(test)]
pub(super) use local::{
    CONNECTION_TEMPORARY_ATTEMPTS_FOR_TEST, connection_temporary_path_for_test,
    create_connection_temporary_for_test,
};
pub use local::{LocalConnectionOperationGuard, LocalConnectionRepository};
pub(super) use local::{decode_snapshot, encode_snapshot, new_revision, parse_revision_token};
pub(super) use model::{
    CatalogSource, DecodedSnapshot, account_matches_binding, binding_matches_selection,
    validate_state,
};
pub use model::{
    ConnectionAccount, ConnectionCatalogSeed, ConnectionRepositoryError, ConnectionRevision,
    ConnectionSnapshot, ModelLastFailure, ModelRequestFailureKind, StoredModelBinding,
};
pub(super) use mutation::validate_catalog_seeds;
pub use mutation::{ConnectionCommit, ConnectionRepository, PreparedConnectionMutation};
