use std::path::PathBuf;

use super::{
    super::super::LocalCredentialStoreError,
    model::{CredentialRevision, CredentialSnapshot},
    mutation::{
        CredentialMutationAction, PreparedAccountSessionMutation, PreparedCredentialMutation,
    },
};
use crate::model_service::{AccountId, ApiCredential, CredentialStore, ProviderId};

pub(super) fn validate_candidate(
    action: CredentialMutationAction,
    candidate: Option<&ApiCredential>,
) -> Result<(), LocalCredentialStoreError> {
    match (action, candidate) {
        (CredentialMutationAction::Add | CredentialMutationAction::Replace, Some(_))
        | (CredentialMutationAction::Remove, None) => Ok(()),
        (CredentialMutationAction::Add | CredentialMutationAction::Replace, None)
        | (CredentialMutationAction::Remove, Some(_)) => {
            Err(LocalCredentialStoreError::InvalidMutation)
        },
    }
}

#[derive(Clone)]
pub(super) struct StoredCredentialSnapshot {
    revision: CredentialRevision,
    entries: Vec<CredentialEntry>,
    credentials: CredentialStore,
    account_sessions: CredentialStore,
}

impl StoredCredentialSnapshot {
    pub(in crate::model_service::local_credentials) fn new(
        revision: CredentialRevision,
        entries: Vec<CredentialEntry>,
    ) -> Result<Self, LocalCredentialStoreError> {
        let mut credentials = CredentialStore::new(entries.iter().map(|entry| {
            (
                (entry.provider.clone(), entry.account.clone()),
                entry.credential.clone(),
            )
        }))
        .map_err(|_| LocalCredentialStoreError::InvalidContents(PathBuf::new()))?;
        let account_sessions = CredentialStore::new(entries.iter().filter_map(|entry| {
            entry.account_session.as_ref().map(|session| {
                (
                    (entry.provider.clone(), entry.account.clone()),
                    session.clone(),
                )
            })
        }))
        .map_err(|_| LocalCredentialStoreError::InvalidContents(PathBuf::new()))?;
        credentials.retain_auxiliary_secret_material(
            entries
                .iter()
                .filter_map(|entry| entry.account_session.clone()),
        );
        Ok(Self {
            revision,
            entries,
            credentials,
            account_sessions,
        })
    }

    pub(in crate::model_service::local_credentials) fn into_credentials(self) -> CredentialStore {
        self.credentials
    }

    pub(super) fn public(self) -> CredentialSnapshot {
        CredentialSnapshot::from_stored(self.revision, self.credentials, self.account_sessions)
    }

    pub(super) const fn revision(&self) -> &CredentialRevision {
        &self.revision
    }

    pub(super) fn entries(&self) -> &[CredentialEntry] {
        &self.entries
    }

    pub(super) fn resolve(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Option<&ApiCredential> {
        self.credentials.resolve(provider, account)
    }

    pub(super) fn resolve_account_session(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Option<&ApiCredential> {
        self.account_sessions.resolve(provider, account)
    }

    pub(super) fn apply(
        &mut self,
        mutation: &PreparedCredentialMutation,
        candidate: Option<&ApiCredential>,
    ) {
        let position = self.entries.iter().position(|entry| {
            &entry.provider == mutation.provider() && &entry.account == mutation.account()
        });
        match mutation.action() {
            CredentialMutationAction::Add => self.entries.push(CredentialEntry {
                provider: mutation.provider().clone(),
                account: mutation.account().clone(),
                credential: candidate.expect("candidate presence was validated").clone(),
                account_session: None,
            }),
            CredentialMutationAction::Replace => {
                self.entries[position.expect("replace presence was validated")].credential =
                    candidate.expect("candidate presence was validated").clone();
            },
            CredentialMutationAction::Remove => {
                self.entries
                    .remove(position.expect("remove presence was validated"));
            },
        }
    }

    pub(super) fn apply_account_session(
        &mut self,
        mutation: &PreparedAccountSessionMutation,
        candidate: &ApiCredential,
    ) {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| {
                &entry.provider == mutation.provider() && &entry.account == mutation.account()
            })
            .expect("account-session preparation requires the model credential to remain present");
        entry.account_session = Some(candidate.clone());
    }
}

#[derive(Clone)]
pub(super) struct CredentialEntry {
    pub(in crate::model_service::local_credentials) provider: ProviderId,
    pub(in crate::model_service::local_credentials) account: AccountId,
    pub(in crate::model_service::local_credentials) credential: ApiCredential,
    pub(in crate::model_service::local_credentials) account_session: Option<ApiCredential>,
}
