mod local;
mod model;
mod mutation;
mod validation;

pub use local::LocalCredentialRepository;
pub(super) use model::parse_managed_revision_token;
pub use model::{CredentialRevision, CredentialSnapshot};
pub use mutation::{
    CredentialCommit, CredentialMutationAction, CredentialRepository,
    PreparedAccountSessionMutation, PreparedCredentialMutation,
};
pub(super) use validation::{CredentialEntry, StoredCredentialSnapshot};
