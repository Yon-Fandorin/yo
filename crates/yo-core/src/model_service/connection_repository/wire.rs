use std::{
    fmt,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{
    super::{AccountId, ModelId, ModelSelection, ProviderId, StartupTarget},
    ConnectionRepositoryError, ConnectionRevision,
};

pub(super) struct DecodedSnapshot {
    pub(super) revision: ConnectionRevision,
    pub(super) preference: Option<StartupTarget>,
}

pub(super) fn encode(
    revision: &ConnectionRevision,
    preference: Option<&StartupTarget>,
) -> Result<Vec<u8>, ConnectionRepositoryError> {
    serde_norway::to_string(&WireSnapshot {
        version: 1,
        revision: revision.to_string(),
        preference: preference.map(WireTarget::from),
        bindings: Vec::new(),
        accounts: Vec::new(),
    })
    .map(String::into_bytes)
    .map_err(|_| ConnectionRepositoryError::InvalidContents(PathBuf::new()))
}

pub(super) fn decode(
    path: &Path,
    encoded: &[u8],
) -> Result<DecodedSnapshot, ConnectionRepositoryError> {
    let wire: WireSnapshot = serde_norway::from_slice(encoded)
        .map_err(|_| ConnectionRepositoryError::InvalidContents(path.to_owned()))?;
    if wire.version != 1 {
        return Err(ConnectionRepositoryError::UnsupportedVersion {
            path: path.to_owned(),
            version: wire.version,
        });
    }
    if !wire.bindings.is_empty() || !wire.accounts.is_empty() {
        return Err(ConnectionRepositoryError::ManagedStateUnsupported(
            path.to_owned(),
        ));
    }
    Ok(DecodedSnapshot {
        revision: parse_revision(path, &wire.revision)?,
        preference: wire
            .preference
            .map(|target| parse_target(path, target))
            .transpose()?,
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
    bindings: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    accounts: Vec<serde_json::Value>,
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
