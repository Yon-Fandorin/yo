use uuid::Uuid;

use crate::{BackendIdentity, JournalSequence, TurnId};

/// One durable Request diagnostic fact at its semantic Journal position.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredRequestTraceEntry {
    sequence: JournalSequence,
    record: StoredRequestTraceRecord,
}

impl StoredRequestTraceEntry {
    pub(super) const fn new(sequence: JournalSequence, record: StoredRequestTraceRecord) -> Self {
        Self { sequence, record }
    }

    #[must_use]
    pub const fn sequence(&self) -> JournalSequence {
        self.sequence
    }

    #[must_use]
    pub const fn record(&self) -> &StoredRequestTraceRecord {
        &self.record
    }
}

/// Payload-free Request diagnostic facts exposed by stored Session recovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoredRequestTraceRecord {
    BindingOpened {
        epoch: u64,
        backend_kind: String,
        backend_version: String,
        binding_identity: BackendIdentity,
        model_identity: BackendIdentity,
        session_locator: BackendIdentity,
        transition: StoredBindingTransition,
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
}

impl StoredBindingTransition {
    pub(super) const fn new(
        mode: StoredBindingTransitionMode,
        cache: StoredBindingCacheState,
        source_anchor_sequence: Option<JournalSequence>,
    ) -> Self {
        Self {
            mode,
            cache,
            source_anchor_sequence,
        }
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
pub enum StoredBindingCloseReason {
    Replaced,
    Revoked,
    Exhausted,
}
