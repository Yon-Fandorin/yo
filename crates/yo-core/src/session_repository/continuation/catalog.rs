use std::sync::Arc;

use super::{
    model::{
        ForkCapture, SessionForkLimits, StoredSessionForkBoundary, StoredSessionForkCatalog,
        StoredSessionForkSourceKind,
    },
    recovery::StoredSessionContinuationError,
};
use crate::{
    JournalDurability, SessionId,
    journal::codec::HistoricalForkKind,
    session_repository::{
        StoredDiscoveryValidation, StoredSessionReader, StoredSessionSnapshot,
        journal::recover_entries, validate_discovery,
    },
};

pub(crate) fn read_fork_catalog(
    reader: &(impl StoredSessionReader + ?Sized),
    session_id: SessionId,
    limits: SessionForkLimits,
) -> Result<StoredSessionForkCatalog, StoredSessionContinuationError> {
    let entries = match reader
        .read_session_bounded(session_id, limits)
        .map_err(|error| StoredSessionContinuationError::new(error.to_string()))?
    {
        StoredSessionSnapshot::Present(entries) if !entries.is_empty() => entries,
        _ => {
            return Err(StoredSessionContinuationError::new(
                "historical fork capture has no complete Session envelopes",
            ));
        },
    };
    let recovered = recover_entries(session_id, &entries)
        .map_err(|error| StoredSessionContinuationError::new(error.to_string()))?;
    let descriptor = recovered.descriptor().ok_or_else(|| {
        StoredSessionContinuationError::new("historical fork capture has no descriptor")
    })?;
    if descriptor.session_id() != session_id
        || validate_discovery(&entries, descriptor, &recovered)
            != StoredDiscoveryValidation::Consistent
    {
        return Err(StoredSessionContinuationError::new(
            "historical fork capture has inconsistent physical discovery metadata",
        ));
    }
    // 최신 실행 가능한 소스도 검증합니다. 이전의 유효한 접두부가 불확실성을 우회할 수 없습니다.
    recovered
        .validate_fork_capture()
        .map_err(|error| StoredSessionContinuationError::new(error.to_string()))?;
    let (points, truncated) = recovered
        .historical_fork_boundaries(limits.returned_boundaries())
        .map_err(|error| StoredSessionContinuationError::new(error.to_string()))?;
    let boundaries = points
        .into_iter()
        .map(|point| StoredSessionForkBoundary {
            source_kind: match point.source {
                HistoricalForkKind::Anchor => StoredSessionForkSourceKind::Anchor,
                HistoricalForkKind::Checkpoint => StoredSessionForkSourceKind::Checkpoint,
                HistoricalForkKind::InitialFork => StoredSessionForkSourceKind::InitialFork,
            },
            journal_cutoff: point.cutoff,
            logical_cutoff: point.logical_cutoff,
            binding_epoch: point.binding_epoch,
            context_epoch: point.context_epoch,
            model_label: point.model_label,
            input_excerpt: point.input_excerpt,
            record_count: point.record_count,
        })
        .collect();
    let durability = JournalDurability::Durable {
        journal_sequence: recovered.journal_cutoff(),
        repository_sequence: entries.last().expect("nonempty capture").sequence(),
    };
    Ok(StoredSessionForkCatalog {
        capture: Arc::new(ForkCapture {
            recovered,
            durability,
        }),
        boundaries,
        truncated,
    })
}
