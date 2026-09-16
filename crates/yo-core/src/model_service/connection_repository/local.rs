use std::{
    fmt, fs,
    io::{ErrorKind, Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use rustix::process;

use super::{
    CONNECTION_TEMPORARY_ATTEMPTS, ConnectionAccount, ConnectionCatalogSeed, ConnectionCommit,
    ConnectionRepository, ConnectionRepositoryError, ConnectionRevision, ConnectionSnapshot,
    DIRECTORY_MODE, DecodedSnapshot, FILE_MODE, FILE_TYPE_MASK, MAX_CONNECTION_BYTES,
    ModelLastFailure, OPERATION_LOCK_FILE, PENDING_OPERATION_FILE, PreparedConnectionMutation,
    REGULAR_FILE_MODE, REPOSITORY_LOCK_FILE, StoredModelBinding,
};
use crate::StartupTarget;

pub(super) fn encode_snapshot(
    revision: &ConnectionRevision,
    preference: Option<&StartupTarget>,
    accounts: &[ConnectionAccount],
    bindings: &[StoredModelBinding],
    catalog_seeds: &[ConnectionCatalogSeed],
) -> Result<Vec<u8>, ConnectionRepositoryError> {
    wire::encode(revision, preference, accounts, bindings, catalog_seeds)
}

pub(super) fn decode_snapshot(
    path: &Path,
    encoded: &[u8],
) -> Result<DecodedSnapshot, ConnectionRepositoryError> {
    wire::decode(path, encoded)
}

pub(super) fn new_revision() -> Result<ConnectionRevision, ConnectionRepositoryError> {
    wire::new_revision()
}

pub(super) fn parse_revision_token(revision: &str) -> Option<String> {
    wire::parse_revision_token(revision)
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
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
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
            Err(source) if source.kind() == ErrorKind::AlreadyExists => {},
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
        Err(error) if error.kind() == ErrorKind::NotFound => {
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
    let decoded = super::decode_snapshot(path, &encoded)?;
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
        if self.user != process::geteuid().as_raw() {
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

mod wire {
    use std::{
        fmt,
        path::{Path, PathBuf},
        str,
    };

    use serde::{
        Deserialize, Serialize,
        de::{Error, Unexpected},
    };

    use super::super::{
        CatalogSource, ConnectionAccount, ConnectionCatalogSeed, ConnectionRepositoryError,
        ConnectionRevision, DecodedSnapshot, ModelLastFailure, ModelRequestFailureKind,
        StoredModelBinding, validate_catalog_seeds, validate_state,
    };
    use crate::{
        AccountId, CompleteModelBinding, ConnectorId, EffectiveModelBinding, EffectiveModelProfile,
        HostId, ModelId, ModelProfileLayer, ModelProfileParameters, ModelSelection,
        NormalizedEndpoint, ProviderId, SEMANTIC_REPLAY_PROFILE, StartupTarget, VersionedProfileId,
    };

    pub(super) fn encode(
        revision: &ConnectionRevision,
        preference: Option<&StartupTarget>,
        accounts: &[ConnectionAccount],
        bindings: &[StoredModelBinding],
        catalog_seeds: &[ConnectionCatalogSeed],
    ) -> Result<Vec<u8>, ConnectionRepositoryError> {
        yo_yaml::to_string(&WireSnapshot {
            revision: revision.to_string(),
            preference: preference.map(WireTarget::from),
            bindings: bindings.iter().map(WireBinding::from).collect(),
            accounts: accounts.iter().map(WireAccount::from).collect(),
            catalogs: catalog_seeds.iter().map(WireCatalog::from).collect(),
        })
        .map(String::into_bytes)
        .map_err(|_| ConnectionRepositoryError::InvalidContents(PathBuf::new()))
    }

    pub(super) fn decode(
        path: &Path,
        encoded: &[u8],
    ) -> Result<DecodedSnapshot, ConnectionRepositoryError> {
        let contents = str::from_utf8(encoded)
            .map_err(|_| ConnectionRepositoryError::InvalidContents(path.to_owned()))?;
        let wire: WireSnapshot = yo_yaml::from_str(contents)
            .map_err(|_| ConnectionRepositoryError::InvalidContents(path.to_owned()))?;
        let invalid = |_| ConnectionRepositoryError::InvalidContents(path.to_owned());
        let accounts = wire
            .accounts
            .into_iter()
            .map(|account| parse_account(account).map_err(invalid))
            .collect::<Result<Vec<_>, _>>()?;
        let bindings = wire
            .bindings
            .into_iter()
            .map(|binding| parse_binding(binding).map_err(invalid))
            .collect::<Result<Vec<_>, _>>()?;
        let catalog_seeds = wire
            .catalogs
            .into_iter()
            .map(|seed| parse_catalog(seed, &accounts).map_err(invalid))
            .collect::<Result<Vec<_>, _>>()?;
        validate_state(&accounts, &bindings).map_err(invalid)?;
        validate_catalog_seeds(&accounts, &catalog_seeds)?;
        Ok(DecodedSnapshot {
            revision: parse_revision(path, &wire.revision)?,
            preference: wire
                .preference
                .map(|target| parse_target(path, target))
                .transpose()?,
            accounts,
            bindings,
            catalog_seeds,
        })
    }

    pub(super) fn new_revision() -> Result<ConnectionRevision, ConnectionRepositoryError> {
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes)
            .map_err(|error| ConnectionRepositoryError::Randomness(error.to_string()))?;
        let mut token = String::with_capacity(36);
        token.push_str("rev-");
        for byte in bytes {
            use fmt::Write as _;
            write!(token, "{byte:02x}").expect("formatting into a String cannot fail");
        }
        Ok(ConnectionRevision::Token(token))
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct WireSnapshot {
        revision: String,
        #[serde(
            default,
            deserialize_with = "deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        preference: Option<WireTarget>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        bindings: Vec<WireBinding>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        accounts: Vec<WireAccount>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        catalogs: Vec<WireCatalog>,
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
    enum WireCatalog {
        OpenrouterDiscovery {
            provider: String,
            account: String,
            base_url: String,
            profile: Box<WireProfile>,
        },
        BuiltIn {
            provider: String,
            account: String,
            catalog: String,
        },
    }

    impl From<&ConnectionCatalogSeed> for WireCatalog {
        fn from(seed: &ConnectionCatalogSeed) -> Self {
            match seed.source() {
                CatalogSource::Discovery { endpoint, profile } => Self::OpenrouterDiscovery {
                    provider: seed.provider().as_str().to_owned(),
                    account: seed.account().as_str().to_owned(),
                    base_url: endpoint.as_str().to_owned(),
                    profile: Box::new(WireProfile::from(profile.as_ref())),
                },
                CatalogSource::BuiltIn { catalog } => Self::BuiltIn {
                    provider: seed.provider().as_str().to_owned(),
                    account: seed.account().as_str().to_owned(),
                    catalog: catalog.as_str().to_owned(),
                },
            }
        }
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct WireAccount {
        provider: String,
        #[serde(
            default,
            deserialize_with = "deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        provider_display_name: Option<String>,
        account: String,
        #[serde(
            default,
            deserialize_with = "deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        account_display_name: Option<String>,
    }

    impl From<&ConnectionAccount> for WireAccount {
        fn from(account: &ConnectionAccount) -> Self {
            Self {
                provider: account.provider_id().as_str().to_owned(),
                provider_display_name: account.provider_display_name().map(str::to_owned),
                account: account.account_id().as_str().to_owned(),
                account_display_name: account.account_display_name().map(str::to_owned),
            }
        }
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct WireBinding {
        provider: String,
        account: String,
        model: String,
        #[serde(
            default,
            deserialize_with = "deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        model_display_name: Option<String>,
        connector: String,
        base_url: String,
        profile: WireProfile,
        #[serde(
            default,
            deserialize_with = "deserialize_disabled_activation",
            skip_serializing_if = "Option::is_none"
        )]
        enabled: Option<bool>,
        #[serde(
            default,
            deserialize_with = "deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        last_failure: Option<WireLastFailure>,
    }

    impl From<&StoredModelBinding> for WireBinding {
        fn from(stored: &StoredModelBinding) -> Self {
            let complete = stored.complete();
            let binding = complete.binding();
            let profile = complete.profile();
            Self {
                provider: binding.provider_id().as_str().to_owned(),
                account: binding.account_id().as_str().to_owned(),
                model: binding.model_id().as_str().to_owned(),
                model_display_name: stored.model_display_name().map(str::to_owned),
                connector: binding.connector_id().as_str().to_owned(),
                base_url: binding.endpoint().as_str().to_owned(),
                profile: WireProfile::from(profile),
                enabled: (!stored.is_enabled()).then_some(false),
                last_failure: stored.last_failure().map(WireLastFailure::from),
            }
        }
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct WireLastFailure {
        kind: String,
        observed_at: String,
    }

    impl From<&ModelLastFailure> for WireLastFailure {
        fn from(failure: &ModelLastFailure) -> Self {
            Self {
                kind: failure.kind().as_str().to_owned(),
                observed_at: failure.observed_at().to_owned(),
            }
        }
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct WireProfile {
        api_dialect: String,
        tokenizer_profile: String,
        input_token_limit: u64,
        #[serde(
            default,
            deserialize_with = "deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        max_output_tokens: Option<u64>,
        #[serde(deserialize_with = "deserialize_non_null_profile_parameters")]
        reasoning_parameters: ModelProfileParameters,
        #[serde(deserialize_with = "deserialize_non_null_profile_parameters")]
        optional_request_parameters: ModelProfileParameters,
        tool_capability_policy: String,
        #[serde(
            default,
            deserialize_with = "deserialize_optional_replay_profile",
            skip_serializing_if = "Option::is_none"
        )]
        replay_profile: Option<String>,
        #[serde(
            default,
            deserialize_with = "deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        image_input_profile: Option<String>,
    }

    impl From<&EffectiveModelProfile> for WireProfile {
        fn from(profile: &EffectiveModelProfile) -> Self {
            Self {
                api_dialect: profile.api_dialect().as_str().to_owned(),
                tokenizer_profile: profile.context().tokenizer_profile().to_owned(),
                input_token_limit: profile.context().input_token_limit(),
                max_output_tokens: profile.context().max_output_tokens(),
                reasoning_parameters: profile.reasoning_parameters().clone(),
                optional_request_parameters: profile.optional_request_parameters().clone(),
                tool_capability_policy: profile.tool_capability_policy().as_str().to_owned(),
                replay_profile: (profile.replay_profile().as_str() != SEMANTIC_REPLAY_PROFILE)
                    .then(|| profile.replay_profile().as_str().to_owned()),
                image_input_profile: profile
                    .image_input_profile()
                    .map(|profile| profile.as_str().to_owned()),
            }
        }
    }

    fn deserialize_non_null_profile_parameters<'de, D>(
        deserializer: D,
    ) -> Result<ModelProfileParameters, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Option::<ModelProfileParameters>::deserialize(deserializer)?
            .ok_or_else(|| Error::invalid_type(Unexpected::Unit, &"a structured profile value"))
    }

    fn deserialize_optional_non_null<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
    where
        D: serde::Deserializer<'de>,
        T: Deserialize<'de>,
    {
        T::deserialize(deserializer).map(Some)
    }

    fn deserialize_optional_replay_profile<'de, D>(
        deserializer: D,
    ) -> Result<Option<String>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if matches!(
            value.as_str(),
            SEMANTIC_REPLAY_PROFILE | crate::KIMI_PRIVATE_REPLAY_PROFILE
        ) {
            Ok(Some(value))
        } else {
            Err(Error::custom(
                "present replay_profile is outside the closed supported set",
            ))
        }
    }

    fn deserialize_disabled_activation<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let enabled = bool::deserialize(deserializer)?;
        if enabled {
            Err(Error::custom("present enabled must be exact boolean false"))
        } else {
            Ok(Some(false))
        }
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
    enum WireTarget {
        Host {
            target: String,
        },
        Model {
            provider: String,
            account: String,
            model: String,
        },
    }

    impl From<&StartupTarget> for WireTarget {
        fn from(target: &StartupTarget) -> Self {
            match target {
                StartupTarget::Host(host) => Self::Host {
                    target: host.reference(),
                },
                StartupTarget::Model(selection) => Self::Model {
                    provider: selection.provider().as_str().to_owned(),
                    account: selection.account().as_str().to_owned(),
                    model: selection.model().as_str().to_owned(),
                },
            }
        }
    }

    fn parse_revision(
        path: &Path,
        revision: &str,
    ) -> Result<ConnectionRevision, ConnectionRepositoryError> {
        parse_revision_token(revision)
            .map(ConnectionRevision::Token)
            .ok_or_else(|| ConnectionRepositoryError::InvalidContents(path.to_owned()))
    }

    pub(super) fn parse_revision_token(revision: &str) -> Option<String> {
        let valid = revision.len() == 36
            && revision.starts_with("rev-")
            && revision[4..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
        valid.then(|| revision.to_owned())
    }

    fn parse_target(
        path: &Path,
        target: WireTarget,
    ) -> Result<StartupTarget, ConnectionRepositoryError> {
        let invalid = |_| ConnectionRepositoryError::InvalidContents(path.to_owned());
        match target {
            WireTarget::Host { target } => HostId::from_reference(&target)
                .map_err(invalid)?
                .map(StartupTarget::Host)
                .ok_or_else(|| ConnectionRepositoryError::InvalidContents(path.to_owned())),
            WireTarget::Model {
                provider,
                account,
                model,
            } => Ok(StartupTarget::Model(ModelSelection::new(
                ProviderId::new(provider).map_err(invalid)?,
                AccountId::new(account).map_err(invalid)?,
                ModelId::new(model).map_err(invalid)?,
            ))),
        }
    }

    fn parse_account(account: WireAccount) -> Result<ConnectionAccount, crate::ModelServiceError> {
        ConnectionAccount::from_durable(
            ProviderId::new(account.provider)?,
            AccountId::new(account.account)?,
            account.provider_display_name,
            account.account_display_name,
        )
    }

    fn parse_binding(binding: WireBinding) -> Result<StoredModelBinding, crate::ModelServiceError> {
        let WireBinding {
            provider,
            account,
            model,
            model_display_name,
            connector,
            base_url,
            profile,
            enabled,
            last_failure,
        } = binding;
        let dialect = profile.api_dialect.parse()?;
        let effective = EffectiveModelBinding::from_durable(
            ProviderId::new(provider)?,
            AccountId::new(account)?,
            ModelId::new(model)?,
            ConnectorId::new(connector)?,
            dialect,
            NormalizedEndpoint::parse(&base_url)?,
        )?;
        let profile = parse_profile(profile)?;
        let last_failure = last_failure
            .map(|failure| {
                let kind = ModelRequestFailureKind::parse(&failure.kind).ok_or_else(|| {
                    crate::ModelServiceError::new("stored model last_failure kind is unsupported")
                })?;
                ModelLastFailure::new(kind, failure.observed_at)
            })
            .transpose()?;
        StoredModelBinding::from_durable_with_state(
            CompleteModelBinding::new(effective, profile)?,
            model_display_name,
            enabled.unwrap_or(true),
            last_failure,
        )
    }

    fn parse_profile(
        profile: WireProfile,
    ) -> Result<EffectiveModelProfile, crate::ModelServiceError> {
        let dialect = profile.api_dialect.parse()?;
        let layer = ModelProfileLayer::new(
            Some(dialect),
            Some(VersionedProfileId::new(profile.tokenizer_profile)?),
            Some(profile.input_token_limit),
            profile.max_output_tokens,
            Some(profile.reasoning_parameters),
            Some(profile.optional_request_parameters),
            Some(VersionedProfileId::new(profile.tool_capability_policy)?),
        )
        .with_replay_profile(
            profile
                .replay_profile
                .map(VersionedProfileId::new)
                .transpose()?,
        )
        .with_image_input_profile(
            profile
                .image_input_profile
                .map(VersionedProfileId::new)
                .transpose()?,
        );
        EffectiveModelProfile::resolve(None, &layer)
    }

    fn parse_catalog(
        seed: WireCatalog,
        accounts: &[ConnectionAccount],
    ) -> Result<ConnectionCatalogSeed, crate::ModelServiceError> {
        match seed {
            WireCatalog::OpenrouterDiscovery {
                provider,
                account,
                base_url,
                profile,
            } => {
                let provider = ProviderId::new(provider)?;
                let account = AccountId::new(account)?;
                let metadata = catalog_account(accounts, &provider, &account)?;
                ConnectionCatalogSeed::discovery(
                    provider,
                    account,
                    metadata.provider_display_name().map(str::to_owned),
                    metadata.account_display_name().map(str::to_owned),
                    NormalizedEndpoint::parse(&base_url)?,
                    parse_profile(*profile)?,
                )
            },
            WireCatalog::BuiltIn {
                provider,
                account,
                catalog,
            } => {
                let provider = ProviderId::new(provider)?;
                let account = AccountId::new(account)?;
                let metadata = catalog_account(accounts, &provider, &account)?;
                ConnectionCatalogSeed::built_in(
                    VersionedProfileId::new(catalog)?,
                    provider,
                    account,
                    metadata.provider_display_name().map(str::to_owned),
                    metadata.account_display_name().map(str::to_owned),
                )
            },
        }
    }

    fn catalog_account<'a>(
        accounts: &'a [ConnectionAccount],
        provider: &ProviderId,
        account: &AccountId,
    ) -> Result<&'a ConnectionAccount, crate::ModelServiceError> {
        accounts
            .iter()
            .find(|candidate| {
                candidate.provider_id() == provider && candidate.account_id() == account
            })
            .ok_or_else(|| crate::ModelServiceError::new("catalog seed has no stored account"))
    }
}
