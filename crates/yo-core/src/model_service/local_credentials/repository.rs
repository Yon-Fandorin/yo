use std::path::{Path, PathBuf};

use super::{
    super::{AccountId, ApiCredential, CredentialStore, ProviderId},
    LocalCredentialStoreError, storage, wire,
};

/// Private opaque compare-and-swap receipt for one complete credential snapshot.
///
/// Its diagnostic projections deliberately hide the underlying token. The token may only be
/// persisted by the credential snapshot and the permission-restricted connection operation
/// journal.
#[derive(Clone, Eq, PartialEq)]
pub struct CredentialRevision(CredentialRevisionKind);

#[derive(Clone, Eq, PartialEq)]
enum CredentialRevisionKind {
    Absent,
    Managed(String),
    Derived(String),
}

impl CredentialRevision {
    #[must_use]
    pub const fn is_absent(&self) -> bool {
        matches!(self.0, CredentialRevisionKind::Absent)
    }

    pub(super) fn absent() -> Self {
        Self(CredentialRevisionKind::Absent)
    }

    pub(super) fn managed(token: String) -> Self {
        Self(CredentialRevisionKind::Managed(token))
    }

    pub(super) fn derived(token: String) -> Self {
        Self(CredentialRevisionKind::Derived(token))
    }

    pub(super) fn managed_token(&self) -> Option<&str> {
        match &self.0 {
            CredentialRevisionKind::Managed(token) => Some(token),
            CredentialRevisionKind::Absent | CredentialRevisionKind::Derived(_) => None,
        }
    }

    pub(crate) fn operation_journal_token(&self) -> &str {
        match &self.0 {
            CredentialRevisionKind::Absent => "absent",
            CredentialRevisionKind::Managed(token) | CredentialRevisionKind::Derived(token) => {
                token
            },
        }
    }

    pub(crate) fn from_operation_journal(value: &str) -> Option<Self> {
        if value == "absent" {
            return Some(Self::absent());
        }
        wire::parse_managed_revision_token(value)
            .map(Self::managed)
            .or_else(|| wire::parse_derived_revision_token(value).map(Self::derived))
    }
}

impl std::fmt::Debug for CredentialRevision {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_absent() {
            formatter.write_str("CredentialRevision(Absent)")
        } else {
            formatter.write_str("CredentialRevision([PRIVATE])")
        }
    }
}

/// One immutable credential snapshot. Secret values remain accessible only through exact
/// Provider-and-Account resolution and retain `ApiCredential` redaction behavior.
#[derive(Clone)]
pub struct CredentialSnapshot {
    revision: CredentialRevision,
    credentials: CredentialStore,
    account_sessions: CredentialStore,
}

impl CredentialSnapshot {
    #[must_use]
    pub const fn revision(&self) -> &CredentialRevision {
        &self.revision
    }

    #[must_use]
    pub fn resolve(&self, provider: &ProviderId, account: &AccountId) -> Option<&ApiCredential> {
        self.credentials.resolve(provider, account)
    }

    /// Resolves the optional account-observation session for one exact Provider and Account.
    ///
    /// Model dispatch must continue to use [`Self::resolve`]; this secret is reserved for the
    /// account-capacity boundary that owns its fixed remote origin.
    #[must_use]
    pub fn resolve_account_session(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Option<&ApiCredential> {
        self.account_sessions.resolve(provider, account)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.credentials.is_empty()
    }

    #[must_use]
    pub const fn credentials(&self) -> &CredentialStore {
        &self.credentials
    }

    /// Prepares an exact account-session mutation against this observed credential revision.
    ///
    /// Keeping preparation on the snapshot lets a caller bind later secret capture and remote
    /// work to the state it actually inspected instead of silently replanning at commit time.
    pub fn prepare_set_account_session(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Result<PreparedAccountSessionMutation, LocalCredentialStoreError> {
        if self.resolve(provider, account).is_none() {
            return Err(LocalCredentialStoreError::InvalidMutation);
        }
        let action = if self.resolve_account_session(provider, account).is_some() {
            CredentialMutationAction::Replace
        } else {
            CredentialMutationAction::Add
        };
        Ok(PreparedAccountSessionMutation {
            expected_revision: self.revision.clone(),
            planned_revision: wire::new_revision()?,
            provider: provider.clone(),
            account: account.clone(),
            action,
        })
    }

    pub(crate) fn matches_expected(&self, mutation: &PreparedCredentialMutation) -> bool {
        self.revision == mutation.expected_revision
            && mutation.action.matches_presence(
                self.resolve(&mutation.provider, &mutation.account)
                    .is_some(),
            )
    }

    pub(crate) fn matches_planned(&self, mutation: &PreparedCredentialMutation) -> bool {
        if self.revision != mutation.planned_revision {
            return false;
        }
        let present = self
            .resolve(&mutation.provider, &mutation.account)
            .is_some();
        match mutation.action {
            CredentialMutationAction::Add | CredentialMutationAction::Replace => present,
            CredentialMutationAction::Remove => !present,
        }
    }
}

impl std::fmt::Debug for CredentialSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CredentialSnapshot")
            .field("revision", &self.revision)
            .field("credential_count", &self.credentials.len())
            .field("account_session_count", &self.account_sessions.len())
            .finish()
    }
}

/// Closed exact-pair action bound into a prepared credential mutation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialMutationAction {
    Add,
    Replace,
    Remove,
}

/// One prepared mutation. It contains coordinates and private revisions but never secret bytes.
#[derive(Clone, Eq, PartialEq)]
pub struct PreparedCredentialMutation {
    expected_revision: CredentialRevision,
    planned_revision: CredentialRevision,
    provider: ProviderId,
    account: AccountId,
    action: CredentialMutationAction,
}

/// One prepared mutation of the optional account-session field for an existing exact account.
///
/// The candidate secret remains in the caller until commit and is never retained here.
#[derive(Clone, Eq, PartialEq)]
pub struct PreparedAccountSessionMutation {
    expected_revision: CredentialRevision,
    planned_revision: CredentialRevision,
    provider: ProviderId,
    account: AccountId,
    action: CredentialMutationAction,
}

impl std::fmt::Debug for PreparedAccountSessionMutation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedAccountSessionMutation")
            .field("expected_revision", &self.expected_revision)
            .field("planned_revision", &self.planned_revision)
            .field("provider", &self.provider)
            .field("account", &self.account)
            .field("action", &self.action)
            .finish()
    }
}

impl PreparedCredentialMutation {
    #[must_use]
    pub const fn expected_revision(&self) -> &CredentialRevision {
        &self.expected_revision
    }

    #[must_use]
    pub const fn planned_revision(&self) -> &CredentialRevision {
        &self.planned_revision
    }

    #[must_use]
    pub const fn action(&self) -> CredentialMutationAction {
        self.action
    }

    #[must_use]
    pub const fn provider(&self) -> &ProviderId {
        &self.provider
    }

    #[must_use]
    pub const fn account(&self) -> &AccountId {
        &self.account
    }

    pub(crate) fn from_operation_journal(
        expected_revision: CredentialRevision,
        planned_revision: CredentialRevision,
        provider: ProviderId,
        account: AccountId,
        action: CredentialMutationAction,
    ) -> Option<Self> {
        if planned_revision.managed_token().is_none() || expected_revision == planned_revision {
            return None;
        }
        Some(Self {
            expected_revision,
            planned_revision,
            provider,
            account,
            action,
        })
    }
}

impl PreparedAccountSessionMutation {
    #[must_use]
    pub const fn planned_revision(&self) -> &CredentialRevision {
        &self.planned_revision
    }
}

impl std::fmt::Debug for PreparedCredentialMutation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedCredentialMutation")
            .field("expected_revision", &self.expected_revision)
            .field("planned_revision", &self.planned_revision)
            .field("provider", &self.provider)
            .field("account", &self.account)
            .field("action", &self.action)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialCommit {
    Committed,
    AlreadyCommitted,
}

/// Storage-neutral exact-pair credential mutation boundary.
pub trait CredentialRepository {
    fn capture(&self) -> Result<CredentialSnapshot, LocalCredentialStoreError>;

    fn prepare_set(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Result<PreparedCredentialMutation, LocalCredentialStoreError>;

    fn prepare_remove(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Result<Option<PreparedCredentialMutation>, LocalCredentialStoreError>;

    fn commit(
        &self,
        mutation: &PreparedCredentialMutation,
        candidate: Option<&ApiCredential>,
    ) -> Result<CredentialCommit, LocalCredentialStoreError>;
}

/// Local bounded `credentials.yaml` repository with private exact-revision CAS.
#[derive(Clone, Debug)]
pub struct LocalCredentialRepository {
    path: PathBuf,
}

impl LocalCredentialRepository {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Captures without creating the file or its parent directory.
    pub fn capture(&self) -> Result<CredentialSnapshot, LocalCredentialStoreError> {
        storage::read_snapshot(&self.path).map(StoredCredentialSnapshot::public)
    }

    /// Re-reads under the credential lock and prepares an exact add or replace without retaining
    /// the candidate secret.
    pub fn prepare_set(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Result<PreparedCredentialMutation, LocalCredentialStoreError> {
        self.prepare(provider, account, true)?
            .ok_or(LocalCredentialStoreError::InvalidMutation)
    }

    /// Re-reads under the credential lock and prepares an exact removal, or returns `None` when
    /// the exact pair is already absent.
    pub fn prepare_remove(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Result<Option<PreparedCredentialMutation>, LocalCredentialStoreError> {
        self.prepare(provider, account, false)
    }

    /// Prepares an add or replacement of the optional account-session field for an existing
    /// Provider-and-Account API credential.
    pub fn prepare_set_account_session(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Result<PreparedAccountSessionMutation, LocalCredentialStoreError> {
        let (_parent, lock) = storage::lock_repository(&self.path)?;
        let snapshot = storage::read_snapshot(&self.path)?.public();
        let mutation = snapshot.prepare_set_account_session(provider, account)?;
        drop(lock);
        Ok(mutation)
    }

    fn prepare(
        &self,
        provider: &ProviderId,
        account: &AccountId,
        set: bool,
    ) -> Result<Option<PreparedCredentialMutation>, LocalCredentialStoreError> {
        let (_parent, lock) = storage::lock_repository(&self.path)?;
        let snapshot = storage::read_snapshot(&self.path)?;
        let present = snapshot.resolve(provider, account).is_some();
        let action = match (set, present) {
            (true, false) => CredentialMutationAction::Add,
            (true, true) => CredentialMutationAction::Replace,
            (false, true) => CredentialMutationAction::Remove,
            (false, false) => return Ok(None),
        };
        let mutation = PreparedCredentialMutation {
            expected_revision: snapshot.revision,
            planned_revision: wire::new_revision()?,
            provider: provider.clone(),
            account: account.clone(),
            action,
        };
        drop(lock);
        Ok(Some(mutation))
    }

    /// Commits the exact prepared pair action. Add and replace require the still in-memory
    /// candidate; remove rejects one so a caller cannot accidentally persist an unrelated secret.
    pub fn commit(
        &self,
        mutation: &PreparedCredentialMutation,
        candidate: Option<&ApiCredential>,
    ) -> Result<CredentialCommit, LocalCredentialStoreError> {
        validate_candidate(mutation.action, candidate)?;
        let (parent, lock) = storage::lock_repository(&self.path)?;
        let mut current = storage::read_snapshot(&self.path)?;

        if current.revision == mutation.planned_revision {
            let applied = match mutation.action {
                CredentialMutationAction::Add | CredentialMutationAction::Replace => {
                    current.resolve(&mutation.provider, &mutation.account) == candidate
                },
                CredentialMutationAction::Remove => current
                    .resolve(&mutation.provider, &mutation.account)
                    .is_none(),
            };
            return if applied {
                Ok(CredentialCommit::AlreadyCommitted)
            } else {
                Err(LocalCredentialStoreError::Conflict(self.path.clone()))
            };
        }
        if current.revision != mutation.expected_revision
            || !mutation.action.matches_presence(
                current
                    .resolve(&mutation.provider, &mutation.account)
                    .is_some(),
            )
        {
            return Err(LocalCredentialStoreError::Conflict(self.path.clone()));
        }

        current.apply(mutation, candidate);
        let planned = mutation
            .planned_revision
            .managed_token()
            .expect("new credential revisions are always managed tokens");
        let encoded = wire::encode(planned, &current.entries)?;
        storage::publish(
            &self.path,
            &parent,
            mutation.expected_revision.is_absent(),
            &encoded,
        )?;
        drop(lock);
        Ok(CredentialCommit::Committed)
    }

    /// Commits one prepared account-session add or replacement without changing the model API
    /// credential in the same account record.
    pub fn commit_account_session(
        &self,
        mutation: &PreparedAccountSessionMutation,
        candidate: &ApiCredential,
    ) -> Result<CredentialCommit, LocalCredentialStoreError> {
        let (parent, lock) = storage::lock_repository(&self.path)?;
        let mut current = storage::read_snapshot(&self.path)?;

        if current.revision == mutation.planned_revision {
            return if current.resolve_account_session(&mutation.provider, &mutation.account)
                == Some(candidate)
            {
                Ok(CredentialCommit::AlreadyCommitted)
            } else {
                Err(LocalCredentialStoreError::Conflict(self.path.clone()))
            };
        }
        if current.revision != mutation.expected_revision
            || current
                .resolve(&mutation.provider, &mutation.account)
                .is_none()
            || !mutation.action.matches_presence(
                current
                    .resolve_account_session(&mutation.provider, &mutation.account)
                    .is_some(),
            )
        {
            return Err(LocalCredentialStoreError::Conflict(self.path.clone()));
        }

        current.apply_account_session(mutation, candidate);
        let planned = mutation
            .planned_revision
            .managed_token()
            .expect("new credential revisions are always managed tokens");
        let encoded = wire::encode(planned, &current.entries)?;
        storage::publish(
            &self.path,
            &parent,
            mutation.expected_revision.is_absent(),
            &encoded,
        )?;
        drop(lock);
        Ok(CredentialCommit::Committed)
    }
}

impl CredentialRepository for LocalCredentialRepository {
    fn capture(&self) -> Result<CredentialSnapshot, LocalCredentialStoreError> {
        Self::capture(self)
    }

    fn prepare_set(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Result<PreparedCredentialMutation, LocalCredentialStoreError> {
        Self::prepare_set(self, provider, account)
    }

    fn prepare_remove(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Result<Option<PreparedCredentialMutation>, LocalCredentialStoreError> {
        Self::prepare_remove(self, provider, account)
    }

    fn commit(
        &self,
        mutation: &PreparedCredentialMutation,
        candidate: Option<&ApiCredential>,
    ) -> Result<CredentialCommit, LocalCredentialStoreError> {
        Self::commit(self, mutation, candidate)
    }
}

impl CredentialMutationAction {
    fn matches_presence(self, present: bool) -> bool {
        match self {
            Self::Add => !present,
            Self::Replace | Self::Remove => present,
        }
    }
}

fn validate_candidate(
    action: CredentialMutationAction,
    candidate: Option<&ApiCredential>,
) -> Result<(), LocalCredentialStoreError> {
    match (action, candidate) {
        (CredentialMutationAction::Add | CredentialMutationAction::Replace, Some(_))
        | (CredentialMutationAction::Remove, None) => Ok(()),
        (CredentialMutationAction::Add | CredentialMutationAction::Replace, None)
        | (CredentialMutationAction::Remove, Some(_)) => {
            Err(LocalCredentialStoreError::InvalidMutation)
        },
    }
}

#[derive(Clone)]
pub(super) struct StoredCredentialSnapshot {
    revision: CredentialRevision,
    pub(super) entries: Vec<CredentialEntry>,
    credentials: CredentialStore,
    account_sessions: CredentialStore,
}

impl StoredCredentialSnapshot {
    pub(super) fn new(
        revision: CredentialRevision,
        entries: Vec<CredentialEntry>,
    ) -> Result<Self, LocalCredentialStoreError> {
        let mut credentials = CredentialStore::new(entries.iter().map(|entry| {
            (
                (entry.provider.clone(), entry.account.clone()),
                entry.credential.clone(),
            )
        }))
        .map_err(|_| LocalCredentialStoreError::InvalidContents(PathBuf::new()))?;
        let account_sessions = CredentialStore::new(entries.iter().filter_map(|entry| {
            entry.account_session.as_ref().map(|session| {
                (
                    (entry.provider.clone(), entry.account.clone()),
                    session.clone(),
                )
            })
        }))
        .map_err(|_| LocalCredentialStoreError::InvalidContents(PathBuf::new()))?;
        credentials.retain_auxiliary_secret_material(
            entries
                .iter()
                .filter_map(|entry| entry.account_session.clone()),
        );
        Ok(Self {
            revision,
            entries,
            credentials,
            account_sessions,
        })
    }

    pub(super) fn into_credentials(self) -> CredentialStore {
        self.credentials
    }

    fn public(self) -> CredentialSnapshot {
        CredentialSnapshot {
            revision: self.revision,
            credentials: self.credentials,
            account_sessions: self.account_sessions,
        }
    }

    fn resolve(&self, provider: &ProviderId, account: &AccountId) -> Option<&ApiCredential> {
        self.credentials.resolve(provider, account)
    }

    fn resolve_account_session(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Option<&ApiCredential> {
        self.account_sessions.resolve(provider, account)
    }

    fn apply(&mut self, mutation: &PreparedCredentialMutation, candidate: Option<&ApiCredential>) {
        let position = self.entries.iter().position(|entry| {
            entry.provider == mutation.provider && entry.account == mutation.account
        });
        match mutation.action {
            CredentialMutationAction::Add => self.entries.push(CredentialEntry {
                provider: mutation.provider.clone(),
                account: mutation.account.clone(),
                credential: candidate.expect("candidate presence was validated").clone(),
                account_session: None,
            }),
            CredentialMutationAction::Replace => {
                self.entries[position.expect("replace presence was validated")].credential =
                    candidate.expect("candidate presence was validated").clone();
            },
            CredentialMutationAction::Remove => {
                self.entries
                    .remove(position.expect("remove presence was validated"));
            },
        }
    }

    fn apply_account_session(
        &mut self,
        mutation: &PreparedAccountSessionMutation,
        candidate: &ApiCredential,
    ) {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.provider == mutation.provider && entry.account == mutation.account)
            .expect("account-session preparation requires the model credential to remain present");
        entry.account_session = Some(candidate.clone());
    }
}

#[derive(Clone)]
pub(super) struct CredentialEntry {
    pub(super) provider: ProviderId,
    pub(super) account: AccountId,
    pub(super) credential: ApiCredential,
    pub(super) account_session: Option<ApiCredential>,
}
