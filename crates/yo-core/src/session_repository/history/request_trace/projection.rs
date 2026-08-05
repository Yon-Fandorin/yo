use super::{
    StoredBindingCacheState, StoredBindingCloseReason, StoredBindingTransition,
    StoredBindingTransitionMode, StoredExchangeDirection, StoredExchangeKind,
    StoredRequestDetailAvailability, StoredRequestTraceEntry, StoredRequestTraceRecord,
};
use crate::{
    BackendIdentity,
    journal::codec::{
        BindingCloseReason, CacheState, DetailAvailability, ExchangeDirection, ExchangeKind,
        JournalRecord, RecoveredJournal, TransitionMode, VersionedIdentity,
    },
};

pub(super) fn project(recovered: &RecoveredJournal) -> Vec<StoredRequestTraceEntry> {
    recovered
        .records()
        .iter()
        .filter_map(|entry| {
            let sequence = entry.journal_sequence()?;
            let record = project_record(entry.record())?;
            Some(StoredRequestTraceEntry::new(sequence, record))
        })
        .collect()
}

fn project_record(record: &JournalRecord) -> Option<StoredRequestTraceRecord> {
    match record {
        JournalRecord::BackendBindingOpened(binding) => {
            Some(StoredRequestTraceRecord::BindingOpened {
                epoch: binding.epoch(),
                backend_kind: binding.backend_kind().to_owned(),
                backend_version: binding.backend_version().to_owned(),
                binding_identity: identity(binding.binding_identity()),
                model_identity: identity(binding.model_identity()),
                session_locator: identity(binding.session_locator()),
                transition: StoredBindingTransition::new(
                    transition_mode(binding.transition().mode()),
                    cache_state(binding.transition().cache()),
                    binding.transition().source_anchor_sequence(),
                ),
            })
        },
        JournalRecord::BackendBindingClosed(binding) => {
            Some(StoredRequestTraceRecord::BindingClosed {
                epoch: binding.epoch(),
                reason: close_reason(binding.reason()),
            })
        },
        JournalRecord::BackendExchangeObserved(exchange) => {
            Some(StoredRequestTraceRecord::ExchangeObserved {
                epoch: exchange.epoch(),
                operation_id: exchange.operation_id().as_uuid(),
                kind: exchange_kind(exchange.kind()),
                direction: exchange_direction(exchange.direction()),
                payload_schema: exchange.payload_schema().to_owned(),
                correlation_sequence: exchange.correlation_sequence(),
                exchange_identity: exchange.exchange_identity().map(identity),
                detail_availability: detail_availability(exchange.detail_availability()),
            })
        },
        JournalRecord::BackendRequestAccepted(request) => {
            Some(StoredRequestTraceRecord::RequestAccepted {
                epoch: request.epoch(),
                turn_id: request.turn_id(),
                operation_id: request.operation_id().as_uuid(),
                exchange_sequence: request.exchange_sequence(),
                request_identity: identity(request.request_identity()),
            })
        },
        JournalRecord::BackendResumableOutcome(outcome) => {
            Some(StoredRequestTraceRecord::ResumableOutcome {
                epoch: outcome.epoch(),
                turn_id: outcome.turn_id(),
                accepted_request_sequence: outcome.accepted_request_sequence(),
                outcome_identity: outcome.outcome_identity().map(identity),
            })
        },
        JournalRecord::ContinuationAnchor(anchor) => {
            Some(StoredRequestTraceRecord::ContinuationAnchor {
                epoch: anchor.epoch(),
                accepted_request_sequence: anchor.accepted_request_sequence(),
                resumable_outcome_sequence: anchor.resumable_outcome_sequence(),
                journal_boundary: anchor.journal_boundary(),
            })
        },
        JournalRecord::SessionDescriptor(_)
        | JournalRecord::CommandCommitted(_)
        | JournalRecord::EventCommitted(_)
        | JournalRecord::MessageReset(_)
        | JournalRecord::MessageSegment(_)
        | JournalRecord::MessageEnded(_) => None,
    }
}

fn identity(value: &VersionedIdentity) -> BackendIdentity {
    BackendIdentity::new(value.schema(), value.value())
}

pub(super) const fn exchange_kind(value: ExchangeKind) -> StoredExchangeKind {
    match value {
        ExchangeKind::Request => StoredExchangeKind::Request,
        ExchangeKind::Response => StoredExchangeKind::Response,
        ExchangeKind::Notification => StoredExchangeKind::Notification,
        ExchangeKind::ServerRequest => StoredExchangeKind::ServerRequest,
        ExchangeKind::Retry => StoredExchangeKind::Retry,
        ExchangeKind::TerminalOutcome => StoredExchangeKind::TerminalOutcome,
    }
}

pub(super) const fn exchange_direction(value: ExchangeDirection) -> StoredExchangeDirection {
    match value {
        ExchangeDirection::YoToBackend => StoredExchangeDirection::YoToBackend,
        ExchangeDirection::BackendToYo => StoredExchangeDirection::BackendToYo,
    }
}

pub(super) const fn detail_availability(
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

pub(super) const fn transition_mode(value: TransitionMode) -> StoredBindingTransitionMode {
    match value {
        TransitionMode::Initial => StoredBindingTransitionMode::Initial,
        TransitionMode::ExactReplay => StoredBindingTransitionMode::ExactReplay,
        TransitionMode::LossyHandoff => StoredBindingTransitionMode::LossyHandoff,
    }
}

pub(super) const fn cache_state(value: CacheState) -> StoredBindingCacheState {
    match value {
        CacheState::NotApplicable => StoredBindingCacheState::NotApplicable,
        CacheState::Lost => StoredBindingCacheState::Lost,
        CacheState::Unknown => StoredBindingCacheState::Unknown,
    }
}

pub(super) const fn close_reason(value: BindingCloseReason) -> StoredBindingCloseReason {
    match value {
        BindingCloseReason::Replaced => StoredBindingCloseReason::Replaced,
        BindingCloseReason::Revoked => StoredBindingCloseReason::Revoked,
        BindingCloseReason::Exhausted => StoredBindingCloseReason::Exhausted,
    }
}
