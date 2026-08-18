use std::{
    fmt,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{
    super::{AccountId, ModelId, ModelSelection, ProviderId, StartupTarget},
    ConnectionAccount, ConnectionCatalogSeed, ConnectionRepositoryError, ConnectionRevision,
    ModelLastFailure, ModelRequestFailureKind, StoredModelBinding,
    catalog_seed::CatalogSource,
    stored,
};
use crate::{
    CompleteModelBinding, ConnectorId, EffectiveModelBinding, EffectiveModelProfile,
    ModelProfileLayer, ModelProfileParameters, NormalizedEndpoint, SEMANTIC_REPLAY_PROFILE,
    VersionedProfileId,
};

pub(super) struct DecodedSnapshot {
    pub(super) revision: ConnectionRevision,
    pub(super) preference: Option<StartupTarget>,
    pub(super) accounts: Vec<ConnectionAccount>,
    pub(super) bindings: Vec<StoredModelBinding>,
    pub(super) catalog_seeds: Vec<ConnectionCatalogSeed>,
}

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
    let contents = std::str::from_utf8(encoded)
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
    stored::validate_state(&accounts, &bindings).map_err(invalid)?;
    super::validate_catalog_seeds(&accounts, &catalog_seeds)?;
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
        profile: WireProfile,
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
            CatalogSource::OpenRouter { endpoint, profile } => Self::OpenrouterDiscovery {
                provider: seed.provider().as_str().to_owned(),
                account: seed.account().as_str().to_owned(),
                base_url: endpoint.as_str().to_owned(),
                profile: WireProfile::from(profile.as_ref()),
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
        }
    }
}

fn deserialize_non_null_profile_parameters<'de, D>(
    deserializer: D,
) -> Result<ModelProfileParameters, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<ModelProfileParameters>::deserialize(deserializer)?.ok_or_else(|| {
        serde::de::Error::invalid_type(serde::de::Unexpected::Unit, &"a structured profile value")
    })
}

fn deserialize_optional_non_null<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

fn deserialize_optional_replay_profile<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if matches!(
        value.as_str(),
        crate::SEMANTIC_REPLAY_PROFILE | crate::KIMI_PRIVATE_REPLAY_PROFILE
    ) {
        Ok(Some(value))
    } else {
        Err(serde::de::Error::custom(
            "present replay_profile is outside the closed supported set",
        ))
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
            StartupTarget::HostCodex => Self::Host {
                target: StartupTarget::HOST_CODEX_REFERENCE.to_owned(),
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
        WireTarget::Host { target } if target == StartupTarget::HOST_CODEX_REFERENCE => {
            Ok(StartupTarget::HostCodex)
        },
        WireTarget::Host { .. } => Err(ConnectionRepositoryError::InvalidContents(path.to_owned())),
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
    StoredModelBinding::from_durable_with_failure(
        CompleteModelBinding::new(effective, profile)?,
        model_display_name,
        last_failure,
    )
}

fn parse_profile(profile: WireProfile) -> Result<EffectiveModelProfile, crate::ModelServiceError> {
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
            ConnectionCatalogSeed::openrouter(
                provider,
                account,
                metadata.provider_display_name().map(str::to_owned),
                metadata.account_display_name().map(str::to_owned),
                NormalizedEndpoint::parse(&base_url)?,
                parse_profile(profile)?,
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
        .find(|candidate| candidate.provider_id() == provider && candidate.account_id() == account)
        .ok_or_else(|| crate::ModelServiceError::new("catalog seed has no stored account"))
}
