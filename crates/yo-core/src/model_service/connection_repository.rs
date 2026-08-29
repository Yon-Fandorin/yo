use std::{
    fmt, fs,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use super::{ModelCatalog, ModelCatalogEntry, ModelSelection, StartupTarget};

mod catalog_seed;
mod error;
mod stored;
mod wire;

pub use catalog_seed::ConnectionCatalogSeed;
pub use error::ConnectionRepositoryError;
pub use stored::{
    ConnectionAccount, ModelLastFailure, ModelRequestFailureKind, StoredModelBinding,
};

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

type StoredSnapshotState = (
    Option<StartupTarget>,
    Vec<ConnectionAccount>,
    Vec<StoredModelBinding>,
    Vec<ConnectionCatalogSeed>,
);

/// Opaque compare-and-swap token for one complete public connection snapshot.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ConnectionRevision {
    Absent,
    Token(String),
}

impl ConnectionRevision {
    #[must_use]
    pub const fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }

    pub(crate) fn from_operation_journal(value: &str) -> Option<Self> {
        if value == "absent" {
            return Some(Self::Absent);
        }
        wire::parse_revision_token(value).map(Self::Token)
    }
}

impl fmt::Display for ConnectionRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent => formatter.write_str("absent"),
            Self::Token(token) => formatter.write_str(token),
        }
    }
}

/// One bounded immutable public repository capture.
#[derive(Clone, Debug)]
pub struct ConnectionSnapshot {
    revision: ConnectionRevision,
    preference: Option<StartupTarget>,
    accounts: Vec<ConnectionAccount>,
    bindings: Vec<StoredModelBinding>,
    catalog_seeds: Vec<ConnectionCatalogSeed>,
    encoded: Vec<u8>,
}

impl ConnectionSnapshot {
    #[must_use]
    pub const fn revision(&self) -> &ConnectionRevision {
        &self.revision
    }

    #[must_use]
    pub const fn preference(&self) -> Option<&StartupTarget> {
        self.preference.as_ref()
    }

    #[must_use]
    pub fn accounts(&self) -> &[ConnectionAccount] {
        &self.accounts
    }

    #[must_use]
    pub fn models(&self) -> &[StoredModelBinding] {
        &self.bindings
    }

    #[must_use]
    pub fn catalog_seeds(&self) -> &[ConnectionCatalogSeed] {
        &self.catalog_seeds
    }

    pub fn openrouter_discovery_seed(
        &self,
        provider: &super::ProviderId,
        account: &super::AccountId,
    ) -> Result<Option<super::OpenRouterDiscoverySeed>, ConnectionRepositoryError> {
        self.catalog_seeds
            .iter()
            .find(|seed| seed.provider() == provider && seed.account() == account)
            .map(ConnectionCatalogSeed::openrouter_seed)
            .transpose()
            .map_err(|_| ConnectionRepositoryError::InvalidMutation)
            .map(Option::flatten)
    }

    pub fn qwencloud_catalog_seed(
        &self,
        provider: &super::ProviderId,
        account: &super::AccountId,
    ) -> Result<Option<super::QwenCloudCatalogSeed>, ConnectionRepositoryError> {
        self.catalog_seeds
            .iter()
            .find(|seed| seed.provider() == provider && seed.account() == account)
            .map(ConnectionCatalogSeed::qwencloud_seed)
            .transpose()
            .map_err(|_| ConnectionRepositoryError::InvalidMutation)
            .map(Option::flatten)
    }

    pub fn kimi_catalog_seed(
        &self,
        provider: &super::ProviderId,
        account: &super::AccountId,
    ) -> Result<Option<super::KimiCatalogSeed>, ConnectionRepositoryError> {
        self.catalog_seeds
            .iter()
            .find(|seed| seed.provider() == provider && seed.account() == account)
            .map(ConnectionCatalogSeed::kimi_seed)
            .transpose()
            .map_err(|_| ConnectionRepositoryError::InvalidMutation)
            .map(Option::flatten)
    }

    pub fn model_catalog(&self) -> Result<ModelCatalog, ConnectionRepositoryError> {
        let entries = self
            .bindings
            .iter()
            .map(|binding| {
                let complete = binding.complete().binding();
                let account = self.accounts.iter().find(|account| {
                    account.provider_id() == complete.provider_id()
                        && account.account_id() == complete.account_id()
                });
                let account = account.ok_or(ConnectionRepositoryError::InvalidMutation)?;
                ModelCatalogEntry::from_stored(
                    binding.complete().clone(),
                    account.provider_display_name().map(str::to_owned),
                    account.account_display_name().map(str::to_owned),
                    binding.model_display_name().map(str::to_owned),
                    binding.last_failure().cloned(),
                    binding.is_enabled(),
                )
                .map_err(|_| ConnectionRepositoryError::InvalidMutation)
            })
            .collect::<Result<Vec<_>, _>>()?;
        ModelCatalog::new(entries).map_err(|_| ConnectionRepositoryError::InvalidMutation)
    }

    /// Builds the exact prospective catalog after one stored upsert.
    pub fn catalog_after_model_upsert(
        &self,
        account: ConnectionAccount,
        binding: StoredModelBinding,
    ) -> Result<ModelCatalog, ConnectionRepositoryError> {
        let (preference, accounts, bindings, catalog_seeds) =
            self.model_upsert_state(account, binding)?;
        Self {
            revision: self.revision.clone(),
            preference,
            accounts,
            bindings,
            catalog_seeds,
            encoded: Vec::new(),
        }
        .model_catalog()
    }

    /// Builds the exact prospective catalog after one stored removal.
    pub fn catalog_after_model_remove(
        &self,
        selection: &ModelSelection,
    ) -> Result<ModelCatalog, ConnectionRepositoryError> {
        let (preference, accounts, bindings, catalog_seeds) = self.model_remove_state(selection)?;
        Self {
            revision: self.revision.clone(),
            preference,
            accounts,
            bindings,
            catalog_seeds,
            encoded: Vec::new(),
        }
        .model_catalog()
    }

    pub(crate) fn matches_planned(&self, mutation: &PreparedConnectionMutation) -> bool {
        self.revision == mutation.planned_revision && self.encoded == mutation.planned_bytes
    }

    /// Prepares exact old-or-new bytes without changing the repository.
    pub fn prepare_preference(
        &self,
        preference: Option<StartupTarget>,
    ) -> Result<Option<PreparedConnectionMutation>, ConnectionRepositoryError> {
        if self.preference == preference {
            return Ok(None);
        }
        self.prepare_snapshot(
            preference,
            self.accounts.clone(),
            self.bindings.clone(),
            self.catalog_seeds.clone(),
        )
    }

    /// Adds or replaces one stored model while preserving unrelated public state.
    pub fn prepare_model_upsert(
        &self,
        account: ConnectionAccount,
        binding: StoredModelBinding,
    ) -> Result<Option<PreparedConnectionMutation>, ConnectionRepositoryError> {
        let (preference, accounts, bindings, catalog_seeds) =
            self.model_upsert_state(account, binding)?;
        self.prepare_snapshot(preference, accounts, bindings, catalog_seeds)
    }

    /// Prepares an external connect epoch even when its public binding is unchanged.
    ///
    /// Credential replacement recovery needs an exact prospective public revision. Rotating a
    /// key therefore receives a new public revision while preserving semantically equal state.
    pub fn prepare_model_connect(
        &self,
        account: ConnectionAccount,
        binding: StoredModelBinding,
    ) -> Result<PreparedConnectionMutation, ConnectionRepositoryError> {
        let direct_connect = DirectConnectIntent {
            account: account.clone(),
            binding: binding.clone(),
        };
        let (preference, accounts, bindings, catalog_seeds) =
            self.model_upsert_state(account, binding)?;
        let mut mutation = self
            .prepare_snapshot_with_mode(preference, accounts, bindings, catalog_seeds, true)?
            .ok_or(ConnectionRepositoryError::InvalidMutation)?;
        mutation.direct_connect = Some(direct_connect);
        Ok(mutation)
    }

    fn model_upsert_state(
        &self,
        account: ConnectionAccount,
        mut binding: StoredModelBinding,
    ) -> Result<StoredSnapshotState, ConnectionRepositoryError> {
        if !stored::account_matches_binding(&account, &binding) {
            return Err(ConnectionRepositoryError::CoordinateMismatch);
        }
        let mut accounts = self.accounts.clone();
        let account_position = accounts.iter().position(|current| {
            current.provider_id() == account.provider_id()
                && current.account_id() == account.account_id()
        });
        match account_position {
            Some(index) => accounts[index] = account,
            None => accounts.push(account),
        }

        let mut bindings = self.bindings.clone();
        let selection = binding.selection();
        let binding_position = bindings
            .iter()
            .position(|current| stored::binding_matches_selection(current, &selection));
        let inserted = binding_position.is_none();
        match binding_position {
            Some(index) => {
                if bindings[index].complete() == binding.complete() {
                    binding = binding.with_enabled(bindings[index].is_enabled());
                }
                bindings[index] = binding;
            },
            None => bindings.push(binding),
        }
        stored::validate_state(&accounts, &bindings)
            .map_err(|_| ConnectionRepositoryError::InvalidMutation)?;
        let preference = self
            .preference
            .clone()
            .or_else(|| inserted.then_some(StartupTarget::Model(selection)));
        Ok((preference, accounts, bindings, self.catalog_seeds.clone()))
    }

    /// Replaces one complete Provider-and-Account definition as one public revision.
    pub fn prepare_group_replace(
        &self,
        account: ConnectionAccount,
        mut replacement_bindings: Vec<StoredModelBinding>,
        replacement_seed: Option<ConnectionCatalogSeed>,
    ) -> Result<PreparedConnectionMutation, ConnectionRepositoryError> {
        if replacement_bindings
            .iter()
            .any(|binding| !stored::account_matches_binding(&account, binding))
            || replacement_seed.as_ref().is_some_and(|seed| {
                seed.provider() != account.provider_id() || seed.account() != account.account_id()
            })
        {
            return Err(ConnectionRepositoryError::CoordinateMismatch);
        }
        if replacement_bindings.is_empty() && replacement_seed.is_none() {
            return Err(ConnectionRepositoryError::InvalidMutation);
        }

        let group_replacement = GroupReplacementIntent {
            account: account.clone(),
            bindings: replacement_bindings.clone(),
            catalog_seed: replacement_seed.clone(),
        };

        let provider = account.provider_id().clone();
        let account_id = account.account_id().clone();
        for replacement in &mut replacement_bindings {
            if let Some(retained) = self.bindings.iter().find(|current| {
                current.selection() == replacement.selection()
                    && current.complete() == replacement.complete()
            }) {
                *replacement = replacement
                    .clone()
                    .with_enabled(retained.is_enabled())
                    .with_last_failure(retained.last_failure().cloned());
            }
        }
        let mut accounts = self.accounts.clone();
        accounts.retain(|current| {
            current.provider_id() != &provider || current.account_id() != &account_id
        });
        accounts.push(account);
        let mut bindings = self.bindings.clone();
        bindings.retain(|current| {
            let binding = current.complete().binding();
            binding.provider_id() != &provider || binding.account_id() != &account_id
        });
        bindings.extend(replacement_bindings);
        let mut catalog_seeds = self.catalog_seeds.clone();
        catalog_seeds.retain(|seed| seed.provider() != &provider || seed.account() != &account_id);
        catalog_seeds.extend(replacement_seed);
        stored::validate_state(&accounts, &bindings)
            .map_err(|_| ConnectionRepositoryError::InvalidMutation)?;
        validate_catalog_seeds(&accounts, &catalog_seeds)?;

        let preference = match self.preference.as_ref() {
            Some(StartupTarget::Model(selection))
                if selection.provider() == &provider && selection.account() == &account_id =>
            {
                bindings
                    .iter()
                    .any(|binding| binding.selection() == *selection)
                    .then(|| StartupTarget::Model(selection.clone()))
            },
            _ => self.preference.clone(),
        };
        let mut mutation = self
            .prepare_snapshot_with_mode(preference, accounts, bindings, catalog_seeds, true)?
            .ok_or(ConnectionRepositoryError::InvalidMutation)?;
        mutation.group_replacement = Some(group_replacement);
        Ok(mutation)
    }

    /// Prepares one warning-only observation update for an exact current complete binding.
    pub fn prepare_model_observation(
        &self,
        selection: &ModelSelection,
        expected_binding: &super::CompleteModelBinding,
        last_failure: Option<ModelLastFailure>,
    ) -> Result<Option<PreparedConnectionMutation>, ConnectionRepositoryError> {
        let Some(index) = self
            .bindings
            .iter()
            .position(|binding| stored::binding_matches_selection(binding, selection))
        else {
            return Ok(None);
        };
        let current = &self.bindings[index];
        if current.complete() != expected_binding || current.last_failure() == last_failure.as_ref()
        {
            return Ok(None);
        }
        let mut bindings = self.bindings.clone();
        bindings[index] = current.clone().with_last_failure(last_failure);
        self.prepare_snapshot(
            self.preference.clone(),
            self.accounts.clone(),
            bindings,
            self.catalog_seeds.clone(),
        )
    }

    /// Enables or disables one exact stored binding without changing its complete identity.
    pub fn prepare_model_activation(
        &self,
        selection: &ModelSelection,
        enabled: bool,
    ) -> Result<Option<PreparedConnectionMutation>, ConnectionRepositoryError> {
        let Some(index) = self
            .bindings
            .iter()
            .position(|binding| stored::binding_matches_selection(binding, selection))
        else {
            return Err(ConnectionRepositoryError::ModelNotFound {
                provider: selection.provider().to_string(),
                account: selection.account().to_string(),
                model: selection.model().to_string(),
            });
        };
        if self.bindings[index].is_enabled() == enabled {
            return Ok(None);
        }
        let mut bindings = self.bindings.clone();
        bindings[index] = bindings[index].clone().with_enabled(enabled);
        let disabled_target = StartupTarget::Model(selection.clone());
        let preference = if !enabled && self.preference.as_ref() == Some(&disabled_target) {
            None
        } else {
            self.preference.clone()
        };
        self.prepare_snapshot(
            preference,
            self.accounts.clone(),
            bindings,
            self.catalog_seeds.clone(),
        )
    }

    /// Removes one stored model, its unused account, and an exact matching preference.
    pub fn prepare_model_remove(
        &self,
        selection: &ModelSelection,
    ) -> Result<PreparedConnectionMutation, ConnectionRepositoryError> {
        let (preference, accounts, bindings, catalog_seeds) = self.model_remove_state(selection)?;
        self.prepare_snapshot(preference, accounts, bindings, catalog_seeds)?
            .ok_or(ConnectionRepositoryError::InvalidMutation)
    }

    fn model_remove_state(
        &self,
        selection: &ModelSelection,
    ) -> Result<StoredSnapshotState, ConnectionRepositoryError> {
        let mut bindings = self.bindings.clone();
        let Some(index) = bindings
            .iter()
            .position(|binding| stored::binding_matches_selection(binding, selection))
        else {
            return Err(ConnectionRepositoryError::ModelNotFound {
                provider: selection.provider().to_string(),
                account: selection.account().to_string(),
                model: selection.model().to_string(),
            });
        };
        bindings.remove(index);
        let account_is_still_used = bindings.iter().any(|binding| {
            let complete = binding.complete().binding();
            complete.provider_id() == selection.provider()
                && complete.account_id() == selection.account()
        }) || self.catalog_seeds.iter().any(|seed| {
            seed.provider() == selection.provider() && seed.account() == selection.account()
        });
        let mut accounts = self.accounts.clone();
        if !account_is_still_used {
            accounts.retain(|account| {
                account.provider_id() != selection.provider()
                    || account.account_id() != selection.account()
            });
        }
        stored::validate_state(&accounts, &bindings)
            .map_err(|_| ConnectionRepositoryError::InvalidMutation)?;
        let removed_target = StartupTarget::Model(selection.clone());
        let preference = if self.preference.as_ref() == Some(&removed_target) {
            None
        } else {
            self.preference.clone()
        };
        Ok((preference, accounts, bindings, self.catalog_seeds.clone()))
    }

    fn prepare_snapshot(
        &self,
        preference: Option<StartupTarget>,
        accounts: Vec<ConnectionAccount>,
        bindings: Vec<StoredModelBinding>,
        catalog_seeds: Vec<ConnectionCatalogSeed>,
    ) -> Result<Option<PreparedConnectionMutation>, ConnectionRepositoryError> {
        self.prepare_snapshot_with_mode(preference, accounts, bindings, catalog_seeds, false)
    }

    fn prepare_snapshot_with_mode(
        &self,
        preference: Option<StartupTarget>,
        accounts: Vec<ConnectionAccount>,
        bindings: Vec<StoredModelBinding>,
        catalog_seeds: Vec<ConnectionCatalogSeed>,
        force_new_revision: bool,
    ) -> Result<Option<PreparedConnectionMutation>, ConnectionRepositoryError> {
        if !force_new_revision
            && self.preference == preference
            && self.accounts == accounts
            && self.bindings == bindings
            && self.catalog_seeds == catalog_seeds
        {
            return Ok(None);
        }
        let planned_revision = wire::new_revision()?;
        let planned_bytes = wire::encode(
            &planned_revision,
            preference.as_ref(),
            &accounts,
            &bindings,
            &catalog_seeds,
        )?;
        if planned_bytes.len() as u64 > MAX_CONNECTION_BYTES {
            return Err(ConnectionRepositoryError::PreparedTooLarge);
        }
        Ok(Some(PreparedConnectionMutation {
            expected_revision: self.revision.clone(),
            planned_revision,
            planned_bytes,
            preference,
            direct_connect: None,
            group_replacement: None,
        }))
    }
}

pub(super) fn validate_catalog_seeds(
    accounts: &[ConnectionAccount],
    seeds: &[ConnectionCatalogSeed],
) -> Result<(), ConnectionRepositoryError> {
    let mut coordinates = std::collections::HashSet::new();
    for seed in seeds {
        if !coordinates.insert((seed.provider().clone(), seed.account().clone()))
            || !accounts.iter().any(|account| {
                account.provider_id() == seed.provider() && account.account_id() == seed.account()
            })
        {
            return Err(ConnectionRepositoryError::InvalidMutation);
        }
    }
    Ok(())
}

/// One immutable exact public mutation prepared from a captured revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedConnectionMutation {
    expected_revision: ConnectionRevision,
    planned_revision: ConnectionRevision,
    planned_bytes: Vec<u8>,
    preference: Option<StartupTarget>,
    direct_connect: Option<DirectConnectIntent>,
    group_replacement: Option<GroupReplacementIntent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DirectConnectIntent {
    account: ConnectionAccount,
    binding: StoredModelBinding,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GroupReplacementIntent {
    account: ConnectionAccount,
    bindings: Vec<StoredModelBinding>,
    catalog_seed: Option<ConnectionCatalogSeed>,
}

impl PreparedConnectionMutation {
    #[must_use]
    pub const fn expected_revision(&self) -> &ConnectionRevision {
        &self.expected_revision
    }

    #[must_use]
    pub const fn planned_revision(&self) -> &ConnectionRevision {
        &self.planned_revision
    }

    #[must_use]
    pub const fn preference(&self) -> Option<&StartupTarget> {
        self.preference.as_ref()
    }

    pub(crate) fn planned_bytes(&self) -> &[u8] {
        &self.planned_bytes
    }

    pub(crate) fn into_journal_mutation(mut self) -> Self {
        self.direct_connect = None;
        self.group_replacement = None;
        self
    }

    pub(crate) fn defines_model_connect(
        &self,
        provider: &super::ProviderId,
        account: &super::AccountId,
        complete_bindings: &[super::CompleteModelBinding],
    ) -> bool {
        let Some(intent) = self.direct_connect.as_ref() else {
            return false;
        };
        if intent.account.provider_id() != provider
            || intent.account.account_id() != account
            || !complete_bindings.contains(intent.binding.complete())
        {
            return false;
        }
        let Ok(decoded) = wire::decode(Path::new(""), &self.planned_bytes) else {
            return false;
        };
        let stored = decoded
            .bindings
            .iter()
            .filter(|binding| {
                let coordinate = binding.complete().binding();
                coordinate.provider_id() == provider && coordinate.account_id() == account
            })
            .map(StoredModelBinding::complete)
            .collect::<Vec<_>>();
        let same_bindings = stored.len() == complete_bindings.len()
            && stored
                .iter()
                .all(|binding| complete_bindings.contains(binding));
        let pair_exists = decoded
            .accounts
            .iter()
            .find(|stored| stored.provider_id() == provider && stored.account_id() == account)
            == Some(&intent.account);
        let exact_target = decoded
            .bindings
            .iter()
            .find(|stored| stored.selection() == intent.binding.selection())
            == Some(&intent.binding);
        pair_exists && exact_target && same_bindings && !complete_bindings.is_empty()
    }

    pub(crate) fn defines_group_replacement(
        &self,
        provider: &super::ProviderId,
        account: &super::AccountId,
        complete_bindings: &[super::CompleteModelBinding],
    ) -> bool {
        let Some(intent) = self.group_replacement.as_ref() else {
            return false;
        };
        let intended_complete = intent
            .bindings
            .iter()
            .map(StoredModelBinding::complete)
            .collect::<Vec<_>>();
        if intent.account.provider_id() != provider
            || intent.account.account_id() != account
            || intended_complete.len() != complete_bindings.len()
            || !intended_complete
                .iter()
                .all(|binding| complete_bindings.contains(binding))
        {
            return false;
        }
        let Ok(decoded) = wire::decode(Path::new(""), &self.planned_bytes) else {
            return false;
        };
        let stored = decoded
            .bindings
            .iter()
            .filter(|binding| {
                let coordinate = binding.complete().binding();
                coordinate.provider_id() == provider && coordinate.account_id() == account
            })
            .map(StoredModelBinding::complete)
            .collect::<Vec<_>>();
        let same_bindings = stored.len() == complete_bindings.len()
            && stored
                .iter()
                .all(|binding| complete_bindings.contains(binding));
        let exact_account = decoded
            .accounts
            .iter()
            .find(|stored| stored.provider_id() == provider && stored.account_id() == account)
            == Some(&intent.account);
        let exact_stored_bindings = stored.len() == intent.bindings.len()
            && intent.bindings.iter().all(|binding| {
                stored
                    .iter()
                    .any(|complete| *complete == binding.complete())
            });
        let decoded_seed = decoded
            .catalog_seeds
            .iter()
            .find(|seed| seed.provider() == provider && seed.account() == account);
        let exact_seed = decoded_seed == intent.catalog_seed.as_ref();
        let non_empty_group = !complete_bindings.is_empty() || intent.catalog_seed.is_some();
        exact_account && same_bindings && exact_stored_bindings && exact_seed && non_empty_group
    }

    pub(crate) fn from_operation_journal(
        expected_revision: ConnectionRevision,
        planned_revision: ConnectionRevision,
        planned_bytes: Vec<u8>,
    ) -> Result<Self, ConnectionRepositoryError> {
        if planned_revision.is_absent()
            || expected_revision == planned_revision
            || planned_bytes.len() as u64 > MAX_CONNECTION_BYTES
        {
            return Err(ConnectionRepositoryError::InvalidContents(PathBuf::new()));
        }
        let decoded = wire::decode(Path::new(""), &planned_bytes)?;
        if decoded.revision != planned_revision {
            return Err(ConnectionRepositoryError::InvalidContents(PathBuf::new()));
        }
        Ok(Self {
            expected_revision,
            planned_revision,
            planned_bytes,
            preference: decoded.preference,
            direct_connect: None,
            group_replacement: None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionCommit {
    Committed,
    AlreadyCommitted,
}

/// Storage-neutral preference publication boundary used by connection orchestration.
pub trait ConnectionRepository {
    type OperationGuard;

    fn acquire_operation(&self) -> Result<Self::OperationGuard, ConnectionRepositoryError>;
    fn recover_pending_operation(&self) -> Result<(), ConnectionRepositoryError>;
    fn capture(&self) -> Result<ConnectionSnapshot, ConnectionRepositoryError>;
    fn commit(
        &self,
        mutation: &PreparedConnectionMutation,
    ) -> Result<ConnectionCommit, ConnectionRepositoryError>;
}

/// Local bounded `connections.yaml` repository with exact revision CAS.
#[derive(Clone, Debug)]
pub struct LocalConnectionRepository {
    path: PathBuf,
}

impl LocalConnectionRepository {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Captures without creating the file or its parent directory.
    pub fn capture(&self) -> Result<ConnectionSnapshot, ConnectionRepositoryError> {
        read_snapshot(&self.path)
    }

    /// Acquires the process-wide operation lane shared by connection mutations.
    pub fn acquire_operation(
        &self,
    ) -> Result<LocalConnectionOperationGuard, ConnectionRepositoryError> {
        let parent = prepare_parent(&self.path)?;
        let path = parent.join(OPERATION_LOCK_FILE);
        let file = open_lock_file(&path)?;
        match file.try_lock() {
            Ok(()) => Ok(LocalConnectionOperationGuard { file, parent }),
            Err(fs::TryLockError::WouldBlock) => {
                Err(ConnectionRepositoryError::OperationBusy(path))
            },
            Err(fs::TryLockError::Error(source)) => {
                Err(ConnectionRepositoryError::io(&path, source))
            },
        }
    }

    /// Fails closed on a journal from a newer operation implementation.
    pub fn recover_pending_operation(&self) -> Result<(), ConnectionRepositoryError> {
        let Some(parent) = self.path.parent() else {
            return Err(ConnectionRepositoryError::InvalidPath(self.path.clone()));
        };
        let path = parent.join(PENDING_OPERATION_FILE);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Ok(_) => Err(ConnectionRepositoryError::PendingOperation(path)),
            Err(source) => Err(ConnectionRepositoryError::io(&path, source)),
        }
    }

    /// Publishes the exact prepared bytes if the expected revision still owns the path.
    pub fn commit(
        &self,
        mutation: &PreparedConnectionMutation,
    ) -> Result<ConnectionCommit, ConnectionRepositoryError> {
        let parent = prepare_parent(&self.path)?;
        let lock_path = parent.join(REPOSITORY_LOCK_FILE);
        let lock = open_lock_file(&lock_path)?;
        lock.lock()
            .map_err(|source| ConnectionRepositoryError::io(&lock_path, source))?;

        let current = read_snapshot(&self.path)?;
        if current.revision == mutation.planned_revision
            && current.encoded == mutation.planned_bytes
        {
            return Ok(ConnectionCommit::AlreadyCommitted);
        }
        if current.revision != mutation.expected_revision {
            return Err(ConnectionRepositoryError::Conflict {
                expected: mutation.expected_revision.clone(),
                observed: current.revision,
            });
        }

        let (temporary, mut file) = create_connection_temporary(&parent)?;
        let publication = (|| {
            file.write_all(&mutation.planned_bytes)
                .map_err(|source| ConnectionRepositoryError::io(&temporary, source))?;
            file.sync_all()
                .map_err(|source| ConnectionRepositoryError::io(&temporary, source))?;
            if mutation.expected_revision.is_absent() {
                fs::hard_link(&temporary, &self.path)
                    .map_err(|source| ConnectionRepositoryError::io(&self.path, source))?;
                fs::remove_file(&temporary)
                    .map_err(|source| ConnectionRepositoryError::io(&temporary, source))?;
            } else {
                fs::rename(&temporary, &self.path)
                    .map_err(|source| ConnectionRepositoryError::io(&self.path, source))?;
            }
            fs::File::open(&parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|source| ConnectionRepositoryError::io(&parent, source))?;
            Ok(())
        })();
        if publication.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        publication?;
        Ok(ConnectionCommit::Committed)
    }
}

fn create_connection_temporary(
    parent: &Path,
) -> Result<(PathBuf, fs::File), ConnectionRepositoryError> {
    create_connection_temporary_with(parent, CONNECTION_TEMPORARY_ATTEMPTS, || {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|error| error.to_string())?;
        Ok(random)
    })
}

fn create_connection_temporary_with(
    parent: &Path,
    attempt_limit: usize,
    mut next_candidate: impl FnMut() -> Result<[u8; 16], String>,
) -> Result<(PathBuf, fs::File), ConnectionRepositoryError> {
    for _ in 0..attempt_limit {
        let random =
            next_candidate().map_err(ConnectionRepositoryError::TemporaryNameRandomness)?;
        let temporary = connection_temporary_path(parent, random);
        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(FILE_MODE)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {},
            Err(source) => return Err(ConnectionRepositoryError::io(&temporary, source)),
        }
    }
    Err(
        ConnectionRepositoryError::TemporaryNameCollisionExhaustion {
            attempts: attempt_limit,
        },
    )
}

fn connection_temporary_path(parent: &Path, random: [u8; 16]) -> PathBuf {
    let mut suffix = String::with_capacity(32);
    for byte in random {
        use fmt::Write as _;
        write!(suffix, "{byte:02x}").expect("formatting into a String cannot fail");
    }
    parent.join(format!(".connections.{suffix}.pending"))
}

#[cfg(test)]
pub(super) fn create_connection_temporary_for_test(
    parent: &Path,
    attempt_limit: usize,
    next_candidate: impl FnMut() -> Result<[u8; 16], String>,
) -> Result<(PathBuf, fs::File), ConnectionRepositoryError> {
    create_connection_temporary_with(parent, attempt_limit, next_candidate)
}

#[cfg(test)]
pub(super) fn connection_temporary_path_for_test(parent: &Path, random: [u8; 16]) -> PathBuf {
    connection_temporary_path(parent, random)
}

#[cfg(test)]
pub(super) const CONNECTION_TEMPORARY_ATTEMPTS_FOR_TEST: usize = CONNECTION_TEMPORARY_ATTEMPTS;

impl ConnectionRepository for LocalConnectionRepository {
    type OperationGuard = LocalConnectionOperationGuard;

    fn acquire_operation(&self) -> Result<Self::OperationGuard, ConnectionRepositoryError> {
        Self::acquire_operation(self)
    }

    fn recover_pending_operation(&self) -> Result<(), ConnectionRepositoryError> {
        Self::recover_pending_operation(self)
    }

    fn capture(&self) -> Result<ConnectionSnapshot, ConnectionRepositoryError> {
        Self::capture(self)
    }

    fn commit(
        &self,
        mutation: &PreparedConnectionMutation,
    ) -> Result<ConnectionCommit, ConnectionRepositoryError> {
        Self::commit(self, mutation)
    }
}

#[derive(Debug)]
pub struct LocalConnectionOperationGuard {
    file: fs::File,
    parent: PathBuf,
}

impl LocalConnectionOperationGuard {
    pub(crate) fn authorizes(&self, journal_path: &Path) -> bool {
        journal_path.parent() == Some(self.parent.as_path())
    }
}

impl Drop for LocalConnectionOperationGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn read_snapshot(path: &Path) -> Result<ConnectionSnapshot, ConnectionRepositoryError> {
    let mut file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ConnectionSnapshot {
                revision: ConnectionRevision::Absent,
                preference: None,
                accounts: Vec::new(),
                bindings: Vec::new(),
                catalog_seeds: Vec::new(),
                encoded: Vec::new(),
            });
        },
        Err(source) => return Err(ConnectionRepositoryError::io(path, source)),
    };
    let before = MetadataSnapshot::capture(path, &file)?;
    before.validate(path)?;
    let mut encoded = Vec::with_capacity(
        usize::try_from(before.len.min(MAX_CONNECTION_BYTES))
            .unwrap_or(MAX_CONNECTION_BYTES as usize),
    );
    Read::by_ref(&mut file)
        .take(MAX_CONNECTION_BYTES + 1)
        .read_to_end(&mut encoded)
        .map_err(|source| ConnectionRepositoryError::io(path, source))?;
    if encoded.len() as u64 > MAX_CONNECTION_BYTES {
        return Err(ConnectionRepositoryError::TooLarge(path.to_owned()));
    }
    let after = MetadataSnapshot::capture(path, &file)?;
    if before != after {
        return Err(ConnectionRepositoryError::Changed(path.to_owned()));
    }
    let decoded = wire::decode(path, &encoded)?;
    Ok(ConnectionSnapshot {
        revision: decoded.revision,
        preference: decoded.preference,
        accounts: decoded.accounts,
        bindings: decoded.bindings,
        catalog_seeds: decoded.catalog_seeds,
        encoded,
    })
}

fn prepare_parent(path: &Path) -> Result<PathBuf, ConnectionRepositoryError> {
    let parent = path
        .parent()
        .ok_or_else(|| ConnectionRepositoryError::InvalidPath(path.to_owned()))?;
    if let Ok(metadata) = fs::symlink_metadata(parent)
        && metadata.file_type().is_symlink()
    {
        return Err(ConnectionRepositoryError::UnsupportedFileType(
            parent.to_owned(),
        ));
    }
    let existed = parent.exists();
    fs::create_dir_all(parent).map_err(|source| ConnectionRepositoryError::io(parent, source))?;
    if !existed {
        fs::set_permissions(parent, fs::Permissions::from_mode(DIRECTORY_MODE))
            .map_err(|source| ConnectionRepositoryError::io(parent, source))?;
    }
    Ok(parent.to_owned())
}

fn open_lock_file(path: &Path) -> Result<fs::File, ConnectionRepositoryError> {
    reject_symlink(path)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(FILE_MODE)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|source| ConnectionRepositoryError::io(path, source))?;
    let metadata = MetadataSnapshot::capture(path, &file)?;
    metadata.validate(path)?;
    Ok(file)
}

fn reject_symlink(path: &Path) -> Result<(), ConnectionRepositoryError> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && metadata.file_type().is_symlink()
    {
        return Err(ConnectionRepositoryError::UnsupportedFileType(
            path.to_owned(),
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MetadataSnapshot {
    device: u64,
    inode: u64,
    mode: u32,
    user: u32,
    group: u32,
    len: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

impl MetadataSnapshot {
    fn capture(path: &Path, file: &fs::File) -> Result<Self, ConnectionRepositoryError> {
        let metadata = file
            .metadata()
            .map_err(|source| ConnectionRepositoryError::io(path, source))?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            user: metadata.uid(),
            group: metadata.gid(),
            len: metadata.len(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        })
    }

    fn validate(&self, path: &Path) -> Result<(), ConnectionRepositoryError> {
        if self.mode & FILE_TYPE_MASK != REGULAR_FILE_MODE {
            return Err(ConnectionRepositoryError::UnsupportedFileType(
                path.to_owned(),
            ));
        }
        if self.user != rustix::process::geteuid().as_raw() {
            return Err(ConnectionRepositoryError::WrongOwner(path.to_owned()));
        }
        if self.mode & 0o077 != 0 {
            return Err(ConnectionRepositoryError::InsecurePermissions(
                path.to_owned(),
            ));
        }
        if self.len > MAX_CONNECTION_BYTES {
            return Err(ConnectionRepositoryError::TooLarge(path.to_owned()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
