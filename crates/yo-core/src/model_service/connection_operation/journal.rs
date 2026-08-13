use std::fmt;

use super::ConnectionOperationError;
use crate::model_service::{
    CredentialMutationAction, CredentialRevision, CredentialSnapshot, PreparedConnectionMutation,
    PreparedCredentialMutation,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionOperationKind {
    ConnectCredentialChange,
    Disconnect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionCredentialAction {
    Add,
    Replace,
    Remove,
    Preserve,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionOperationPhase {
    Intent,
    CredentialCommitted,
    PublicCommitted,
    CredentialRemoved,
    Complete,
}

#[derive(Clone, Eq, PartialEq)]
pub struct ConnectionOperationJournalEntry {
    operation_id: String,
    kind: ConnectionOperationKind,
    config_snapshot_digest: String,
    profile_digests: Vec<String>,
    phase: ConnectionOperationPhase,
    connection: PreparedConnectionMutation,
    credential: JournalCredential,
}

#[derive(Clone, Eq, PartialEq)]
pub(super) enum JournalCredential {
    Mutation(PreparedCredentialMutation),
    Preserve(CredentialRevision),
}

impl ConnectionOperationJournalEntry {
    pub fn connect_credential_change(
        config_snapshot_digest: impl Into<String>,
        profile_digests: Vec<String>,
        connection: PreparedConnectionMutation,
        credential: PreparedCredentialMutation,
    ) -> Result<Self, ConnectionOperationError> {
        if !matches!(
            credential.action(),
            CredentialMutationAction::Add | CredentialMutationAction::Replace
        ) {
            return Err(ConnectionOperationError::InvalidEntry);
        }
        Self::new(
            ConnectionOperationKind::ConnectCredentialChange,
            config_snapshot_digest.into(),
            profile_digests,
            connection,
            JournalCredential::Mutation(credential),
        )
    }

    pub fn disconnect_remove(
        config_snapshot_digest: impl Into<String>,
        profile_digests: Vec<String>,
        connection: PreparedConnectionMutation,
        credential: PreparedCredentialMutation,
    ) -> Result<Self, ConnectionOperationError> {
        if credential.action() != CredentialMutationAction::Remove {
            return Err(ConnectionOperationError::InvalidEntry);
        }
        Self::new(
            ConnectionOperationKind::Disconnect,
            config_snapshot_digest.into(),
            profile_digests,
            connection,
            JournalCredential::Mutation(credential),
        )
    }

    pub fn disconnect_preserve(
        config_snapshot_digest: impl Into<String>,
        profile_digests: Vec<String>,
        connection: PreparedConnectionMutation,
        expected_credential_revision: CredentialRevision,
    ) -> Result<Self, ConnectionOperationError> {
        Self::new(
            ConnectionOperationKind::Disconnect,
            config_snapshot_digest.into(),
            profile_digests,
            connection,
            JournalCredential::Preserve(expected_credential_revision),
        )
    }

    fn new(
        kind: ConnectionOperationKind,
        config_snapshot_digest: String,
        profile_digests: Vec<String>,
        connection: PreparedConnectionMutation,
        credential: JournalCredential,
    ) -> Result<Self, ConnectionOperationError> {
        let operation_id = new_operation_id()?;
        Self::from_stored_parts(
            operation_id,
            kind,
            config_snapshot_digest,
            profile_digests,
            ConnectionOperationPhase::Intent,
            connection,
            credential,
        )
    }

    pub(super) fn from_stored_parts(
        operation_id: String,
        kind: ConnectionOperationKind,
        config_snapshot_digest: String,
        profile_digests: Vec<String>,
        phase: ConnectionOperationPhase,
        connection: PreparedConnectionMutation,
        credential: JournalCredential,
    ) -> Result<Self, ConnectionOperationError> {
        if !valid_operation_id(&operation_id)
            || !valid_digest(&config_snapshot_digest)
            || !profile_digests.iter().all(|digest| valid_digest(digest))
        {
            return Err(ConnectionOperationError::InvalidEntry);
        }
        let entry = Self {
            operation_id,
            kind,
            config_snapshot_digest,
            profile_digests,
            phase,
            connection,
            credential,
        };
        let combination_is_legal = matches!(
            (entry.kind, entry.credential_action()),
            (
                ConnectionOperationKind::ConnectCredentialChange,
                ConnectionCredentialAction::Add | ConnectionCredentialAction::Replace
            ) | (
                ConnectionOperationKind::Disconnect,
                ConnectionCredentialAction::Remove | ConnectionCredentialAction::Preserve
            )
        );
        if !combination_is_legal || !entry.phase_is_legal(phase) {
            return Err(ConnectionOperationError::InvalidEntry);
        }
        Ok(entry)
    }

    #[must_use]
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    #[must_use]
    pub const fn kind(&self) -> ConnectionOperationKind {
        self.kind
    }

    #[must_use]
    pub fn config_snapshot_digest(&self) -> &str {
        &self.config_snapshot_digest
    }

    #[must_use]
    pub fn profile_digests(&self) -> &[String] {
        &self.profile_digests
    }

    #[must_use]
    pub const fn phase(&self) -> ConnectionOperationPhase {
        self.phase
    }

    #[must_use]
    pub const fn connection_mutation(&self) -> &PreparedConnectionMutation {
        &self.connection
    }

    #[must_use]
    pub const fn credential_action(&self) -> ConnectionCredentialAction {
        match &self.credential {
            JournalCredential::Mutation(mutation) => match mutation.action() {
                CredentialMutationAction::Add => ConnectionCredentialAction::Add,
                CredentialMutationAction::Replace => ConnectionCredentialAction::Replace,
                CredentialMutationAction::Remove => ConnectionCredentialAction::Remove,
            },
            JournalCredential::Preserve(_) => ConnectionCredentialAction::Preserve,
        }
    }

    #[must_use]
    pub const fn credential_mutation(&self) -> Option<&PreparedCredentialMutation> {
        match &self.credential {
            JournalCredential::Mutation(mutation) => Some(mutation),
            JournalCredential::Preserve(_) => None,
        }
    }

    #[must_use]
    pub const fn expected_credential_revision(&self) -> &CredentialRevision {
        match &self.credential {
            JournalCredential::Mutation(mutation) => mutation.expected_revision(),
            JournalCredential::Preserve(revision) => revision,
        }
    }

    pub(super) fn credential(&self) -> &JournalCredential {
        &self.credential
    }

    pub(super) fn with_phase(
        &self,
        next: ConnectionOperationPhase,
    ) -> Result<Self, ConnectionOperationError> {
        if self.next_phase() != Some(next) {
            return Err(ConnectionOperationError::InvalidTransition {
                kind: self.kind,
                action: self.credential_action(),
                from: self.phase,
                to: next,
            });
        }
        let mut advanced = self.clone();
        advanced.phase = next;
        Ok(advanced)
    }

    fn next_phase(&self) -> Option<ConnectionOperationPhase> {
        use ConnectionOperationPhase::{
            Complete, CredentialCommitted, CredentialRemoved, Intent, PublicCommitted,
        };
        match (self.kind, self.credential_action(), self.phase) {
            (ConnectionOperationKind::ConnectCredentialChange, _, Intent) => {
                Some(CredentialCommitted)
            },
            (ConnectionOperationKind::ConnectCredentialChange, _, CredentialCommitted) => {
                Some(PublicCommitted)
            },
            (ConnectionOperationKind::ConnectCredentialChange, _, PublicCommitted) => {
                Some(Complete)
            },
            (ConnectionOperationKind::Disconnect, ConnectionCredentialAction::Remove, Intent) => {
                Some(PublicCommitted)
            },
            (
                ConnectionOperationKind::Disconnect,
                ConnectionCredentialAction::Remove,
                PublicCommitted,
            ) => Some(CredentialRemoved),
            (
                ConnectionOperationKind::Disconnect,
                ConnectionCredentialAction::Remove,
                CredentialRemoved,
            ) => Some(Complete),
            (ConnectionOperationKind::Disconnect, ConnectionCredentialAction::Preserve, Intent) => {
                Some(PublicCommitted)
            },
            (
                ConnectionOperationKind::Disconnect,
                ConnectionCredentialAction::Preserve,
                PublicCommitted,
            ) => Some(Complete),
            _ => None,
        }
    }

    fn phase_is_legal(&self, phase: ConnectionOperationPhase) -> bool {
        if phase == ConnectionOperationPhase::Intent {
            return true;
        }
        let mut cursor = self.clone();
        cursor.phase = ConnectionOperationPhase::Intent;
        while let Some(next) = cursor.next_phase() {
            if next == phase {
                return true;
            }
            cursor.phase = next;
        }
        false
    }

    pub(super) fn credential_matches_expected(&self, snapshot: &CredentialSnapshot) -> bool {
        match &self.credential {
            JournalCredential::Mutation(mutation) => snapshot.matches_expected(mutation),
            JournalCredential::Preserve(revision) => snapshot.revision() == revision,
        }
    }

    pub(super) fn credential_matches_planned(&self, snapshot: &CredentialSnapshot) -> bool {
        match &self.credential {
            JournalCredential::Mutation(mutation) => snapshot.matches_planned(mutation),
            JournalCredential::Preserve(_) => false,
        }
    }
}

impl fmt::Debug for ConnectionOperationJournalEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectionOperationJournalEntry")
            .field("operation_id", &self.operation_id)
            .field("kind", &self.kind)
            .field("config_snapshot_digest", &self.config_snapshot_digest)
            .field("profile_digest_count", &self.profile_digests.len())
            .field("phase", &self.phase)
            .field("credential_action", &self.credential_action())
            .finish_non_exhaustive()
    }
}

fn new_operation_id() -> Result<String, ConnectionOperationError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| ConnectionOperationError::Randomness(error.to_string()))?;
    let mut operation_id = String::with_capacity(35);
    operation_id.push_str("op-");
    for byte in bytes {
        use fmt::Write as _;
        write!(operation_id, "{byte:02x}").expect("formatting into a String cannot fail");
    }
    Ok(operation_id)
}

pub(super) fn valid_operation_id(value: &str) -> bool {
    value.len() == 35
        && value.starts_with("op-")
        && value[3..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub(super) fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
