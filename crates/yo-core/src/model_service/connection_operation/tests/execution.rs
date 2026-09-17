use std::{fs, os::unix::fs::symlink};

use super::{
    super::execution::RecoveryStep,
    support::{CANDIDATE_SECRET, Fixture, account, candidate, provider},
};
use crate::model_service::{
    AccountId, ApiCredential, ConnectionCredentialAction, ConnectionOperationError,
    ConnectionOperationExecutionError, ConnectionOperationExecutionOutcome,
    ConnectionOperationPhase, ConnectionOperationRepositoryKind,
    LocalConnectionOperationRepositories, ProviderId, StartupTarget,
};

mod commit;
mod noop;
mod observations;

fn repositories(fixture: &Fixture) -> LocalConnectionOperationRepositories {
    LocalConnectionOperationRepositories::from_paths(
        fixture.connections.path(),
        fixture.credentials.path(),
        fixture.journal.path(),
    )
    .unwrap()
}

fn recover(
    fixture: &Fixture,
) -> Result<ConnectionOperationExecutionOutcome, ConnectionOperationExecutionError> {
    repositories(fixture)
        .acquire()
        .unwrap()
        .recover_pending_operation()
}

fn publish_at_phase(
    fixture: &Fixture,
    entry: &super::super::ConnectionOperationJournalEntry,
    target: ConnectionOperationPhase,
) {
    let mut guard = fixture.operation_guard();
    let mut current = entry.clone();
    fixture
        .journal
        .publish_intent(&mut guard, &current)
        .unwrap();
    while current.phase() != target {
        let next = current.next_phase().unwrap();
        current = fixture.journal.advance(&mut guard, &current, next).unwrap();
    }
}
