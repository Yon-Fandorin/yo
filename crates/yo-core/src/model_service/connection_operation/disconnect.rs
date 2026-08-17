use std::{error::Error, fmt};

use super::{
    ConnectionOperationError, ConnectionOperationJournalEntry, ConnectionOperationPhase,
    execution::{
        ConnectionOperationExecutionError, LocalConnectionOperationSession, credential_error,
        journal_error, public_error,
    },
};
use crate::model_service::{
    ConnectionRepositoryError, ConnectionRevision, CredentialRevision, LocalCredentialStoreError,
    ModelSelection, PreparedConnectionMutation, PreparedCredentialMutation,
};

/// The credential result derived from the prospective post-disconnect binding set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalDisconnectCredentialAction {
    Preserve,
    Remove,
}

/// A safe external-disconnect preparation failure.
#[derive(Debug)]
pub enum ExternalDisconnectError {
    PublicPreparation(ConnectionRepositoryError),
    CredentialPreparation(LocalCredentialStoreError),
    RequiredCredentialAbsent,
    JournalPreparation(ConnectionOperationError),
}

impl fmt::Display for ExternalDisconnectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PublicPreparation(source) => {
                write!(formatter, "preparing the stored public removal failed: {source}")
            },
            Self::CredentialPreparation(source) => {
                write!(formatter, "preparing the account credential action failed: {source}")
            },
            Self::RequiredCredentialAbsent => formatter.write_str(
                "the selected Provider and Account have no stored credential required by the disconnect plan; inspect the current connection state before retrying",
            ),
            Self::JournalPreparation(source) => {
                write!(formatter, "preparing the secret-free disconnect intent failed: {source}")
            },
        }
    }
}

impl Error for ExternalDisconnectError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::PublicPreparation(source) => Some(source),
            Self::CredentialPreparation(source) => Some(source),
            Self::JournalPreparation(source) => Some(source),
            Self::RequiredCredentialAbsent => None,
        }
    }
}

#[derive(Debug)]
enum PreparedDisconnectCredential {
    Preserve(CredentialRevision),
    Remove(PreparedCredentialMutation),
}

/// One secret-free public-first disconnect plan prepared under the retained operation lane.
#[derive(Debug)]
pub struct PreparedExternalDisconnect {
    connection: PreparedConnectionMutation,
    credential: PreparedDisconnectCredential,
}

impl PreparedExternalDisconnect {
    #[must_use]
    pub const fn credential_action(&self) -> ExternalDisconnectCredentialAction {
        match &self.credential {
            PreparedDisconnectCredential::Preserve(_) => {
                ExternalDisconnectCredentialAction::Preserve
            },
            PreparedDisconnectCredential::Remove(_) => ExternalDisconnectCredentialAction::Remove,
        }
    }
}

impl LocalConnectionOperationSession<'_> {
    /// Binds one exact public removal to the credential result derived by the caller's complete
    /// prospective catalog. No repository bytes change during preparation.
    pub fn prepare_external_disconnect(
        &mut self,
        expected_connection_revision: &ConnectionRevision,
        selection: &ModelSelection,
        action: ExternalDisconnectCredentialAction,
    ) -> Result<PreparedExternalDisconnect, ConnectionOperationExecutionError> {
        self.directory_identity.revalidate()?;
        let snapshot = self
            .repositories
            .connections
            .capture()
            .map_err(ConnectionOperationExecutionError::PublicCapture)?;
        if snapshot.revision() != expected_connection_revision {
            return Err(disconnect_preparation_error(
                ExternalDisconnectError::PublicPreparation(ConnectionRepositoryError::Conflict {
                    expected: expected_connection_revision.clone(),
                    observed: snapshot.revision().clone(),
                }),
            ));
        }
        let connection = snapshot.prepare_model_remove(selection).map_err(|source| {
            disconnect_preparation_error(ExternalDisconnectError::PublicPreparation(source))
        })?;
        let credential = match action {
            ExternalDisconnectCredentialAction::Preserve => {
                let snapshot = self.repositories.credentials.capture().map_err(|source| {
                    disconnect_preparation_error(ExternalDisconnectError::CredentialPreparation(
                        source,
                    ))
                })?;
                if snapshot
                    .resolve(selection.provider(), selection.account())
                    .is_none()
                {
                    return Err(disconnect_preparation_error(
                        ExternalDisconnectError::RequiredCredentialAbsent,
                    ));
                }
                PreparedDisconnectCredential::Preserve(snapshot.revision().clone())
            },
            ExternalDisconnectCredentialAction::Remove => {
                let mutation = self
                    .repositories
                    .credentials
                    .prepare_remove(selection.provider(), selection.account())
                    .map_err(|source| {
                        disconnect_preparation_error(
                            ExternalDisconnectError::CredentialPreparation(source),
                        )
                    })?
                    .ok_or_else(|| {
                        disconnect_preparation_error(
                            ExternalDisconnectError::RequiredCredentialAbsent,
                        )
                    })?;
                PreparedDisconnectCredential::Remove(mutation)
            },
        };
        Ok(PreparedExternalDisconnect {
            connection,
            credential,
        })
    }

    /// Commits one prepared disconnect in journal, public, and optional credential order.
    pub fn commit_external_disconnect(
        &mut self,
        prepared: PreparedExternalDisconnect,
    ) -> Result<(), ConnectionOperationExecutionError> {
        self.commit_external_disconnect_with(prepared, |_| Ok(()))
    }

    fn commit_external_disconnect_with(
        &mut self,
        prepared: PreparedExternalDisconnect,
        mut observe: impl FnMut(DisconnectStep) -> Result<(), ConnectionOperationExecutionError>,
    ) -> Result<(), ConnectionOperationExecutionError> {
        let entry = match prepared.credential {
            PreparedDisconnectCredential::Preserve(revision) => {
                ConnectionOperationJournalEntry::disconnect_preserve(prepared.connection, revision)
            },
            PreparedDisconnectCredential::Remove(mutation) => {
                ConnectionOperationJournalEntry::disconnect_remove(prepared.connection, mutation)
            },
        }
        .map_err(|source| {
            disconnect_preparation_error(ExternalDisconnectError::JournalPreparation(source))
        })?;

        self.directory_identity.revalidate()?;
        self.repositories
            .journal
            .publish_intent(&mut self.guard, &entry)
            .map_err(|source| journal_error(&entry, source))?;
        observe(DisconnectStep::JournalPublished)?;

        self.directory_identity.revalidate()?;
        self.repositories
            .connections
            .commit(entry.connection_mutation())
            .map_err(|source| public_error(&entry, source))?;
        observe(DisconnectStep::PublicCommitted)?;
        let mut entry = self.advance_disconnect_phase(
            entry,
            ConnectionOperationPhase::PublicCommitted,
            &mut observe,
        )?;

        if let Some(mutation) = entry.credential_mutation() {
            self.directory_identity.revalidate()?;
            self.repositories
                .credentials
                .commit(mutation, None)
                .map_err(|source| credential_error(&entry, source))?;
            observe(DisconnectStep::CredentialRemoved)?;
            entry = self.advance_disconnect_phase(
                entry,
                ConnectionOperationPhase::CredentialRemoved,
                &mut observe,
            )?;
        }
        let entry =
            self.advance_disconnect_phase(entry, ConnectionOperationPhase::Complete, &mut observe)?;
        self.directory_identity.revalidate()?;
        self.repositories
            .journal
            .clear_complete(&mut self.guard, &entry)
            .map_err(|source| journal_error(&entry, source))?;
        observe(DisconnectStep::JournalCleared)
    }

    fn advance_disconnect_phase(
        &mut self,
        entry: ConnectionOperationJournalEntry,
        next: ConnectionOperationPhase,
        observe: &mut impl FnMut(DisconnectStep) -> Result<(), ConnectionOperationExecutionError>,
    ) -> Result<ConnectionOperationJournalEntry, ConnectionOperationExecutionError> {
        self.directory_identity.revalidate()?;
        let entry = self
            .repositories
            .journal
            .advance(&mut self.guard, &entry, next)
            .map_err(|source| journal_error(&entry, source))?;
        observe(DisconnectStep::JournalAdvanced(next))?;
        Ok(entry)
    }

    #[cfg(test)]
    pub(super) fn commit_external_disconnect_until(
        &mut self,
        prepared: PreparedExternalDisconnect,
        stop: DisconnectStep,
    ) -> Result<(), ConnectionOperationExecutionError> {
        self.commit_external_disconnect_with(prepared, |step| {
            if step == stop {
                Err(ConnectionOperationExecutionError::InjectedInterruption)
            } else {
                Ok(())
            }
        })
    }
}

fn disconnect_preparation_error(
    source: ExternalDisconnectError,
) -> ConnectionOperationExecutionError {
    ConnectionOperationExecutionError::ExternalDisconnectPreparation(source)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DisconnectStep {
    JournalPublished,
    PublicCommitted,
    JournalAdvanced(ConnectionOperationPhase),
    CredentialRemoved,
    JournalCleared,
}
