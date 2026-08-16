use std::{
    error::Error,
    fmt, thread,
    time::{Duration, Instant},
};

use super::{
    ConnectionOperationError, ConnectionOperationJournalEntry, ConnectionOperationPhase,
    execution::{
        ConnectionOperationExecutionError, LocalConnectionOperationSession, credential_error,
        journal_error, public_error,
    },
};
use crate::{
    ApiCredential, ApiDialect, CompleteModelBinding, ConnectorFailureKind,
    CredentialMutationAction, ModelConnectorCancellation, ModelConnectorEvent,
    ModelConnectorInputItem, ModelConnectorInputRole, ModelConnectorLimits, ModelConnectorPoll,
    ModelConnectorRequest, ModelConnectorStream, ModelConnectorTerminal,
    OpenAiChatCompletionsConnector, OpenAiResponsesConnector, PreparedConnectionMutation,
    PreparedCredentialMutation, RequestToolExposure,
    model_profile_admission::{admit_explicit_model_profile, admit_new_complete_binding},
    model_service::LocalCredentialStoreError,
};

const CONNECTION_VERIFICATION_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// A safe external-connect failure. It never retains or formats candidate credential bytes.
#[derive(Debug)]
pub enum ExternalConnectionError {
    InvalidVerificationSet,
    CredentialPreparation(LocalCredentialStoreError),
    JournalPreparation(ConnectionOperationError),
    UnsupportedProfile {
        target: String,
    },
    Verification {
        target: String,
        kind: ConnectorFailureKind,
    },
    CacheAffinityPreparation,
}

impl fmt::Display for ExternalConnectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidVerificationSet => formatter.write_str(
                "external connect requires a non-empty, unique complete binding set for one Provider and Account",
            ),
            Self::CredentialPreparation(source) => {
                write!(formatter, "preparing the account credential change failed: {source}")
            },
            Self::JournalPreparation(source) => {
                write!(formatter, "preparing the secret-free connection intent failed: {source}")
            },
            Self::UnsupportedProfile { target } => write!(
                formatter,
                "external connection verification does not support the resolved profile for {target}"
            ),
            Self::Verification { target, kind } => write!(
                formatter,
                "external connection verification failed for {target} ({kind:?})"
            ),
            Self::CacheAffinityPreparation => formatter.write_str(
                "preparing the Kimi Code verification cache-affinity context failed",
            ),
        }
    }
}

impl Error for ExternalConnectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CredentialPreparation(source) => Some(source),
            Self::JournalPreparation(source) => Some(source),
            Self::InvalidVerificationSet
            | Self::UnsupportedProfile { .. }
            | Self::Verification { .. }
            | Self::CacheAffinityPreparation => None,
        }
    }
}

/// A secret-free external connection plan awaiting candidate-only verification.
pub struct PreparedExternalConnection {
    pub(super) config_snapshot_digest: String,
    pub(super) connection: PreparedConnectionMutation,
    pub(super) credential: PreparedCredentialMutation,
    pub(super) bindings: Vec<CompleteModelBinding>,
}

impl PreparedExternalConnection {
    /// Returns the exact, secret-free complete bindings awaiting verification.
    pub fn verification_bindings(&self) -> &[CompleteModelBinding] {
        &self.bindings
    }

    pub(super) fn new(
        config_snapshot_digest: String,
        connection: PreparedConnectionMutation,
        credential: PreparedCredentialMutation,
        bindings: Vec<CompleteModelBinding>,
    ) -> Result<Self, ExternalConnectionError> {
        let mut identities = Vec::new();
        let valid = bindings.first().is_some_and(|first| {
            let first = first.binding();
            credential.provider() == first.provider_id()
                && credential.account() == first.account_id()
                && bindings.iter().all(|complete| {
                    let binding = complete.binding();
                    binding.provider_id() == first.provider_id()
                        && binding.account_id() == first.account_id()
                        && if identities.contains(complete) {
                            false
                        } else {
                            identities.push(complete.clone());
                            true
                        }
                })
        });
        if !valid {
            return Err(ExternalConnectionError::InvalidVerificationSet);
        }
        if let Some(unsupported) = bindings
            .iter()
            .find(|complete| admit_new_complete_binding(complete).is_err())
        {
            return Err(ExternalConnectionError::UnsupportedProfile {
                target: unsupported.binding().selection_reference(),
            });
        }
        Ok(Self {
            config_snapshot_digest,
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

/// A candidate credential and exact plan that passed every required binding verification.
pub struct VerifiedExternalConnection {
    pub(super) prepared: PreparedExternalConnection,
    pub(super) candidate: ApiCredential,
}

impl LocalConnectionOperationSession<'_> {
    /// Prepares a candidate-only external connection without persisting credential bytes.
    pub fn prepare_external_connection(
        &mut self,
        config_snapshot_digest: impl Into<String>,
        connection: PreparedConnectionMutation,
        bindings: Vec<CompleteModelBinding>,
    ) -> Result<PreparedExternalConnection, ConnectionOperationExecutionError> {
        self.directory_identity.revalidate()?;
        let first = bindings.first().ok_or({
            ConnectionOperationExecutionError::ExternalPreparation(
                ExternalConnectionError::InvalidVerificationSet,
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
        PreparedExternalConnection::new(
            config_snapshot_digest.into(),
            connection,
            credential,
            bindings,
        )
        .map_err(ConnectionOperationExecutionError::ExternalPreparation)
    }

    /// Commits one verified external connect in journal, credential, public order.
    pub fn commit_verified_external_connection(
        &mut self,
        verified: VerifiedExternalConnection,
    ) -> Result<(), ConnectionOperationExecutionError> {
        self.commit_verified_external_connection_with(verified, |_| Ok(()))
    }

    fn commit_verified_external_connection_with(
        &mut self,
        verified: VerifiedExternalConnection,
        mut observe: impl FnMut(ConnectStep) -> Result<(), ConnectionOperationExecutionError>,
    ) -> Result<(), ConnectionOperationExecutionError> {
        let VerifiedExternalConnection {
            prepared,
            candidate,
        } = verified;
        let entry = ConnectionOperationJournalEntry::connect_credential_change(
            prepared.config_snapshot_digest,
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
    pub(super) fn commit_verified_external_connection_until(
        &mut self,
        verified: VerifiedExternalConnection,
        stop: ConnectStep,
    ) -> Result<(), ConnectionOperationExecutionError> {
        self.commit_verified_external_connection_with(verified, |step| {
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

/// Verifies every captured binding with only the in-memory candidate credential.
pub fn verify_external_connection(
    prepared: PreparedExternalConnection,
    candidate: ApiCredential,
) -> Result<VerifiedExternalConnection, ExternalConnectionError> {
    let cache_hint = prepared
        .bindings
        .iter()
        .any(|complete| {
            admit_new_complete_binding(complete)
                .is_ok_and(|admitted| admitted.requires_cache_affinity_hint())
        })
        .then(|| {
            crate::SessionId::new()
                .map(crate::model_connector::ModelCacheAffinityHint::for_verification)
                .map_err(|_| ExternalConnectionError::CacheAffinityPreparation)
        })
        .transpose()?;
    verify_external_connection_with(prepared, candidate, |complete, candidate| {
        verify_complete_binding(complete, candidate, cache_hint.as_ref())
    })
}

fn verify_external_connection_with(
    prepared: PreparedExternalConnection,
    candidate: ApiCredential,
    mut verify: impl FnMut(&CompleteModelBinding, &ApiCredential) -> Result<(), ConnectorFailureKind>,
) -> Result<VerifiedExternalConnection, ExternalConnectionError> {
    for complete in &prepared.bindings {
        let target = complete.binding().selection_reference();
        if admit_new_complete_binding(complete).is_err() {
            return Err(ExternalConnectionError::UnsupportedProfile { target });
        }
        verify(complete, &candidate)
            .map_err(|kind| ExternalConnectionError::Verification { target, kind })?;
    }
    Ok(VerifiedExternalConnection {
        prepared,
        candidate,
    })
}

fn verify_complete_binding(
    complete: &CompleteModelBinding,
    candidate: &ApiCredential,
    cache_hint: Option<&crate::model_connector::ModelCacheAffinityHint>,
) -> Result<(), ConnectorFailureKind> {
    let request = verification_request_with_cache(complete, cache_hint)?;
    let limits = verification_limits();
    let cancellation = ModelConnectorCancellation::new();
    let mut stream = match complete.binding().api_dialect() {
        ApiDialect::OpenAiResponses => {
            OpenAiResponsesConnector::new(complete.binding(), candidate.clone(), limits)
                .and_then(|connector| connector.start(request, cancellation.clone()))
        },
        ApiDialect::OpenAiChatCompletions => {
            OpenAiChatCompletionsConnector::new(complete.binding(), candidate.clone(), limits)
                .and_then(|connector| connector.start(request, cancellation.clone()))
        },
        ApiDialect::KimiChatCompletions => {
            crate::KimiChatCompletionsConnector::new(complete, candidate.clone(), limits)
                .and_then(|connector| connector.start(request, cancellation.clone()))
        },
    }
    .map_err(|error| error.kind())?;

    verify_semantic_stream(
        &mut stream,
        Instant::now() + CONNECTION_VERIFICATION_TIMEOUT,
    )
}

#[cfg(test)]
pub(super) fn verification_request(
    complete: &CompleteModelBinding,
) -> Result<ModelConnectorRequest, ConnectorFailureKind> {
    verification_request_with_cache(complete, None)
}

pub(super) fn verification_request_with_cache(
    complete: &CompleteModelBinding,
    cache_hint: Option<&crate::model_connector::ModelCacheAffinityHint>,
) -> Result<ModelConnectorRequest, ConnectorFailureKind> {
    let reasoning = admit_explicit_model_profile(complete.profile())
        .map_err(|_| ConnectorFailureKind::Configuration)?
        .reasoning_effort();
    let request = ModelConnectorRequest::new(
        vec![ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
            content: "Reply briefly to confirm this model connection.".to_owned(),
            refusal: None,
        }],
        RequestToolExposure::disabled(),
        match complete.binding().api_dialect() {
            ApiDialect::KimiChatCompletions => complete.profile().context().max_output_tokens(),
            ApiDialect::OpenAiResponses | ApiDialect::OpenAiChatCompletions => {
                complete.profile().context().max_output_tokens().min(32)
            },
        },
        reasoning,
    )
    .map_err(|error| error.kind())?;
    Ok(match cache_hint {
        Some(hint) => request.with_cache_affinity_hint(hint.clone()),
        None => request,
    })
}

pub(super) trait VerificationStreamPort {
    fn poll(&mut self) -> Result<ModelConnectorPoll, ConnectorFailureKind>;
    fn cancel(&mut self);
    fn shutdown(&mut self) -> Result<(), ConnectorFailureKind>;
}

impl VerificationStreamPort for ModelConnectorStream {
    fn poll(&mut self) -> Result<ModelConnectorPoll, ConnectorFailureKind> {
        ModelConnectorStream::poll(self).map_err(|error| error.kind())
    }

    fn cancel(&mut self) {
        ModelConnectorStream::cancel(self);
    }

    fn shutdown(&mut self) -> Result<(), ConnectorFailureKind> {
        ModelConnectorStream::shutdown(self).map_err(|error| error.kind())
    }
}

pub(super) fn verify_semantic_stream(
    stream: &mut impl VerificationStreamPort,
    deadline: Instant,
) -> Result<(), ConnectorFailureKind> {
    let mut message_done = false;
    let result = loop {
        if Instant::now() >= deadline {
            break Err(ConnectorFailureKind::Timeout);
        }
        let poll = match stream.poll() {
            Ok(poll) => poll,
            Err(kind) => break Err(kind),
        };
        match poll {
            ModelConnectorPoll::Pending => thread::park_timeout(Duration::from_millis(5)),
            ModelConnectorPoll::Closed => break Err(ConnectorFailureKind::Protocol),
            ModelConnectorPoll::Event(ModelConnectorEvent::MessageDone { .. }) => {
                message_done = true;
            },
            ModelConnectorPoll::Event(ModelConnectorEvent::FunctionCallStarted { .. })
            | ModelConnectorPoll::Event(ModelConnectorEvent::FunctionArgumentsDelta { .. })
            | ModelConnectorPoll::Event(ModelConnectorEvent::FunctionCallDone { .. }) => {
                break Err(ConnectorFailureKind::Protocol);
            },
            ModelConnectorPoll::Event(ModelConnectorEvent::Terminal {
                status: ModelConnectorTerminal::Completed,
                ..
            }) if message_done => break Ok(()),
            ModelConnectorPoll::Event(ModelConnectorEvent::Terminal { .. }) => {
                break Err(ConnectorFailureKind::Protocol);
            },
            ModelConnectorPoll::Event(_) => {},
        }
    };
    stream.cancel();
    let cleanup = stream.shutdown();
    result.and(cleanup)
}

pub(super) fn verification_limits() -> ModelConnectorLimits {
    ModelConnectorLimits {
        absolute_request_timeout: Some(CONNECTION_VERIFICATION_TIMEOUT),
        max_redirects: 1,
        max_error_body_bytes: 16 * 1024,
        max_sse_event_bytes: 64 * 1024,
        max_sse_events: 1_024,
        max_output_items: 16,
        max_response_text_bytes: 64 * 1024,
        max_refusal_bytes: 16 * 1024,
        max_reasoning_bytes: 64 * 1024,
        max_function_argument_bytes: 16 * 1024,
        ..ModelConnectorLimits::default()
    }
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

#[cfg(test)]
pub(super) fn verify_external_connection_for_test(
    prepared: PreparedExternalConnection,
    candidate: ApiCredential,
    verify: impl FnMut(&CompleteModelBinding, &ApiCredential) -> Result<(), ConnectorFailureKind>,
) -> Result<VerifiedExternalConnection, ExternalConnectionError> {
    verify_external_connection_with(prepared, candidate, verify)
}
