use yo_core::{
    BackendIdentity, JournalSequence, SessionDescriptor,
    session_repository::{
        StoredBindingCacheState, StoredBindingCloseReason, StoredBindingTransitionMode,
        StoredContinuationStrategy, StoredExchangeDirection, StoredExchangeKind,
        StoredReplayExecutor, StoredRequestDetailAvailability, StoredRequestTraceRecord,
        StoredSessionContinuity, StoredSessionHistory, StoredSessionRecovery,
    },
};

use super::{continuity_text, cutoff_text, discovery_text, recovery_text};

pub(super) fn project(history: &StoredSessionHistory) -> String {
    project_parts(
        history.descriptor(),
        history.journal_cutoff(),
        history.recovery(),
        history.continuity(),
        history.discovery_consistent(),
        history
            .request_trace()
            .iter()
            .map(|entry| (entry.sequence().get(), entry.record())),
    )
}

pub(super) fn project_parts<'record>(
    descriptor: &SessionDescriptor,
    journal_cutoff: Option<JournalSequence>,
    recovery: StoredSessionRecovery,
    continuity: StoredSessionContinuity,
    discovery_consistent: bool,
    records: impl IntoIterator<Item = (u64, &'record StoredRequestTraceRecord)>,
) -> String {
    let mut output = format!(
        "Stored Session Request diagnostic\n\
         session={}\n\
         workspace={}\n\
         journal_cutoff={}\n\
         message_recovery={}\n\
         durability_continuity={}\n\
         discovery={}\n\
         observation_boundary=validated-session-journal-correlation\n\
         request_audit_detail=unavailable(reason=no-audit-reader)",
        descriptor.session_id(),
        descriptor.workspace_path(),
        cutoff_text(journal_cutoff),
        recovery_text(recovery),
        continuity_text(continuity),
        discovery_text(discovery_consistent),
    );
    for (sequence, record) in records {
        output.push_str("\n\n");
        output.push_str(&format_record(sequence, record));
    }
    output
}

pub(in crate::runner) fn format_record(sequence: u64, record: &StoredRequestTraceRecord) -> String {
    let prefix = format!("[J#{sequence:03}]");
    match record {
        StoredRequestTraceRecord::BindingOpened {
            epoch,
            backend_kind,
            backend_version,
            binding_identity,
            model_identity,
            session_locator,
            transition,
            continuation_strategy,
        } => format!(
            "{prefix} binding.opened\n\
             epoch={epoch}\n\
             backend_kind={backend_kind:?}\n\
             backend_version={backend_version:?}\n\
             {}\n\
             {}\n\
             {}\n\
             continuation_strategy={}\n\
             transition={}\n\
             cache={}\n\
             source_anchor_sequence={}",
            identity_text("binding_identity", binding_identity),
            identity_text("model_identity", model_identity),
            identity_text("session_locator", session_locator),
            continuation_strategy_text(*continuation_strategy),
            transition_mode_text(transition.mode()),
            cache_state_text(transition.cache()),
            sequence_text(transition.source_anchor_sequence()),
        ),
        StoredRequestTraceRecord::BindingClosed { epoch, reason } => format!(
            "{prefix} binding.closed\n\
             epoch={epoch}\n\
             reason={}",
            close_reason_text(*reason)
        ),
        StoredRequestTraceRecord::ExchangeObserved {
            epoch,
            operation_id,
            kind,
            direction,
            payload_schema,
            correlation_sequence,
            exchange_identity,
            detail_availability,
        } => {
            let identity = exchange_identity.as_ref().map_or_else(
                || "exchange_identity=none".to_owned(),
                |identity| identity_text("exchange_identity", identity),
            );
            format!(
                "{prefix} exchange.observed\n\
                 epoch={epoch}\n\
                 operation_id={operation_id}\n\
                 kind={}\n\
                 direction={}\n\
                 payload_schema={payload_schema:?}\n\
                 correlation_sequence={}\n\
                 {identity}\n\
                 detail_availability={}",
                exchange_kind_text(*kind),
                exchange_direction_text(*direction),
                sequence_text(*correlation_sequence),
                detail_availability_text(*detail_availability),
            )
        },
        StoredRequestTraceRecord::RequestAccepted {
            epoch,
            turn_id,
            operation_id,
            exchange_sequence,
            request_identity,
        } => format!(
            "{prefix} request.accepted\n\
             epoch={epoch}\n\
             turn={}\n\
             operation_id={operation_id}\n\
             exchange_sequence={}\n\
             {}",
            turn_id.get().get(),
            exchange_sequence.get(),
            identity_text("request_identity", request_identity),
        ),
        StoredRequestTraceRecord::ResumableOutcome {
            epoch,
            turn_id,
            accepted_request_sequence,
            outcome_identity,
            replay_delta_sequence,
        } => {
            let identity = outcome_identity.as_ref().map_or_else(
                || "outcome_identity=none".to_owned(),
                |identity| identity_text("outcome_identity", identity),
            );
            format!(
                "{prefix} outcome.resumable\n\
                 epoch={epoch}\n\
                 turn={}\n\
                 accepted_request_sequence={}\n\
                 replay_delta_sequence={}\n\
                 {identity}",
                turn_id.get().get(),
                accepted_request_sequence.get(),
                sequence_text(*replay_delta_sequence),
            )
        },
        StoredRequestTraceRecord::ContinuationAnchor {
            epoch,
            accepted_request_sequence,
            resumable_outcome_sequence,
            journal_boundary,
        } => format!(
            "{prefix} continuation.anchor\n\
             epoch={epoch}\n\
             accepted_request_sequence={}\n\
             resumable_outcome_sequence={}\n\
             journal_boundary={}",
            accepted_request_sequence.get(),
            resumable_outcome_sequence.get(),
            journal_boundary.get(),
        ),
    }
}

pub(super) const fn continuation_strategy_text(
    strategy: StoredContinuationStrategy,
) -> &'static str {
    match strategy {
        StoredContinuationStrategy::ExactReplay {
            executor: StoredReplayExecutor::LocalClient,
        } => "exact-replay(local-client)",
        StoredContinuationStrategy::ExactReplay {
            executor: StoredReplayExecutor::ManagedServer,
        } => "exact-replay(managed-server)",
        StoredContinuationStrategy::BackendManagedState => "backend-managed-state",
    }
}

fn identity_text(name: &str, identity: &BackendIdentity) -> String {
    format!(
        "{name}.schema={:?}\n{name}.value={:?}",
        identity.schema(),
        identity.value()
    )
}

fn sequence_text(sequence: Option<JournalSequence>) -> String {
    sequence.map_or_else(|| "none".to_owned(), |sequence| sequence.get().to_string())
}

pub(super) const fn exchange_kind_text(kind: StoredExchangeKind) -> &'static str {
    match kind {
        StoredExchangeKind::Request => "request",
        StoredExchangeKind::Response => "response",
        StoredExchangeKind::Notification => "notification",
        StoredExchangeKind::ServerRequest => "server-request",
        StoredExchangeKind::Retry => "retry",
        StoredExchangeKind::TerminalOutcome => "terminal-outcome",
    }
}

pub(super) const fn exchange_direction_text(direction: StoredExchangeDirection) -> &'static str {
    match direction {
        StoredExchangeDirection::YoToBackend => "yo-to-backend",
        StoredExchangeDirection::BackendToYo => "backend-to-yo",
    }
}

pub(super) const fn detail_availability_text(
    availability: StoredRequestDetailAvailability,
) -> &'static str {
    match availability {
        StoredRequestDetailAvailability::Persisted => "persisted",
        StoredRequestDetailAvailability::Volatile => "volatile",
        StoredRequestDetailAvailability::Missing => "missing",
        StoredRequestDetailAvailability::Unsupported => "unsupported",
        StoredRequestDetailAvailability::Unpersisted => "unpersisted",
        StoredRequestDetailAvailability::Redacted => "redacted",
    }
}

pub(super) const fn transition_mode_text(mode: StoredBindingTransitionMode) -> &'static str {
    match mode {
        StoredBindingTransitionMode::Initial => "initial",
        StoredBindingTransitionMode::ExactReplay => "exact-replay",
        StoredBindingTransitionMode::LossyHandoff => "lossy-handoff",
    }
}

pub(super) const fn cache_state_text(state: StoredBindingCacheState) -> &'static str {
    match state {
        StoredBindingCacheState::NotApplicable => "not-applicable",
        StoredBindingCacheState::Lost => "lost",
        StoredBindingCacheState::Unknown => "unknown",
    }
}

pub(super) const fn close_reason_text(reason: StoredBindingCloseReason) -> &'static str {
    match reason {
        StoredBindingCloseReason::Replaced => "replaced",
        StoredBindingCloseReason::Revoked => "revoked",
        StoredBindingCloseReason::Exhausted => "exhausted",
    }
}
