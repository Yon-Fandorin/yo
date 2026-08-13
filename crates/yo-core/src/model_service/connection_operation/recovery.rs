use super::{
    ConnectionCredentialAction, ConnectionOperationError, ConnectionOperationJournalEntry,
    ConnectionOperationKind, ConnectionOperationPhase,
};
use crate::model_service::{ConnectionSnapshot, CredentialSnapshot};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionOperationRecovery {
    Abandon,
    CommitPublic,
    CommitCredentialRemoval,
    Complete,
}

pub fn plan_connection_recovery(
    entry: &ConnectionOperationJournalEntry,
    credentials: &CredentialSnapshot,
    connections: &ConnectionSnapshot,
) -> Result<ConnectionOperationRecovery, ConnectionOperationError> {
    let credential_state = if entry.credential_matches_expected(credentials) {
        ObservedState::Expected
    } else if entry.credential_matches_planned(credentials) {
        ObservedState::Planned
    } else {
        ObservedState::Other
    };
    let connection_state =
        if connections.revision() == entry.connection_mutation().expected_revision() {
            ObservedState::Expected
        } else if connections.matches_planned(entry.connection_mutation()) {
            ObservedState::Planned
        } else {
            ObservedState::Other
        };

    let decision = match (entry.kind(), entry.credential_action()) {
        (
            ConnectionOperationKind::ConnectCredentialChange,
            ConnectionCredentialAction::Add | ConnectionCredentialAction::Replace,
        ) => plan_connect(entry.phase(), credential_state, connection_state),
        (ConnectionOperationKind::Disconnect, ConnectionCredentialAction::Remove) => {
            plan_disconnect_remove(entry.phase(), credential_state, connection_state)
        },
        (ConnectionOperationKind::Disconnect, ConnectionCredentialAction::Preserve) => {
            plan_disconnect_preserve(entry.phase(), credential_state, connection_state)
        },
        _ => None,
    };

    decision.ok_or(ConnectionOperationError::RecoveryConflict {
        kind: entry.kind(),
        action: entry.credential_action(),
        phase: entry.phase(),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ObservedState {
    Expected,
    Planned,
    Other,
}

fn plan_connect(
    phase: ConnectionOperationPhase,
    credentials: ObservedState,
    connections: ObservedState,
) -> Option<ConnectionOperationRecovery> {
    match (credentials, connections) {
        (ObservedState::Expected, ObservedState::Expected)
            if phase == ConnectionOperationPhase::Intent =>
        {
            Some(ConnectionOperationRecovery::Abandon)
        },
        (ObservedState::Planned, ObservedState::Expected)
            if matches!(
                phase,
                ConnectionOperationPhase::Intent | ConnectionOperationPhase::CredentialCommitted
            ) =>
        {
            Some(ConnectionOperationRecovery::CommitPublic)
        },
        (ObservedState::Planned, ObservedState::Planned) => {
            Some(ConnectionOperationRecovery::Complete)
        },
        _ => None,
    }
}

fn plan_disconnect_remove(
    phase: ConnectionOperationPhase,
    credentials: ObservedState,
    connections: ObservedState,
) -> Option<ConnectionOperationRecovery> {
    match (credentials, connections) {
        (ObservedState::Expected, ObservedState::Expected)
            if phase == ConnectionOperationPhase::Intent =>
        {
            Some(ConnectionOperationRecovery::Abandon)
        },
        (ObservedState::Expected, ObservedState::Planned)
            if matches!(
                phase,
                ConnectionOperationPhase::Intent | ConnectionOperationPhase::PublicCommitted
            ) =>
        {
            Some(ConnectionOperationRecovery::CommitCredentialRemoval)
        },
        (ObservedState::Planned, ObservedState::Planned) => {
            Some(ConnectionOperationRecovery::Complete)
        },
        _ => None,
    }
}

fn plan_disconnect_preserve(
    phase: ConnectionOperationPhase,
    credentials: ObservedState,
    connections: ObservedState,
) -> Option<ConnectionOperationRecovery> {
    match (credentials, connections) {
        (ObservedState::Expected, ObservedState::Expected)
            if phase == ConnectionOperationPhase::Intent =>
        {
            Some(ConnectionOperationRecovery::Abandon)
        },
        (ObservedState::Expected, ObservedState::Planned) => {
            Some(ConnectionOperationRecovery::Complete)
        },
        _ => None,
    }
}
