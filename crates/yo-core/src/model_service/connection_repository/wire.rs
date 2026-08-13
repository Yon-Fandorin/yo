use std::{
    fmt,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{
    super::{AccountId, ModelId, ModelSelection, ProviderId, StartupTarget},
    ConnectionRepositoryError, ConnectionRevision, ManagedConnectionAccount,
    ManagedConnectionBinding, managed,
};
use crate::{
    CompleteModelBinding, ConnectorId, EffectiveModelBinding, EffectiveModelProfile,
    ModelProfileLayer, ModelProfileParameters, NormalizedEndpoint, VersionedProfileId,
    validate_profile_yaml_number_spellings,
};

pub(super) struct DecodedSnapshot {
    pub(super) revision: ConnectionRevision,
    pub(super) preference: Option<StartupTarget>,
    pub(super) accounts: Vec<ManagedConnectionAccount>,
    pub(super) bindings: Vec<ManagedConnectionBinding>,
}

pub(super) fn encode(
    revision: &ConnectionRevision,
    preference: Option<&StartupTarget>,
    accounts: &[ManagedConnectionAccount],
    bindings: &[ManagedConnectionBinding],
) -> Result<Vec<u8>, ConnectionRepositoryError> {
    serde_norway::to_string(&WireSnapshot {
        version: 1,
        revision: revision.to_string(),
        preference: preference.map(WireTarget::from),
        bindings: bindings.iter().map(WireBinding::from).collect(),
        accounts: accounts.iter().map(WireAccount::from).collect(),
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
    validate_profile_yaml_number_spellings(contents)
        .map_err(|_| ConnectionRepositoryError::InvalidContents(path.to_owned()))?;
    let wire: WireSnapshot = serde_norway::from_str(contents)
        .map_err(|_| ConnectionRepositoryError::InvalidContents(path.to_owned()))?;
    if wire.version != 1 {
        return Err(ConnectionRepositoryError::UnsupportedVersion {
            path: path.to_owned(),
            version: wire.version,
        });
    }
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
    managed::validate_state(&accounts, &bindings).map_err(invalid)?;
    Ok(DecodedSnapshot {
        revision: parse_revision(path, &wire.revision)?,
        preference: wire
            .preference
            .map(|target| parse_target(path, target))
            .transpose()?,
        accounts,
        bindings,
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
    version: u32,
    revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preference: Option<WireTarget>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    bindings: Vec<WireBinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    accounts: Vec<WireAccount>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireAccount {
    provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_display_name: Option<String>,
    account: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    account_display_name: Option<String>,
}

impl From<&ManagedConnectionAccount> for WireAccount {
    fn from(account: &ManagedConnectionAccount) -> Self {
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model_display_name: Option<String>,
    connector: String,
    base_url: String,
    profile: WireProfile,
}

impl From<&ManagedConnectionBinding> for WireBinding {
    fn from(managed: &ManagedConnectionBinding) -> Self {
        let complete = managed.complete();
        let binding = complete.binding();
        let profile = complete.profile();
        Self {
            provider: binding.provider_id().as_str().to_owned(),
            account: binding.account_id().as_str().to_owned(),
            model: binding.model_id().as_str().to_owned(),
            model_display_name: managed.model_display_name().map(str::to_owned),
            connector: binding.connector_id().as_str().to_owned(),
            base_url: binding.endpoint().as_str().to_owned(),
            profile: WireProfile {
                api_dialect: profile.api_dialect().as_str().to_owned(),
                tokenizer_profile: profile.context().tokenizer_profile().to_owned(),
                input_token_limit: profile.context().input_token_limit(),
                max_output_tokens: profile.context().max_output_tokens(),
                reasoning_parameters: profile.reasoning_parameters().clone(),
                optional_request_parameters: profile.optional_request_parameters().clone(),
                tool_capability_policy: profile.tool_capability_policy().as_str().to_owned(),
                verification_profile: profile.verification_profile().as_str().to_owned(),
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireProfile {
    api_dialect: String,
    tokenizer_profile: String,
    input_token_limit: u64,
    max_output_tokens: u64,
    reasoning_parameters: ModelProfileParameters,
    optional_request_parameters: ModelProfileParameters,
    tool_capability_policy: String,
    verification_profile: String,
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

fn parse_account(
    account: WireAccount,
) -> Result<ManagedConnectionAccount, crate::ModelServiceError> {
    ManagedConnectionAccount::from_durable(
        ProviderId::new(account.provider)?,
        AccountId::new(account.account)?,
        account.provider_display_name,
        account.account_display_name,
    )
}

fn parse_binding(
    binding: WireBinding,
) -> Result<ManagedConnectionBinding, crate::ModelServiceError> {
    let WireBinding {
        provider,
        account,
        model,
        model_display_name,
        connector,
        base_url,
        profile,
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
    let layer = ModelProfileLayer::new(
        Some(dialect),
        Some(VersionedProfileId::new(profile.tokenizer_profile)?),
        Some(profile.input_token_limit),
        Some(profile.max_output_tokens),
        Some(profile.reasoning_parameters),
        Some(profile.optional_request_parameters),
        Some(VersionedProfileId::new(profile.tool_capability_policy)?),
        Some(VersionedProfileId::new(profile.verification_profile)?),
    );
    let profile = EffectiveModelProfile::resolve(None, &layer)?;
    ManagedConnectionBinding::from_durable(
        CompleteModelBinding::new(effective, profile)?,
        model_display_name,
    )
}
