use uuid::Uuid;

use crate::{BackendIdentity, JournalSequence, TurnId};

pub(crate) mod projection;

pub(crate) use projection::{project as project_recovered, project_live};

/// One payload-free Request diagnostic fact at its semantic Journal position.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestTraceEntry {
    sequence: JournalSequence,
    record: RequestTraceRecord,
}

impl RequestTraceEntry {
    pub(crate) const fn new(sequence: JournalSequence, record: RequestTraceRecord) -> Self {
        Self { sequence, record }
    }

    #[must_use]
    pub const fn sequence(&self) -> JournalSequence {
        self.sequence
    }

    #[must_use]
    pub const fn record(&self) -> &RequestTraceRecord {
        &self.record
    }
}

/// Payload-free Request diagnostic facts shared by live and stored Session readers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RequestTraceRecord {
    BindingOpened {
        epoch: u64,
        backend_kind: String,
        backend_version: String,
        binding_identity: BackendIdentity,
        model_identity: BackendIdentity,
        session_locator: BackendIdentity,
        transition: StoredBindingTransition,
        continuation_strategy: StoredContinuationStrategy,
    },
    BindingClosed {
        epoch: u64,
        reason: StoredBindingCloseReason,
    },
    ExchangeObserved {
        epoch: u64,
        operation_id: Uuid,
        kind: StoredExchangeKind,
        direction: StoredExchangeDirection,
        payload_schema: String,
        correlation_sequence: Option<JournalSequence>,
        exchange_identity: Option<BackendIdentity>,
        detail_availability: StoredRequestDetailAvailability,
    },
    RequestAccepted {
        epoch: u64,
        turn_id: TurnId,
        operation_id: Uuid,
        exchange_sequence: JournalSequence,
        request_identity: BackendIdentity,
    },
    ResumableOutcome {
        epoch: u64,
        turn_id: TurnId,
        accepted_request_sequence: JournalSequence,
        outcome_identity: Option<BackendIdentity>,
        replay_delta_sequence: Option<JournalSequence>,
    },
    ContinuationAnchor {
        epoch: u64,
        accepted_request_sequence: JournalSequence,
        resumable_outcome_sequence: JournalSequence,
        journal_boundary: JournalSequence,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredExchangeKind {
    Request,
    Response,
    Notification,
    ServerRequest,
    Retry,
    TerminalOutcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredExchangeDirection {
    YoToBackend,
    BackendToYo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredRequestDetailAvailability {
    Persisted,
    Volatile,
    Missing,
    Unsupported,
    Unpersisted,
    Redacted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredBindingTransition {
    mode: StoredBindingTransitionMode,
    cache: StoredBindingCacheState,
    source_anchor_sequence: Option<JournalSequence>,
    source_checkpoint_sequence: Option<JournalSequence>,
}

impl StoredBindingTransition {
    pub(crate) const fn new(
        mode: StoredBindingTransitionMode,
        cache: StoredBindingCacheState,
        source_anchor_sequence: Option<JournalSequence>,
    ) -> Self {
        Self {
            mode,
            cache,
            source_anchor_sequence,
            source_checkpoint_sequence: None,
        }
    }

    pub(crate) const fn with_source_checkpoint_sequence(
        mut self,
        source_checkpoint_sequence: JournalSequence,
    ) -> Self {
        self.source_checkpoint_sequence = Some(source_checkpoint_sequence);
        self
    }

    #[must_use]
    pub const fn mode(&self) -> StoredBindingTransitionMode {
        self.mode
    }

    #[must_use]
    pub const fn cache(&self) -> StoredBindingCacheState {
        self.cache
    }

    #[must_use]
    pub const fn source_anchor_sequence(&self) -> Option<JournalSequence> {
        self.source_anchor_sequence
    }

    #[must_use]
    pub const fn source_checkpoint_sequence(&self) -> Option<JournalSequence> {
        self.source_checkpoint_sequence
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredBindingTransitionMode {
    Initial,
    ExactReplay,
    LossyHandoff,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredBindingCacheState {
    NotApplicable,
    Lost,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredContinuationStrategy {
    ExactReplay { executor: StoredReplayExecutor },
    BackendManagedState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredReplayExecutor {
    LocalClient,
    ManagedServer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredBindingCloseReason {
    Replaced,
    Revoked,
    Exhausted,
}
