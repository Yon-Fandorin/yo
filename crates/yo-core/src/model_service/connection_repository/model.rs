use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    error::Error,
    fmt::{self, Display, Formatter, Result as FmtResult},
    io::Error as IoError,
    path::{Path, PathBuf},
};

use super::{super::catalog::validate_display_name, MAX_CONNECTION_BYTES};
use crate::{
    AccountId, CompleteModelBinding, EffectiveModelProfile, ModelCatalog, ModelCatalogEntry,
    ModelSelection, ModelServiceError, NormalizedEndpoint, ProviderId, StartupTarget,
    VersionedProfileId,
};

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
        super::parse_revision_token(value).map(Self::Token)
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
    pub(super) revision: ConnectionRevision,
    pub(super) preference: Option<StartupTarget>,
    pub(super) accounts: Vec<ConnectionAccount>,
    pub(super) bindings: Vec<StoredModelBinding>,
    pub(super) catalog_seeds: Vec<ConnectionCatalogSeed>,
    pub(super) encoded: Vec<u8>,
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

    /// Returns only the exact Provider-and-Account persisted seed, without service resolution.
    pub fn catalog_seed(
        &self,
        provider: &crate::ProviderId,
        account: &crate::AccountId,
    ) -> Option<&ConnectionCatalogSeed> {
        self.catalog_seeds
            .iter()
            .find(|seed| seed.provider() == provider && seed.account() == account)
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
}

/// Decoded connection state shared by mutation planning and local publication.
#[derive(Clone, Debug)]
pub(super) struct DecodedSnapshot {
    pub(super) revision: ConnectionRevision,
    pub(super) preference: Option<crate::StartupTarget>,
    pub(super) accounts: Vec<ConnectionAccount>,
    pub(super) bindings: Vec<StoredModelBinding>,
    pub(super) catalog_seeds: Vec<ConnectionCatalogSeed>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionCatalogSeed {
    provider: ProviderId,
    account: AccountId,
    provider_display_name: Option<String>,
    account_display_name: Option<String>,
    source: CatalogSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CatalogSource {
    Discovery {
        endpoint: NormalizedEndpoint,
        profile: Box<EffectiveModelProfile>,
    },
    BuiltIn {
        catalog: VersionedProfileId,
    },
}

impl ConnectionCatalogSeed {
    pub fn discovery(
        provider: ProviderId,
        account: AccountId,
        provider_display_name: Option<String>,
        account_display_name: Option<String>,
        endpoint: NormalizedEndpoint,
        profile: EffectiveModelProfile,
    ) -> Result<Self, ModelServiceError> {
        // This neutral descriptor still encodes the historical openrouter_discovery
        // wire kind; reject a mismatched durable identity before it is representable.
        if provider.as_str() != "openrouter" {
            return Err(ModelServiceError::new(
                "OpenRouter discovery seed requires ProviderId openrouter",
            ));
        }
        crate::ConnectionAccount::new(
            provider.clone(),
            account.clone(),
            provider_display_name.clone(),
            account_display_name.clone(),
        )?;
        Ok(Self {
            provider,
            account,
            provider_display_name,
            account_display_name,
            source: CatalogSource::Discovery {
                endpoint,
                profile: Box::new(profile),
            },
        })
    }

    pub fn built_in(
        catalog: VersionedProfileId,
        provider: ProviderId,
        account: AccountId,
        provider_display_name: Option<String>,
        account_display_name: Option<String>,
    ) -> Result<Self, ModelServiceError> {
        // These exact persisted source identities keep durable decoding closed. Service
        // endpoints, model rows, and catalog resolution belong to provider crates.
        match (provider.as_str(), catalog.as_str()) {
            ("kimi", "kimi-platform-ai/v1" | "kimi-code-membership/v1")
            | (
                "qwencloud",
                "qwencloud-coding-plan-cn/v1"
                | "qwencloud-coding-plan-intl/v1"
                | "qwencloud-token-plan-team-intl/v1",
            ) => {},
            ("kimi", _) => {
                return Err(ModelServiceError::new(format!(
                    "unsupported Kimi catalog profile {catalog}"
                )));
            },
            ("qwencloud", _) => {
                return Err(ModelServiceError::new(format!(
                    "unsupported QwenCloud catalog profile {catalog}"
                )));
            },
            _ => {
                return Err(ModelServiceError::new(format!(
                    "Provider {provider} does not own a built-in catalog"
                )));
            },
        }
        crate::ConnectionAccount::new(
            provider.clone(),
            account.clone(),
            provider_display_name.clone(),
            account_display_name.clone(),
        )?;
        Ok(Self {
            provider,
            account,
            provider_display_name,
            account_display_name,
            source: CatalogSource::BuiltIn { catalog },
        })
    }

    pub const fn provider(&self) -> &ProviderId {
        &self.provider
    }

    pub const fn account(&self) -> &AccountId {
        &self.account
    }

    pub fn provider_display_name(&self) -> Option<&str> {
        self.provider_display_name.as_deref()
    }

    pub fn account_display_name(&self) -> Option<&str> {
        self.account_display_name.as_deref()
    }

    pub(crate) const fn source(&self) -> &CatalogSource {
        &self.source
    }

    /// Returns the exact built-in adapter identity when this is a static catalog seed.
    #[must_use]
    pub fn built_in_profile(&self) -> Option<&VersionedProfileId> {
        match &self.source {
            CatalogSource::BuiltIn { catalog } => Some(catalog),
            CatalogSource::Discovery { .. } => None,
        }
    }

    /// Returns the exact endpoint and effective profile of the persisted discovery source.
    #[must_use]
    pub fn discovery_definition(&self) -> Option<(&NormalizedEndpoint, &EffectiveModelProfile)> {
        match &self.source {
            CatalogSource::Discovery { endpoint, profile } => Some((endpoint, profile.as_ref())),
            CatalogSource::BuiltIn { .. } => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionAccount {
    provider_id: ProviderId,
    account_id: AccountId,
    provider_display_name: Option<String>,
    account_display_name: Option<String>,
}

impl ConnectionAccount {
    pub fn new(
        provider_id: ProviderId,
        account_id: AccountId,
        provider_display_name: Option<String>,
        account_display_name: Option<String>,
    ) -> Result<Self, ModelServiceError> {
        reject_new_host_provider(&provider_id)?;
        Self::from_durable(
            provider_id,
            account_id,
            provider_display_name,
            account_display_name,
        )
    }

    pub(super) fn from_durable(
        provider_id: ProviderId,
        account_id: AccountId,
        provider_display_name: Option<String>,
        account_display_name: Option<String>,
    ) -> Result<Self, ModelServiceError> {
        validate_display_name("Provider", provider_display_name.as_deref())?;
        validate_display_name("Account", account_display_name.as_deref())?;
        Ok(Self {
            provider_id,
            account_id,
            provider_display_name,
            account_display_name,
        })
    }

    #[must_use]
    pub const fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    #[must_use]
    pub const fn account_id(&self) -> &AccountId {
        &self.account_id
    }

    #[must_use]
    pub fn provider_display_name(&self) -> Option<&str> {
        self.provider_display_name.as_deref()
    }

    #[must_use]
    pub fn account_display_name(&self) -> Option<&str> {
        self.account_display_name.as_deref()
    }

    /// Stable Provider-and-Account reference using the same canonical escaping as ModelTarget.
    #[must_use]
    pub fn canonical_reference(&self) -> String {
        format!(
            "{}:{}",
            super::super::selection::encode_coordinate_segment(self.provider_id.as_str()),
            super::super::selection::encode_coordinate_segment(self.account_id.as_str()),
        )
    }
}

/// One durable stored model binding and its model presentation metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredModelBinding {
    complete: CompleteModelBinding,
    model_display_name: Option<String>,
    enabled: bool,
    last_failure: Option<ModelLastFailure>,
}

impl StoredModelBinding {
    pub fn new(
        complete: CompleteModelBinding,
        model_display_name: Option<String>,
    ) -> Result<Self, ModelServiceError> {
        reject_new_host_provider(complete.binding().provider_id())?;
        Self::from_durable(complete, model_display_name)
    }

    pub(super) fn from_durable(
        complete: CompleteModelBinding,
        model_display_name: Option<String>,
    ) -> Result<Self, ModelServiceError> {
        validate_display_name("Model", model_display_name.as_deref())?;
        Ok(Self {
            complete,
            model_display_name,
            enabled: true,
            last_failure: None,
        })
    }

    pub(super) fn from_durable_with_state(
        complete: CompleteModelBinding,
        model_display_name: Option<String>,
        enabled: bool,
        last_failure: Option<ModelLastFailure>,
    ) -> Result<Self, ModelServiceError> {
        let mut stored = Self::from_durable(complete, model_display_name)?;
        stored.enabled = enabled;
        stored.last_failure = last_failure;
        Ok(stored)
    }

    #[must_use]
    pub const fn complete(&self) -> &CompleteModelBinding {
        &self.complete
    }

    #[must_use]
    pub fn model_display_name(&self) -> Option<&str> {
        self.model_display_name.as_deref()
    }

    #[must_use]
    pub const fn last_failure(&self) -> Option<&ModelLastFailure> {
        self.last_failure.as_ref()
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub(super) fn with_last_failure(mut self, last_failure: Option<ModelLastFailure>) -> Self {
        self.last_failure = last_failure;
        self
    }

    pub(super) fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    #[must_use]
    pub fn selection(&self) -> ModelSelection {
        let binding = self.complete.binding();
        ModelSelection::new(
            binding.provider_id().clone(),
            binding.account_id().clone(),
            binding.model_id().clone(),
        )
    }
}

/// Closed, secret-free classification for one actual model request failure.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ModelRequestFailureKind {
    Authentication,
    AccessDenied,
    ModelUnavailable,
    RateLimited,
    RequestRejected,
    ProviderUnavailable,
    Transport,
    Timeout,
    Protocol,
    ResponseLimit,
    LocalConfiguration,
}

impl ModelRequestFailureKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authentication => "authentication",
            Self::AccessDenied => "access_denied",
            Self::ModelUnavailable => "model_unavailable",
            Self::RateLimited => "rate_limited",
            Self::RequestRejected => "request_rejected",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::Transport => "transport",
            Self::Timeout => "timeout",
            Self::Protocol => "protocol",
            Self::ResponseLimit => "response_limit",
            Self::LocalConfiguration => "local_configuration",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "authentication" => Some(Self::Authentication),
            "access_denied" => Some(Self::AccessDenied),
            "model_unavailable" => Some(Self::ModelUnavailable),
            "rate_limited" => Some(Self::RateLimited),
            "request_rejected" => Some(Self::RequestRejected),
            "provider_unavailable" => Some(Self::ProviderUnavailable),
            "transport" => Some(Self::Transport),
            "timeout" => Some(Self::Timeout),
            "protocol" => Some(Self::Protocol),
            "response_limit" => Some(Self::ResponseLimit),
            "local_configuration" => Some(Self::LocalConfiguration),
            _ => None,
        }
    }
}

impl Display for ModelRequestFailureKind {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter.write_str(self.as_str())
    }
}

/// One warning-only per-model observation retained outside complete-binding identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelLastFailure {
    kind: ModelRequestFailureKind,
    observed_at: String,
}

impl ModelLastFailure {
    pub fn new(
        kind: ModelRequestFailureKind,
        observed_at: impl Into<String>,
    ) -> Result<Self, ModelServiceError> {
        let observed_at = observed_at.into();
        let timestamp = observed_at.parse::<jiff::Timestamp>().map_err(|_| {
            ModelServiceError::new("model last_failure observed_at must be canonical UTC RFC 3339")
        })?;
        if timestamp.subsec_nanosecond() != 0 || timestamp.to_string() != observed_at {
            return Err(ModelServiceError::new(
                "model last_failure observed_at must be canonical UTC RFC 3339 at whole-second precision",
            ));
        }
        Ok(Self { kind, observed_at })
    }

    #[must_use]
    pub const fn kind(&self) -> ModelRequestFailureKind {
        self.kind
    }

    #[must_use]
    pub fn observed_at(&self) -> &str {
        &self.observed_at
    }
}

pub(super) fn validate_state(
    accounts: &[ConnectionAccount],
    bindings: &[StoredModelBinding],
) -> Result<(), ModelServiceError> {
    let mut account_coordinates = HashSet::new();
    let mut provider_display_names = HashMap::new();
    for account in accounts {
        let coordinate = (account.provider_id().clone(), account.account_id().clone());
        if !account_coordinates.insert(coordinate.clone()) {
            return Err(ModelServiceError::new(format!(
                "duplicate stored account for Provider {} and Account {}",
                account.provider_id(),
                account.account_id()
            )));
        }
        require_consistent_provider_display(
            &mut provider_display_names,
            account.provider_id().clone(),
            account.provider_display_name(),
        )?;
    }

    let mut binding_coordinates = HashSet::new();
    for binding in bindings {
        let complete = binding.complete().binding();
        let account_coordinate = (
            complete.provider_id().clone(),
            complete.account_id().clone(),
        );
        if !account_coordinates.contains(&account_coordinate) {
            return Err(ModelServiceError::new(format!(
                "stored model for Provider {}, Account {}, Model {} has no stored account",
                complete.provider_id(),
                complete.account_id(),
                complete.model_id()
            )));
        }
        let coordinate = (
            complete.provider_id().clone(),
            complete.account_id().clone(),
            complete.model_id().clone(),
        );
        if !binding_coordinates.insert(coordinate) {
            return Err(ModelServiceError::new(format!(
                "duplicate stored model for Provider {}, Account {}, Model {}",
                complete.provider_id(),
                complete.account_id(),
                complete.model_id()
            )));
        }
    }
    Ok(())
}

pub(super) fn account_matches_binding(
    account: &ConnectionAccount,
    binding: &StoredModelBinding,
) -> bool {
    let complete = binding.complete().binding();
    account.provider_id() == complete.provider_id() && account.account_id() == complete.account_id()
}

pub(super) fn binding_matches_selection(
    binding: &StoredModelBinding,
    selection: &ModelSelection,
) -> bool {
    let complete = binding.complete().binding();
    complete.provider_id() == selection.provider()
        && complete.account_id() == selection.account()
        && complete.model_id() == selection.model()
}

fn reject_new_host_provider(provider_id: &ProviderId) -> Result<(), ModelServiceError> {
    if provider_id.as_str() == "host" {
        return Err(ModelServiceError::new(
            "new stored connections cannot use the reserved ProviderId host",
        ));
    }
    Ok(())
}

fn require_consistent_provider_display(
    names: &mut HashMap<ProviderId, Option<String>>,
    provider_id: ProviderId,
    value: Option<&str>,
) -> Result<(), ModelServiceError> {
    let value = value.map(str::to_owned);
    match names.entry(provider_id.clone()) {
        Entry::Vacant(entry) => {
            entry.insert(value);
            Ok(())
        },
        Entry::Occupied(entry) if entry.get() == &value => Ok(()),
        Entry::Occupied(_) => Err(ModelServiceError::new(format!(
            "inconsistent stored display name for Provider {provider_id}"
        ))),
    }
}

#[derive(Debug)]
pub enum ConnectionRepositoryError {
    Io {
        path: PathBuf,
        source: IoError,
    },
    InvalidPath(PathBuf),
    UnsupportedFileType(PathBuf),
    WrongOwner(PathBuf),
    InsecurePermissions(PathBuf),
    TooLarge(PathBuf),
    Changed(PathBuf),
    InvalidContents(PathBuf),
    CoordinateMismatch,
    ModelNotFound {
        provider: String,
        account: String,
        model: String,
    },
    InvalidMutation,
    PreparedTooLarge,
    OperationBusy(PathBuf),
    PendingOperation(PathBuf),
    Conflict {
        expected: ConnectionRevision,
        observed: ConnectionRevision,
    },
    Randomness(String),
    TemporaryNameRandomness(String),
    TemporaryNameCollisionExhaustion {
        attempts: usize,
    },
}

impl ConnectionRepositoryError {
    pub(super) fn io(path: &Path, source: IoError) -> Self {
        Self::Io {
            path: path.to_owned(),
            source,
        }
    }
}

impl fmt::Display for ConnectionRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::InvalidPath(path) => {
                write!(formatter, "{} has no parent directory", path.display())
            },
            Self::UnsupportedFileType(path) => write!(
                formatter,
                "{} is not a regular connection file",
                path.display()
            ),
            Self::WrongOwner(path) => write!(
                formatter,
                "{} is not owned by the current effective user",
                path.display()
            ),
            Self::InsecurePermissions(path) => write!(
                formatter,
                "{} must not grant group or other permissions",
                path.display()
            ),
            Self::TooLarge(path) => write!(
                formatter,
                "{} exceeds the {MAX_CONNECTION_BYTES}-byte connection-file limit",
                path.display()
            ),
            Self::Changed(path) => write!(
                formatter,
                "{} changed while its connection snapshot was being read",
                path.display()
            ),
            Self::InvalidContents(path) if path.as_os_str().is_empty() => {
                formatter.write_str("the prepared connection snapshot is invalid")
            },
            Self::InvalidContents(path) => write!(
                formatter,
                "{} contains an invalid connection snapshot",
                path.display()
            ),
            Self::CoordinateMismatch => formatter.write_str(
                "the stored account and binding must name the same Provider and Account",
            ),
            Self::ModelNotFound {
                provider,
                account,
                model,
            } => write!(
                formatter,
                "stored model not found for Provider {provider}, Account {account}, Model {model}",
            ),
            Self::InvalidMutation => {
                formatter.write_str("the prepared stored connection mutation is invalid")
            },
            Self::PreparedTooLarge => {
                formatter.write_str("the prepared connection snapshot exceeds its bounded size")
            },
            Self::OperationBusy(path) => write!(
                formatter,
                "another connection operation owns {}",
                path.display()
            ),
            Self::PendingOperation(path) => write!(
                formatter,
                "{} contains a pending connection operation that this build cannot recover",
                path.display()
            ),
            Self::Conflict { expected, observed } => write!(
                formatter,
                "connection revision conflict: expected {expected}, observed {observed}; inspect the current default and retry"
            ),
            Self::Randomness(message) => write!(
                formatter,
                "generating a connection revision failed: {message}"
            ),
            Self::TemporaryNameRandomness(message) => write!(
                formatter,
                "generating a connection publication temporary name failed: {message}"
            ),
            Self::TemporaryNameCollisionExhaustion { attempts } => write!(
                formatter,
                "all {attempts} generated connection publication temporary names already exist"
            ),
        }
    }
}

impl Error for ConnectionRepositoryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
