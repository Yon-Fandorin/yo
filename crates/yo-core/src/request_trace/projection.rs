use super::{
    RequestTraceEntry, RequestTraceRecord, StoredBindingCacheState, StoredBindingCloseReason,
    StoredBindingTransition, StoredBindingTransitionMode, StoredContinuationStrategy,
    StoredExchangeDirection, StoredExchangeKind, StoredReplayExecutor,
    StoredRequestDetailAvailability,
};
use crate::{
    BackendIdentity, ContinuationStrategy, ReplayExecutor,
    journal::codec::{
        BindingCloseReason, CacheState, DetailAvailability, ExchangeDirection, ExchangeKind,
        JournalRecord, RecoveredJournal, TransitionMode, VersionedIdentity,
    },
};

pub(crate) fn project(recovered: &RecoveredJournal) -> Vec<RequestTraceEntry> {
    recovered
        .records()
        .iter()
        .filter_map(|entry| {
            let sequence = entry.journal_sequence()?;
            let record = project_record(entry.record())?;
            Some(RequestTraceEntry::new(sequence, record))
        })
        .collect()
}

pub(crate) fn project_live(
    sequence: crate::JournalSequence,
    record: &crate::journal::SemanticRecord,
) -> Option<RequestTraceEntry> {
    let record = match record {
        crate::journal::SemanticRecord::BackendBindingOpened(binding) => binding_opened(binding),
        crate::journal::SemanticRecord::BackendBindingClosed(binding) => binding_closed(binding),
        crate::journal::SemanticRecord::BackendExchangeObserved(exchange) => {
            exchange_observed(exchange)
        },
        crate::journal::SemanticRecord::BackendRequestAccepted(request) => {
            request_accepted(request)
        },
        crate::journal::SemanticRecord::ModelReplayDelta(_) => return None,
        crate::journal::SemanticRecord::BackendResumableOutcome(outcome) => {
            resumable_outcome(outcome)
        },
        crate::journal::SemanticRecord::ContinuationAnchor(anchor) => continuation_anchor(anchor),
        crate::journal::SemanticRecord::ContextPolicyChanged(_)
        | crate::journal::SemanticRecord::ContextCheckpoint(_) => return None,
        crate::journal::SemanticRecord::CommandCommitted(_)
        | crate::journal::SemanticRecord::EventCommitted(_) => return None,
    };
    Some(RequestTraceEntry::new(sequence, record))
}

fn binding_opened(binding: &crate::journal::codec::BackendBindingOpened) -> RequestTraceRecord {
    RequestTraceRecord::BindingOpened {
        epoch: binding.epoch(),
        backend_kind: binding.backend_kind().to_owned(),
        backend_version: binding.backend_version().to_owned(),
        binding_identity: identity(binding.binding_identity()),
        model_identity: identity(binding.model_identity()),
        session_locator: identity(binding.session_locator()),
        transition: {
            let transition = StoredBindingTransition::new(
                transition_mode(binding.transition().mode()),
                cache_state(binding.transition().cache()),
                binding.transition().source_anchor_sequence(),
            );
            match binding.transition().source_checkpoint_sequence() {
                Some(sequence) => transition.with_source_checkpoint_sequence(sequence),
                None => transition,
            }
        },
        continuation_strategy: continuation_strategy(binding.continuation_strategy()),
    }
}

fn exchange_observed(
    exchange: &crate::journal::codec::BackendExchangeObserved,
) -> RequestTraceRecord {
    RequestTraceRecord::ExchangeObserved {
        epoch: exchange.epoch(),
        operation_id: exchange.operation_id().as_uuid(),
        kind: exchange_kind(exchange.kind()),
        direction: exchange_direction(exchange.direction()),
        payload_schema: exchange.payload_schema().to_owned(),
        correlation_sequence: exchange.correlation_sequence(),
        exchange_identity: exchange.exchange_identity().map(identity),
        detail_availability: detail_availability(exchange.detail_availability()),
    }
}

fn binding_closed(binding: &crate::journal::codec::BackendBindingClosed) -> RequestTraceRecord {
    RequestTraceRecord::BindingClosed {
        epoch: binding.epoch(),
        reason: close_reason(binding.reason()),
    }
}

fn request_accepted(request: &crate::journal::codec::BackendRequestAccepted) -> RequestTraceRecord {
    RequestTraceRecord::RequestAccepted {
        epoch: request.epoch(),
        turn_id: request.turn_id(),
        operation_id: request.operation_id().as_uuid(),
        exchange_sequence: request.exchange_sequence(),
        request_identity: identity(request.request_identity()),
    }
}

fn resumable_outcome(
    outcome: &crate::journal::codec::BackendResumableOutcome,
) -> RequestTraceRecord {
    RequestTraceRecord::ResumableOutcome {
        epoch: outcome.epoch(),
        turn_id: outcome.turn_id(),
        accepted_request_sequence: outcome.accepted_request_sequence(),
        outcome_identity: outcome.outcome_identity().map(identity),
        replay_delta_sequence: outcome.replay_delta_sequence(),
    }
}

fn continuation_anchor(anchor: &crate::journal::codec::ContinuationAnchor) -> RequestTraceRecord {
    RequestTraceRecord::ContinuationAnchor {
        epoch: anchor.epoch(),
        accepted_request_sequence: anchor.accepted_request_sequence(),
        resumable_outcome_sequence: anchor.resumable_outcome_sequence(),
        journal_boundary: anchor.journal_boundary(),
    }
}

fn project_record(record: &JournalRecord) -> Option<RequestTraceRecord> {
    match record {
        JournalRecord::BackendBindingOpened(binding) => Some(binding_opened(binding)),
        JournalRecord::BackendBindingClosed(binding) => Some(binding_closed(binding)),
        JournalRecord::BackendExchangeObserved(exchange) => Some(exchange_observed(exchange)),
        JournalRecord::BackendRequestAccepted(request) => Some(request_accepted(request)),
        JournalRecord::ModelReplayDelta(_) => None,
        JournalRecord::BackendResumableOutcome(outcome) => Some(resumable_outcome(outcome)),
        JournalRecord::ContinuationAnchor(anchor) => Some(continuation_anchor(anchor)),
        JournalRecord::SessionDescriptor(_)
        | JournalRecord::CommandCommitted(_)
        | JournalRecord::EventCommitted(_)
        | JournalRecord::ContextPolicyChanged(_)
        | JournalRecord::ContextCheckpoint(_)
        | JournalRecord::MessageReset(_)
        | JournalRecord::MessageSegment(_)
        | JournalRecord::MessageEnded(_) => None,
    }
}

fn identity(value: &VersionedIdentity) -> BackendIdentity {
    BackendIdentity::new(value.schema(), value.value())
}

pub(crate) const fn exchange_kind(value: ExchangeKind) -> StoredExchangeKind {
    match value {
        ExchangeKind::Request => StoredExchangeKind::Request,
        ExchangeKind::Response => StoredExchangeKind::Response,
        ExchangeKind::Notification => StoredExchangeKind::Notification,
        ExchangeKind::ServerRequest => StoredExchangeKind::ServerRequest,
        ExchangeKind::Retry => StoredExchangeKind::Retry,
        ExchangeKind::TerminalOutcome => StoredExchangeKind::TerminalOutcome,
    }
}

pub(crate) const fn exchange_direction(value: ExchangeDirection) -> StoredExchangeDirection {
    match value {
        ExchangeDirection::YoToBackend => StoredExchangeDirection::YoToBackend,
        ExchangeDirection::BackendToYo => StoredExchangeDirection::BackendToYo,
    }
}

pub(crate) const fn detail_availability(
    value: DetailAvailability,
) -> StoredRequestDetailAvailability {
    match value {
        DetailAvailability::Persisted => StoredRequestDetailAvailability::Persisted,
        DetailAvailability::Volatile => StoredRequestDetailAvailability::Volatile,
        DetailAvailability::Missing => StoredRequestDetailAvailability::Missing,
        DetailAvailability::Unsupported => StoredRequestDetailAvailability::Unsupported,
        DetailAvailability::Unpersisted => StoredRequestDetailAvailability::Unpersisted,
        DetailAvailability::Redacted => StoredRequestDetailAvailability::Redacted,
    }
}

pub(crate) const fn transition_mode(value: TransitionMode) -> StoredBindingTransitionMode {
    match value {
        TransitionMode::Initial => StoredBindingTransitionMode::Initial,
        TransitionMode::ExactReplay => StoredBindingTransitionMode::ExactReplay,
        TransitionMode::LossyHandoff => StoredBindingTransitionMode::LossyHandoff,
    }
}

pub(crate) const fn cache_state(value: CacheState) -> StoredBindingCacheState {
    match value {
        CacheState::NotApplicable => StoredBindingCacheState::NotApplicable,
        CacheState::Lost => StoredBindingCacheState::Lost,
        CacheState::Unknown => StoredBindingCacheState::Unknown,
    }
}

pub(crate) const fn continuation_strategy(
    value: ContinuationStrategy,
) -> StoredContinuationStrategy {
    match value {
        ContinuationStrategy::ExactReplay { executor, .. } => {
            StoredContinuationStrategy::ExactReplay {
                executor: match executor {
                    ReplayExecutor::LocalClient => StoredReplayExecutor::LocalClient,
                    ReplayExecutor::ManagedServer => StoredReplayExecutor::ManagedServer,
                },
            }
        },
        ContinuationStrategy::BackendManagedState => {
            StoredContinuationStrategy::BackendManagedState
        },
    }
}

pub(crate) const fn close_reason(value: BindingCloseReason) -> StoredBindingCloseReason {
    match value {
        BindingCloseReason::Replaced => StoredBindingCloseReason::Replaced,
        BindingCloseReason::Revoked => StoredBindingCloseReason::Revoked,
        BindingCloseReason::Exhausted => StoredBindingCloseReason::Exhausted,
    }
}
