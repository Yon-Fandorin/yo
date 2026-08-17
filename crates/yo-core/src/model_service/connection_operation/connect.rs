use std::{error::Error, fmt};

use super::{
    ConnectionOperationError, ConnectionOperationJournalEntry, ConnectionOperationPhase,
    execution::{
        ConnectionOperationExecutionError, LocalConnectionOperationSession, credential_error,
        journal_error, public_error,
    },
};
use crate::{
    AccountId, ApiCredential, CompleteModelBinding, CredentialMutationAction,
    PreparedConnectionMutation, PreparedCredentialMutation, ProviderId,
    model_profile_admission::admit_new_complete_binding, model_service::LocalCredentialStoreError,
};

/// A safe external-connect failure. It never retains or formats candidate credential bytes.
#[derive(Debug)]
pub enum ExternalConnectionError {
    InvalidBindingSet,
    CredentialPreparation(LocalCredentialStoreError),
    JournalPreparation(ConnectionOperationError),
    UnsupportedProfile { target: String },
}

impl fmt::Display for ExternalConnectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBindingSet => formatter.write_str(
                "external connect requires one exact stored Provider-and-Account definition with unique complete bindings or a catalog seed",
            ),
            Self::CredentialPreparation(source) => {
                write!(formatter, "preparing the account credential change failed: {source}")
            },
            Self::JournalPreparation(source) => {
                write!(formatter, "preparing the secret-free connection intent failed: {source}")
            },
            Self::UnsupportedProfile { target } => write!(
                formatter,
                "external connection does not support the resolved profile for {target}"
            ),
        }
    }
}

impl Error for ExternalConnectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CredentialPreparation(source) => Some(source),
            Self::JournalPreparation(source) => Some(source),
            Self::InvalidBindingSet | Self::UnsupportedProfile { .. } => None,
        }
    }
}

/// A secret-free, structurally admitted external connection plan.
pub struct PreparedExternalConnection {
    pub(super) connection: PreparedConnectionMutation,
    pub(super) credential: PreparedCredentialMutation,
    pub(super) bindings: Vec<CompleteModelBinding>,
}

impl PreparedExternalConnection {
    /// Returns the exact, secret-free complete bindings that will be published.
    pub fn bindings(&self) -> &[CompleteModelBinding] {
        &self.bindings
    }

    pub(super) fn new(
        connection: PreparedConnectionMutation,
        credential: PreparedCredentialMutation,
        bindings: Vec<CompleteModelBinding>,
    ) -> Result<Self, ExternalConnectionError> {
        let mut identities = Vec::new();
        let valid = connection.defines_model_connect(
            credential.provider(),
            credential.account(),
            &bindings,
        ) && bindings.iter().all(|complete| {
            let binding = complete.binding();
            credential.provider() == binding.provider_id()
                && credential.account() == binding.account_id()
                && if identities.contains(complete) {
                    false
                } else {
                    identities.push(complete.clone());
                    true
                }
        });
        if !valid {
            return Err(ExternalConnectionError::InvalidBindingSet);
        }
        if let Some(unsupported) = bindings
            .iter()
            .find(|complete| admit_new_complete_binding(complete).is_err())
        {
            return Err(ExternalConnectionError::UnsupportedProfile {
                target: unsupported.binding().selection_reference(),
            });
        }
        let connection = connection.into_journal_mutation();
        Ok(Self {
            connection,
            credential,
            bindings,
        })
    }

    fn new_group_replacement(
        connection: PreparedConnectionMutation,
        credential: PreparedCredentialMutation,
        bindings: Vec<CompleteModelBinding>,
    ) -> Result<Self, ExternalConnectionError> {
        if !connection.defines_group_replacement(
            credential.provider(),
            credential.account(),
            &bindings,
        ) {
            return Err(ExternalConnectionError::InvalidBindingSet);
        }
        if let Some(unsupported) = bindings
            .iter()
            .find(|complete| admit_new_complete_binding(complete).is_err())
        {
            return Err(ExternalConnectionError::UnsupportedProfile {
                target: unsupported.binding().selection_reference(),
            });
        }
        let connection = connection.into_journal_mutation();
        Ok(Self {
            connection,
            credential,
            bindings,
        })
    }

    #[must_use]
    pub fn binding_count(&self) -> usize {
        self.bindings.len()
    }

    /// Exact add-or-replace action prepared from the captured credential repository.
    #[must_use]
    pub const fn credential_action(&self) -> CredentialMutationAction {
        self.credential.action()
    }
}

impl LocalConnectionOperationSession<'_> {
    /// Prepares an external connection without opening or persisting credential bytes.
    pub fn prepare_external_connection(
        &mut self,
        connection: PreparedConnectionMutation,
        bindings: Vec<CompleteModelBinding>,
    ) -> Result<PreparedExternalConnection, ConnectionOperationExecutionError> {
        self.directory_identity.revalidate()?;
        let first = bindings.first().ok_or({
            ConnectionOperationExecutionError::ExternalPreparation(
                ExternalConnectionError::InvalidBindingSet,
            )
        })?;
        let credential = self
            .repositories
            .credentials
            .prepare_set(first.binding().provider_id(), first.binding().account_id())
            .map_err(|source| {
                ConnectionOperationExecutionError::ExternalPreparation(
                    ExternalConnectionError::CredentialPreparation(source),
                )
            })?;
        PreparedExternalConnection::new(connection, credential, bindings)
            .map_err(ConnectionOperationExecutionError::ExternalPreparation)
    }

    /// Prepares one grouped definition import. Catalog and discovery seeds may contain no
    /// immediately routable model, but still publish one pair credential and public revision.
    pub fn prepare_external_definition(
        &mut self,
        connection: PreparedConnectionMutation,
        provider: &ProviderId,
        account: &AccountId,
        bindings: Vec<CompleteModelBinding>,
    ) -> Result<PreparedExternalConnection, ConnectionOperationExecutionError> {
        self.directory_identity.revalidate()?;
        let credential = self
            .repositories
            .credentials
            .prepare_set(provider, account)
            .map_err(|source| {
                ConnectionOperationExecutionError::ExternalPreparation(
                    ExternalConnectionError::CredentialPreparation(source),
                )
            })?;
        PreparedExternalConnection::new_group_replacement(connection, credential, bindings)
            .map_err(ConnectionOperationExecutionError::ExternalPreparation)
    }

    /// Commits one structurally admitted external connect in journal, credential, public order.
    pub fn commit_external_connection(
        &mut self,
        prepared: PreparedExternalConnection,
        candidate: ApiCredential,
    ) -> Result<(), ConnectionOperationExecutionError> {
        self.commit_external_connection_with(prepared, candidate, |_| Ok(()))
    }

    fn commit_external_connection_with(
        &mut self,
        prepared: PreparedExternalConnection,
        candidate: ApiCredential,
        mut observe: impl FnMut(ConnectStep) -> Result<(), ConnectionOperationExecutionError>,
    ) -> Result<(), ConnectionOperationExecutionError> {
        let entry = ConnectionOperationJournalEntry::connect_credential_change(
            prepared.connection,
            prepared.credential,
        )
        .map_err(|source| {
            ConnectionOperationExecutionError::ExternalPreparation(
                ExternalConnectionError::JournalPreparation(source),
            )
        })?;

        self.directory_identity.revalidate()?;
        self.repositories
            .journal
            .publish_intent(&mut self.guard, &entry)
            .map_err(|source| journal_error(&entry, source))?;
        observe(ConnectStep::JournalPublished)?;
        self.directory_identity.revalidate()?;
        let mutation = entry
            .credential_mutation()
            .expect("connect journal entries always contain a credential mutation");
        self.repositories
            .credentials
            .commit(mutation, Some(&candidate))
            .map_err(|source| credential_error(&entry, source))?;
        observe(ConnectStep::CredentialCommitted)?;
        let entry = self.advance_connect_phase(
            entry,
            ConnectionOperationPhase::CredentialCommitted,
            &mut observe,
        )?;
        self.directory_identity.revalidate()?;
        self.repositories
            .connections
            .commit(entry.connection_mutation())
            .map_err(|source| public_error(&entry, source))?;
        observe(ConnectStep::PublicCommitted)?;
        let entry = self.advance_connect_phase(
            entry,
            ConnectionOperationPhase::PublicCommitted,
            &mut observe,
        )?;
        let entry =
            self.advance_connect_phase(entry, ConnectionOperationPhase::Complete, &mut observe)?;
        self.directory_identity.revalidate()?;
        self.repositories
            .journal
            .clear_complete(&mut self.guard, &entry)
            .map_err(|source| journal_error(&entry, source))?;
        observe(ConnectStep::JournalCleared)
    }

    fn advance_connect_phase(
        &mut self,
        entry: ConnectionOperationJournalEntry,
        next: ConnectionOperationPhase,
        observe: &mut impl FnMut(ConnectStep) -> Result<(), ConnectionOperationExecutionError>,
    ) -> Result<ConnectionOperationJournalEntry, ConnectionOperationExecutionError> {
        self.directory_identity.revalidate()?;
        let entry = self
            .repositories
            .journal
            .advance(&mut self.guard, &entry, next)
            .map_err(|source| journal_error(&entry, source))?;
        observe(ConnectStep::JournalAdvanced(next))?;
        Ok(entry)
    }

    #[cfg(test)]
    pub(super) fn commit_external_connection_until(
        &mut self,
        prepared: PreparedExternalConnection,
        candidate: ApiCredential,
        stop: ConnectStep,
    ) -> Result<(), ConnectionOperationExecutionError> {
        self.commit_external_connection_with(prepared, candidate, |step| {
            if step == stop {
                Err(ConnectionOperationExecutionError::InjectedInterruption)
            } else {
                Ok(())
            }
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConnectStep {
    JournalPublished,
    CredentialCommitted,
    JournalAdvanced(ConnectionOperationPhase),
    PublicCommitted,
    JournalCleared,
}

trait BindingReference {
    fn selection_reference(&self) -> String;
}

impl BindingReference for crate::EffectiveModelBinding {
    fn selection_reference(&self) -> String {
        crate::ModelSelection::new(
            self.provider_id().clone(),
            self.account_id().clone(),
            self.model_id().clone(),
        )
        .canonical_reference()
    }
}
