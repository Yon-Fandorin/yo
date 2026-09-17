use super::{
    super::{
        ConnectionOperationError, ConnectionOperationJournalEntry, ConnectionOperationPhase,
        ConnectionOperationRecovery,
    },
    model::{
        ConnectionOperationExecutionError, ConnectionOperationExecutionOutcome, credential_error,
        journal_error, public_error,
    },
    paths::LocalDirectoryIdentity,
    repositories::LocalConnectionOperationRepositories,
};
use crate::model_service::LocalConnectionOperationGuard;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum RecoveryStep {
    JournalAbandoned,
    JournalAdvanced(ConnectionOperationPhase),
    PublicCommitted,
    CredentialRemoved,
    JournalCleared,
}

pub(in super::super) fn execute_recovery(
    repositories: &LocalConnectionOperationRepositories,
    directory_identity: &LocalDirectoryIdentity,
    guard: &mut LocalConnectionOperationGuard,
    entry: ConnectionOperationJournalEntry,
    observe: &mut impl FnMut(RecoveryStep) -> Result<(), ConnectionOperationExecutionError>,
) -> Result<ConnectionOperationExecutionOutcome, ConnectionOperationExecutionError> {
    let recovered_from = entry.phase();
    directory_identity.revalidate()?;
    let credentials = repositories
        .credentials
        .capture()
        .map_err(|source| credential_error(&entry, source))?;
    directory_identity.revalidate()?;
    let connections = repositories
        .connections
        .capture()
        .map_err(|source| public_error(&entry, source))?;
    let decision = super::super::plan_connection_recovery(&entry, &credentials, &connections)
        .map_err(|source| journal_error(&entry, source))?;
    let kind = entry.kind();
    let action = entry.credential_action();
    let outcome = match decision {
        ConnectionOperationRecovery::Abandon => {
            directory_identity.revalidate()?;
            repositories
                .journal
                .abandon_intent(guard, &entry)
                .map_err(|source| journal_error(&entry, source))?;
            observe(RecoveryStep::JournalAbandoned)?;
            ConnectionOperationExecutionOutcome::Abandoned { kind, action }
        },
        ConnectionOperationRecovery::CommitPublic => {
            let mut entry = advance_to(
                repositories,
                guard,
                entry,
                ConnectionOperationPhase::CredentialCommitted,
                directory_identity,
                observe,
            )?;
            directory_identity.revalidate()?;
            repositories
                .connections
                .commit(entry.connection_mutation())
                .map_err(|source| public_error(&entry, source))?;
            observe(RecoveryStep::PublicCommitted)?;
            entry = advance_to(
                repositories,
                guard,
                entry,
                ConnectionOperationPhase::PublicCommitted,
                directory_identity,
                observe,
            )?;
            complete_and_clear(repositories, directory_identity, guard, entry, observe)?;
            ConnectionOperationExecutionOutcome::Completed {
                kind,
                action,
                recovered_from,
            }
        },
        ConnectionOperationRecovery::CommitCredentialRemoval => {
            let mut entry = advance_to(
                repositories,
                guard,
                entry,
                ConnectionOperationPhase::PublicCommitted,
                directory_identity,
                observe,
            )?;
            let mutation = entry
                .credential_mutation()
                .expect("disconnect-remove journal entries always contain a mutation");
            directory_identity.revalidate()?;
            repositories
                .credentials
                .commit(mutation, None)
                .map_err(|source| credential_error(&entry, source))?;
            observe(RecoveryStep::CredentialRemoved)?;
            entry = advance_to(
                repositories,
                guard,
                entry,
                ConnectionOperationPhase::CredentialRemoved,
                directory_identity,
                observe,
            )?;
            complete_and_clear(repositories, directory_identity, guard, entry, observe)?;
            ConnectionOperationExecutionOutcome::Completed {
                kind,
                action,
                recovered_from,
            }
        },
        ConnectionOperationRecovery::Complete => {
            complete_and_clear(repositories, directory_identity, guard, entry, observe)?;
            ConnectionOperationExecutionOutcome::Completed {
                kind,
                action,
                recovered_from,
            }
        },
    };
    Ok(outcome)
}

fn advance_to(
    repositories: &LocalConnectionOperationRepositories,
    guard: &mut LocalConnectionOperationGuard,
    mut entry: ConnectionOperationJournalEntry,
    target: ConnectionOperationPhase,
    directory_identity: &LocalDirectoryIdentity,
    observe: &mut impl FnMut(RecoveryStep) -> Result<(), ConnectionOperationExecutionError>,
) -> Result<ConnectionOperationJournalEntry, ConnectionOperationExecutionError> {
    while entry.phase() != target {
        let next = entry
            .next_phase()
            .ok_or_else(|| journal_error(&entry, ConnectionOperationError::InvalidEntry))?;
        directory_identity.revalidate()?;
        entry = repositories
            .journal
            .advance(guard, &entry, next)
            .map_err(|source| journal_error(&entry, source))?;
        observe(RecoveryStep::JournalAdvanced(next))?;
    }
    Ok(entry)
}

fn complete_and_clear(
    repositories: &LocalConnectionOperationRepositories,
    directory_identity: &LocalDirectoryIdentity,
    guard: &mut LocalConnectionOperationGuard,
    entry: ConnectionOperationJournalEntry,
    observe: &mut impl FnMut(RecoveryStep) -> Result<(), ConnectionOperationExecutionError>,
) -> Result<(), ConnectionOperationExecutionError> {
    let entry = advance_to(
        repositories,
        guard,
        entry,
        ConnectionOperationPhase::Complete,
        directory_identity,
        observe,
    )?;
    directory_identity.revalidate()?;
    repositories
        .journal
        .clear_complete(guard, &entry)
        .map_err(|source| journal_error(&entry, source))?;
    observe(RecoveryStep::JournalCleared)
}
