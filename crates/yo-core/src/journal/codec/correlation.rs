use uuid::{Uuid, Variant, Version};

use crate::{JournalSequence, SubmissionId, TurnId};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct OperationId(Uuid);

impl OperationId {
    pub(crate) fn from_uuid(value: Uuid) -> Option<Self> {
        (value.get_version() == Some(Version::Random) && value.get_variant() == Variant::RFC4122)
            .then_some(Self(value))
    }

    pub(crate) const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl From<SubmissionId> for OperationId {
    fn from(value: SubmissionId) -> Self {
        Self(value.as_uuid())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VersionedIdentity {
    schema: String,
    value: String,
}

impl VersionedIdentity {
    pub(crate) fn new(schema: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            schema: schema.into(),
            value: value.into(),
        }
    }

    pub(crate) fn schema(&self) -> &str {
        &self.schema
    }

    pub(crate) fn value(&self) -> &str {
        &self.value
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExchangeKind {
    Request,
    Response,
    Notification,
    ServerRequest,
    Retry,
    TerminalOutcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExchangeDirection {
    YoToBackend,
    BackendToYo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DetailAvailability {
    Persisted,
    Volatile,
    Missing,
    Unsupported,
    Unpersisted,
    Redacted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BackendExchangeObserved {
    epoch: u64,
    operation_id: OperationId,
    kind: ExchangeKind,
    direction: ExchangeDirection,
    payload_schema: String,
    correlation_sequence: Option<JournalSequence>,
    exchange_identity: Option<VersionedIdentity>,
    detail_availability: DetailAvailability,
}

impl BackendExchangeObserved {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        epoch: u64,
        operation_id: OperationId,
        kind: ExchangeKind,
        direction: ExchangeDirection,
        payload_schema: impl Into<String>,
        correlation_sequence: Option<JournalSequence>,
        exchange_identity: Option<VersionedIdentity>,
        detail_availability: DetailAvailability,
    ) -> Self {
        Self {
            epoch,
            operation_id,
            kind,
            direction,
            payload_schema: payload_schema.into(),
            correlation_sequence,
            exchange_identity,
            detail_availability,
        }
    }

    pub(crate) const fn epoch(&self) -> u64 {
        self.epoch
    }

    pub(crate) const fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    pub(crate) const fn kind(&self) -> ExchangeKind {
        self.kind
    }

    pub(crate) const fn direction(&self) -> ExchangeDirection {
        self.direction
    }

    pub(crate) fn payload_schema(&self) -> &str {
        &self.payload_schema
    }

    pub(crate) const fn correlation_sequence(&self) -> Option<JournalSequence> {
        self.correlation_sequence
    }

    pub(crate) const fn exchange_identity(&self) -> Option<&VersionedIdentity> {
        self.exchange_identity.as_ref()
    }

    pub(crate) const fn detail_availability(&self) -> DetailAvailability {
        self.detail_availability
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransitionMode {
    Initial,
    ExactReplay,
    LossyHandoff,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CacheState {
    NotApplicable,
    Lost,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BindingTransition {
    mode: TransitionMode,
    cache: CacheState,
    source_anchor_sequence: Option<JournalSequence>,
}

impl BindingTransition {
    pub(crate) const fn new(
        mode: TransitionMode,
        cache: CacheState,
        source_anchor_sequence: Option<JournalSequence>,
    ) -> Self {
        Self {
            mode,
            cache,
            source_anchor_sequence,
        }
    }

    pub(crate) const fn mode(&self) -> TransitionMode {
        self.mode
    }

    pub(crate) const fn cache(&self) -> CacheState {
        self.cache
    }

    pub(crate) const fn source_anchor_sequence(&self) -> Option<JournalSequence> {
        self.source_anchor_sequence
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BackendBindingOpened {
    epoch: u64,
    backend_kind: String,
    backend_version: String,
    binding_identity: VersionedIdentity,
    model_identity: VersionedIdentity,
    session_locator: VersionedIdentity,
    transition: BindingTransition,
}

impl BackendBindingOpened {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        epoch: u64,
        backend_kind: impl Into<String>,
        backend_version: impl Into<String>,
        binding_identity: VersionedIdentity,
        model_identity: VersionedIdentity,
        session_locator: VersionedIdentity,
        transition: BindingTransition,
    ) -> Self {
        Self {
            epoch,
            backend_kind: backend_kind.into(),
            backend_version: backend_version.into(),
            binding_identity,
            model_identity,
            session_locator,
            transition,
        }
    }

    pub(crate) const fn epoch(&self) -> u64 {
        self.epoch
    }

    pub(crate) fn backend_kind(&self) -> &str {
        &self.backend_kind
    }

    pub(crate) fn backend_version(&self) -> &str {
        &self.backend_version
    }

    pub(crate) const fn binding_identity(&self) -> &VersionedIdentity {
        &self.binding_identity
    }

    pub(crate) const fn model_identity(&self) -> &VersionedIdentity {
        &self.model_identity
    }

    pub(crate) const fn session_locator(&self) -> &VersionedIdentity {
        &self.session_locator
    }

    pub(crate) const fn transition(&self) -> &BindingTransition {
        &self.transition
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BindingCloseReason {
    Replaced,
    Revoked,
    Exhausted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BackendBindingClosed {
    epoch: u64,
    reason: BindingCloseReason,
}

impl BackendBindingClosed {
    pub(crate) const fn new(epoch: u64, reason: BindingCloseReason) -> Self {
        Self { epoch, reason }
    }

    pub(crate) const fn epoch(&self) -> u64 {
        self.epoch
    }

    pub(crate) const fn reason(&self) -> BindingCloseReason {
        self.reason
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BackendRequestAccepted {
    epoch: u64,
    turn_id: TurnId,
    operation_id: OperationId,
    exchange_sequence: JournalSequence,
    request_identity: VersionedIdentity,
}

impl BackendRequestAccepted {
    pub(crate) const fn new(
        epoch: u64,
        turn_id: TurnId,
        operation_id: OperationId,
        exchange_sequence: JournalSequence,
        request_identity: VersionedIdentity,
    ) -> Self {
        Self {
            epoch,
            turn_id,
            operation_id,
            exchange_sequence,
            request_identity,
        }
    }

    pub(crate) const fn epoch(&self) -> u64 {
        self.epoch
    }

    pub(crate) const fn turn_id(&self) -> TurnId {
        self.turn_id
    }

    pub(crate) const fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    pub(crate) const fn exchange_sequence(&self) -> JournalSequence {
        self.exchange_sequence
    }

    pub(crate) const fn request_identity(&self) -> &VersionedIdentity {
        &self.request_identity
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BackendResumableOutcome {
    epoch: u64,
    turn_id: TurnId,
    accepted_request_sequence: JournalSequence,
    outcome_identity: Option<VersionedIdentity>,
}

impl BackendResumableOutcome {
    pub(crate) const fn new(
        epoch: u64,
        turn_id: TurnId,
        accepted_request_sequence: JournalSequence,
        outcome_identity: Option<VersionedIdentity>,
    ) -> Self {
        Self {
            epoch,
            turn_id,
            accepted_request_sequence,
            outcome_identity,
        }
    }

    pub(crate) const fn epoch(&self) -> u64 {
        self.epoch
    }

    pub(crate) const fn turn_id(&self) -> TurnId {
        self.turn_id
    }

    pub(crate) const fn accepted_request_sequence(&self) -> JournalSequence {
        self.accepted_request_sequence
    }

    pub(crate) const fn outcome_identity(&self) -> Option<&VersionedIdentity> {
        self.outcome_identity.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContinuationAnchor {
    epoch: u64,
    accepted_request_sequence: JournalSequence,
    resumable_outcome_sequence: JournalSequence,
    journal_boundary: JournalSequence,
}

impl ContinuationAnchor {
    pub(crate) const fn new(
        epoch: u64,
        accepted_request_sequence: JournalSequence,
        resumable_outcome_sequence: JournalSequence,
        journal_boundary: JournalSequence,
    ) -> Self {
        Self {
            epoch,
            accepted_request_sequence,
            resumable_outcome_sequence,
            journal_boundary,
        }
    }

    pub(crate) const fn epoch(&self) -> u64 {
        self.epoch
    }

    pub(crate) const fn accepted_request_sequence(&self) -> JournalSequence {
        self.accepted_request_sequence
    }

    pub(crate) const fn resumable_outcome_sequence(&self) -> JournalSequence {
        self.resumable_outcome_sequence
    }

    pub(crate) const fn journal_boundary(&self) -> JournalSequence {
        self.journal_boundary
    }
}
