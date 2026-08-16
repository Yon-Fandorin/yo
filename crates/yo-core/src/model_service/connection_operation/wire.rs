use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{
    ConnectionCredentialAction, ConnectionOperationError, ConnectionOperationJournalEntry,
    ConnectionOperationKind, ConnectionOperationPhase, journal::JournalCredential,
};
use crate::model_service::{
    AccountId, ConnectionRevision, CredentialMutationAction, CredentialRevision,
    PreparedConnectionMutation, PreparedCredentialMutation, ProviderId,
};

pub(super) fn encode(
    entry: &ConnectionOperationJournalEntry,
) -> Result<Vec<u8>, ConnectionOperationError> {
    let planned_snapshot = std::str::from_utf8(entry.connection_mutation().planned_bytes())
        .map_err(|_| ConnectionOperationError::InvalidEntry)?;
    yo_yaml::to_string(&WireEntry {
        operation_id: entry.operation_id(),
        kind: entry.kind().into(),
        config_snapshot_digest: entry.config_snapshot_digest(),
        phase: entry.phase().into(),
        connection: WireConnection {
            expected_revision: entry.connection_mutation().expected_revision().to_string(),
            planned_revision: entry.connection_mutation().planned_revision().to_string(),
            planned_snapshot,
        },
        credential: WireCredentialRef::from(entry),
    })
    .map(String::into_bytes)
    .map_err(|_| ConnectionOperationError::InvalidEntry)
}

pub(super) fn decode(
    path: &Path,
    encoded: &[u8],
) -> Result<ConnectionOperationJournalEntry, ConnectionOperationError> {
    let wire: WireEntryOwned = yo_yaml::from_slice(encoded).map_err(|_| {
        if yo_yaml::has_any_top_level_mapping_key(encoded, &["version", "profile_digests"])
            .unwrap_or(false)
        {
            ConnectionOperationError::RetiredYamlFormat(path.to_owned())
        } else {
            ConnectionOperationError::InvalidContents(path.to_owned())
        }
    })?;
    let invalid = || ConnectionOperationError::InvalidContents(path.to_owned());
    let expected_connection =
        ConnectionRevision::from_operation_journal(&wire.connection.expected_revision)
            .ok_or_else(invalid)?;
    let planned_connection =
        ConnectionRevision::from_operation_journal(&wire.connection.planned_revision)
            .ok_or_else(invalid)?;
    let connection = PreparedConnectionMutation::from_operation_journal(
        expected_connection,
        planned_connection,
        wire.connection.planned_snapshot.into_bytes(),
    )
    .map_err(|_| invalid())?;
    let credential = parse_credential(path, wire.credential)?;
    ConnectionOperationJournalEntry::from_stored_parts(
        wire.operation_id,
        wire.kind.into(),
        wire.config_snapshot_digest,
        wire.phase.into(),
        connection,
        credential,
    )
    .map_err(|_| invalid())
}

fn parse_credential(
    path: &Path,
    credential: WireCredential,
) -> Result<JournalCredential, ConnectionOperationError> {
    let invalid = || ConnectionOperationError::InvalidContents(path.to_owned());
    let action = credential.action();
    match credential {
        WireCredential::Add {
            expected_revision,
            planned_revision,
            provider,
            account,
        }
        | WireCredential::Replace {
            expected_revision,
            planned_revision,
            provider,
            account,
        }
        | WireCredential::Remove {
            expected_revision,
            planned_revision,
            provider,
            account,
        } => {
            let action = match action {
                ConnectionCredentialAction::Add => CredentialMutationAction::Add,
                ConnectionCredentialAction::Replace => CredentialMutationAction::Replace,
                ConnectionCredentialAction::Remove => CredentialMutationAction::Remove,
                ConnectionCredentialAction::Preserve => unreachable!("matched mutation variant"),
            };
            let expected = CredentialRevision::from_operation_journal(&expected_revision)
                .ok_or_else(invalid)?;
            let planned = CredentialRevision::from_operation_journal(&planned_revision)
                .ok_or_else(invalid)?;
            let provider = ProviderId::new(provider).map_err(|_| invalid())?;
            let account = AccountId::new(account).map_err(|_| invalid())?;
            PreparedCredentialMutation::from_operation_journal(
                expected, planned, provider, account, action,
            )
            .map(JournalCredential::Mutation)
            .ok_or_else(invalid)
        },
        WireCredential::Preserve { expected_revision } => {
            CredentialRevision::from_operation_journal(&expected_revision)
                .map(JournalCredential::Preserve)
                .ok_or_else(invalid)
        },
    }
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct WireEntry<'a> {
    operation_id: &'a str,
    kind: WireKind,
    config_snapshot_digest: &'a str,
    phase: WirePhase,
    connection: WireConnection<'a>,
    credential: WireCredentialRef<'a>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEntryOwned {
    operation_id: String,
    kind: WireKind,
    config_snapshot_digest: String,
    phase: WirePhase,
    connection: WireConnectionOwned,
    credential: WireCredential,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct WireConnection<'a> {
    expected_revision: String,
    planned_revision: String,
    planned_snapshot: &'a str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireConnectionOwned {
    expected_revision: String,
    planned_revision: String,
    planned_snapshot: String,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum WireKind {
    ConnectCredentialChange,
    Disconnect,
}

impl From<ConnectionOperationKind> for WireKind {
    fn from(value: ConnectionOperationKind) -> Self {
        match value {
            ConnectionOperationKind::ConnectCredentialChange => Self::ConnectCredentialChange,
            ConnectionOperationKind::Disconnect => Self::Disconnect,
        }
    }
}

impl From<WireKind> for ConnectionOperationKind {
    fn from(value: WireKind) -> Self {
        match value {
            WireKind::ConnectCredentialChange => Self::ConnectCredentialChange,
            WireKind::Disconnect => Self::Disconnect,
        }
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum WirePhase {
    Intent,
    CredentialCommitted,
    PublicCommitted,
    CredentialRemoved,
    Complete,
}

impl From<ConnectionOperationPhase> for WirePhase {
    fn from(value: ConnectionOperationPhase) -> Self {
        match value {
            ConnectionOperationPhase::Intent => Self::Intent,
            ConnectionOperationPhase::CredentialCommitted => Self::CredentialCommitted,
            ConnectionOperationPhase::PublicCommitted => Self::PublicCommitted,
            ConnectionOperationPhase::CredentialRemoved => Self::CredentialRemoved,
            ConnectionOperationPhase::Complete => Self::Complete,
        }
    }
}

impl From<WirePhase> for ConnectionOperationPhase {
    fn from(value: WirePhase) -> Self {
        match value {
            WirePhase::Intent => Self::Intent,
            WirePhase::CredentialCommitted => Self::CredentialCommitted,
            WirePhase::PublicCommitted => Self::PublicCommitted,
            WirePhase::CredentialRemoved => Self::CredentialRemoved,
            WirePhase::Complete => Self::Complete,
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum WireCredentialRef<'a> {
    Add {
        expected_revision: &'a str,
        planned_revision: &'a str,
        provider: &'a str,
        account: &'a str,
    },
    Replace {
        expected_revision: &'a str,
        planned_revision: &'a str,
        provider: &'a str,
        account: &'a str,
    },
    Remove {
        expected_revision: &'a str,
        planned_revision: &'a str,
        provider: &'a str,
        account: &'a str,
    },
    Preserve {
        expected_revision: &'a str,
    },
}

impl<'a> From<&'a ConnectionOperationJournalEntry> for WireCredentialRef<'a> {
    fn from(entry: &'a ConnectionOperationJournalEntry) -> Self {
        match entry.credential() {
            JournalCredential::Mutation(mutation) => {
                let fields = || {
                    (
                        mutation.expected_revision().operation_journal_token(),
                        mutation.planned_revision().operation_journal_token(),
                        mutation.provider().as_str(),
                        mutation.account().as_str(),
                    )
                };
                match mutation.action() {
                    CredentialMutationAction::Add => {
                        let (expected_revision, planned_revision, provider, account) = fields();
                        Self::Add {
                            expected_revision,
                            planned_revision,
                            provider,
                            account,
                        }
                    },
                    CredentialMutationAction::Replace => {
                        let (expected_revision, planned_revision, provider, account) = fields();
                        Self::Replace {
                            expected_revision,
                            planned_revision,
                            provider,
                            account,
                        }
                    },
                    CredentialMutationAction::Remove => {
                        let (expected_revision, planned_revision, provider, account) = fields();
                        Self::Remove {
                            expected_revision,
                            planned_revision,
                            provider,
                            account,
                        }
                    },
                }
            },
            JournalCredential::Preserve(revision) => Self::Preserve {
                expected_revision: revision.operation_journal_token(),
            },
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum WireCredential {
    Add {
        expected_revision: String,
        planned_revision: String,
        provider: String,
        account: String,
    },
    Replace {
        expected_revision: String,
        planned_revision: String,
        provider: String,
        account: String,
    },
    Remove {
        expected_revision: String,
        planned_revision: String,
        provider: String,
        account: String,
    },
    Preserve {
        expected_revision: String,
    },
}

impl WireCredential {
    const fn action(&self) -> ConnectionCredentialAction {
        match self {
            Self::Add { .. } => ConnectionCredentialAction::Add,
            Self::Replace { .. } => ConnectionCredentialAction::Replace,
            Self::Remove { .. } => ConnectionCredentialAction::Remove,
            Self::Preserve { .. } => ConnectionCredentialAction::Preserve,
        }
    }
}
