use std::collections::{BTreeMap, BTreeSet};

use super::super::{
    BindingCloseReason, CacheState, ExchangeDirection, ExchangeKind, JournalCodecError,
    JournalRecord, OperationId, TransitionMode,
};
use crate::{
    AgentCommand, AgentEvent, ContinuationStrategy, JournalSequence, ModelReplay, ModelReplayItem,
    ReplayProfile, TurnId, TurnOutcome,
    backend::{provider_private_schema, validate_provider_private_replay_sequence},
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct CorrelationRecovery {
    reference_targets: BTreeMap<JournalSequence, ReferenceTarget>,
    operation_roots: BTreeSet<OperationId>,
    submission_commands: BTreeMap<OperationId, TurnId>,
    latest_request_exchange: BTreeMap<(u64, OperationId), JournalSequence>,
    latest_accepted_request: BTreeMap<(u64, TurnId), JournalSequence>,
    completed_turns: BTreeMap<TurnId, JournalSequence>,
    session_created: bool,
    open_epoch: Option<u64>,
    open_strategy: Option<ContinuationStrategy>,
    last_epoch: Option<u64>,
    last_close_reason: Option<BindingCloseReason>,
    replacement_source: Option<(JournalSequence, u64)>,
    latest_anchor: Option<JournalSequence>,
    model_replay: ModelReplay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReferenceTarget {
    Exchange(IndexedExchange),
    Anchor { epoch: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IndexedExchange {
    epoch: u64,
    operation_id: OperationId,
    kind: ExchangeKind,
    direction: ExchangeDirection,
}

impl From<&super::super::BackendExchangeObserved> for IndexedExchange {
    fn from(exchange: &super::super::BackendExchangeObserved) -> Self {
        Self {
            epoch: exchange.epoch(),
            operation_id: exchange.operation_id(),
            kind: exchange.kind(),
            direction: exchange.direction(),
        }
    }
}

impl CorrelationRecovery {
    pub(super) fn observe(
        &mut self,
        sequence: JournalSequence,
        record: &JournalRecord,
        previous_in_commit: Option<(JournalSequence, &JournalRecord)>,
    ) -> Result<(), JournalCodecError> {
        let anchor_before_record = self.latest_anchor;
        if !matches!(record, JournalRecord::ContinuationAnchor(_)) {
            self.latest_anchor = None;
        }

        match record {
            JournalRecord::CommandCommitted(command) => self.observe_command(command),
            JournalRecord::EventCommitted(event) => self.observe_event(sequence, event),
            JournalRecord::BackendExchangeObserved(exchange) => {
                self.observe_exchange(sequence, exchange)?;
            },
            JournalRecord::BackendBindingOpened(binding) => {
                self.observe_binding_open(binding)?;
            },
            JournalRecord::BackendBindingClosed(binding) => {
                if self.open_epoch != Some(binding.epoch()) {
                    return Err(JournalCodecError::new(
                        "backend_binding_closed must close the current epoch",
                    ));
                }
                self.open_epoch = None;
                self.open_strategy = None;
                self.last_close_reason = Some(binding.reason());
                self.replacement_source = if binding.reason() == BindingCloseReason::Replaced {
                    anchor_before_record.map(|anchor| (anchor, binding.epoch()))
                } else {
                    None
                };
            },
            JournalRecord::BackendRequestAccepted(request) => {
                if self.open_epoch != Some(request.epoch()) {
                    return Err(JournalCodecError::new(
                        "backend_request_accepted requires its verified open epoch",
                    ));
                }
                let operation = request.operation_id();
                let Some(command_turn) = self.submission_commands.get(&operation) else {
                    return Err(JournalCodecError::new(
                        "backend_request_accepted has no matching submission command",
                    ));
                };
                if *command_turn != request.turn_id() {
                    return Err(JournalCodecError::new(
                        "backend_request_accepted turn does not match its submission command",
                    ));
                }
                if self
                    .latest_request_exchange
                    .get(&(request.epoch(), operation))
                    != Some(&request.exchange_sequence())
                {
                    return Err(JournalCodecError::new(
                        "backend_request_accepted must reference the latest outbound request exchange",
                    ));
                }
                self.latest_accepted_request
                    .insert((request.epoch(), request.turn_id()), sequence);
            },
            JournalRecord::ModelReplayDelta(replay) => {
                if self.open_epoch != Some(replay.epoch())
                    || !matches!(
                        self.open_strategy,
                        Some(ContinuationStrategy::ExactReplay { .. })
                    )
                {
                    return Err(JournalCodecError::new(
                        "model_replay_delta requires an exact-replay open epoch",
                    ));
                }
                let has_provider_private =
                    replay.delta().items().iter().any(|item| {
                        matches!(item, ModelReplayItem::ProviderPrivateAssistant { .. })
                    });
                match self.open_strategy {
                    Some(ContinuationStrategy::ExactReplay {
                        replay_profile: ReplayProfile::SemanticOnly,
                        ..
                    }) if has_provider_private => {
                        return Err(JournalCodecError::new(
                            "semantic-only exact replay cannot contain provider-private items",
                        ));
                    },
                    Some(ContinuationStrategy::ExactReplay {
                        replay_profile: ReplayProfile::ProviderPrivateLocalPlaintext,
                        ..
                    }) => {
                        validate_provider_private_replay_sequence(
                            replay.delta().items(),
                            provider_private_schema(ReplayProfile::ProviderPrivateLocalPlaintext)
                                .expect("the provider-private profile has an exact schema"),
                        )
                        .map_err(JournalCodecError::new)?;
                    },
                    _ => {},
                }
                if self
                    .latest_accepted_request
                    .get(&(replay.epoch(), replay.turn_id()))
                    != Some(&replay.accepted_request_sequence())
                {
                    return Err(JournalCodecError::new(
                        "model_replay_delta must reference the latest accepted request",
                    ));
                }
                let Some((
                    _,
                    JournalRecord::EventCommitted(AgentEvent::TurnFinished {
                        turn,
                        outcome: TurnOutcome::Completed,
                    }),
                )) = previous_in_commit
                else {
                    return Err(JournalCodecError::new(
                        "model_replay_delta must immediately follow its completed Turn",
                    ));
                };
                if turn.turn_id() != replay.turn_id() {
                    return Err(JournalCodecError::new(
                        "model_replay_delta Turn does not match its completed Turn",
                    ));
                }
                self.model_replay.apply(replay.delta()).map_err(|detail| {
                    JournalCodecError::new(format!(
                        "model_replay_delta cannot extend the replay chain: {detail}"
                    ))
                })?;
            },
            JournalRecord::BackendResumableOutcome(outcome) => {
                if self.open_epoch != Some(outcome.epoch()) {
                    return Err(JournalCodecError::new(
                        "backend_resumable_outcome requires its open epoch",
                    ));
                }
                if self
                    .latest_accepted_request
                    .get(&(outcome.epoch(), outcome.turn_id()))
                    != Some(&outcome.accepted_request_sequence())
                {
                    return Err(JournalCodecError::new(
                        "backend_resumable_outcome must reference the latest accepted request",
                    ));
                }
                if !self
                    .completed_turns
                    .get(&outcome.turn_id())
                    .is_some_and(|completed| *completed > outcome.accepted_request_sequence())
                {
                    return Err(JournalCodecError::new(
                        "backend_resumable_outcome requires a preceding completed Turn",
                    ));
                }
                match self.open_strategy {
                    Some(ContinuationStrategy::ExactReplay { .. }) => {
                        let Some(replay_sequence) = outcome.replay_delta_sequence() else {
                            return Err(JournalCodecError::new(
                                "exact-replay outcome requires replay_delta_sequence",
                            ));
                        };
                        let Some((previous_sequence, JournalRecord::ModelReplayDelta(replay))) =
                            previous_in_commit
                        else {
                            return Err(JournalCodecError::new(
                                "exact-replay outcome must immediately follow its replay delta",
                            ));
                        };
                        if replay_sequence != previous_sequence
                            || replay.epoch() != outcome.epoch()
                            || replay.turn_id() != outcome.turn_id()
                            || replay.accepted_request_sequence()
                                != outcome.accepted_request_sequence()
                        {
                            return Err(JournalCodecError::new(
                                "exact-replay outcome does not match its replay delta",
                            ));
                        }
                    },
                    Some(ContinuationStrategy::BackendManagedState) => {
                        if outcome.replay_delta_sequence().is_some()
                            || !matches!(
                                previous_in_commit,
                                Some((_, JournalRecord::EventCommitted(AgentEvent::TurnFinished {
                                    turn,
                                    outcome: TurnOutcome::Completed,
                                }))) if turn.turn_id() == outcome.turn_id()
                            )
                        {
                            return Err(JournalCodecError::new(
                                "backend-managed outcome must immediately follow its completed Turn without replay evidence",
                            ));
                        }
                    },
                    None => {
                        return Err(JournalCodecError::new(
                            "backend_resumable_outcome requires a continuation strategy",
                        ));
                    },
                }
            },
            JournalRecord::ContinuationAnchor(anchor) => {
                let Some((previous_sequence, JournalRecord::BackendResumableOutcome(outcome))) =
                    previous_in_commit
                else {
                    return Err(JournalCodecError::new(
                        "continuation_anchor must immediately follow its resumable outcome in one commit",
                    ));
                };
                if previous_sequence != anchor.resumable_outcome_sequence()
                    || anchor.journal_boundary() != previous_sequence
                    || anchor.epoch() != outcome.epoch()
                    || anchor.accepted_request_sequence() != outcome.accepted_request_sequence()
                {
                    return Err(JournalCodecError::new(
                        "continuation_anchor does not match its resumable outcome",
                    ));
                }
                self.latest_anchor = Some(sequence);
            },
            JournalRecord::SessionDescriptor(_)
            | JournalRecord::MessageReset(_)
            | JournalRecord::MessageSegment(_)
            | JournalRecord::MessageEnded(_) => {},
        }

        match record {
            JournalRecord::BackendExchangeObserved(exchange) => {
                self.reference_targets
                    .insert(sequence, ReferenceTarget::Exchange(exchange.into()));
            },
            JournalRecord::ContinuationAnchor(anchor) => {
                self.reference_targets.insert(
                    sequence,
                    ReferenceTarget::Anchor {
                        epoch: anchor.epoch(),
                    },
                );
            },
            _ => {},
        }
        Ok(())
    }

    pub(super) const fn open_epoch(&self) -> Option<u64> {
        self.open_epoch
    }

    pub(super) const fn latest_anchor(&self) -> Option<JournalSequence> {
        self.latest_anchor
    }

    pub(super) const fn model_replay(&self) -> &ModelReplay {
        &self.model_replay
    }

    fn observe_command(&mut self, command: &crate::journal::CommittedCommand) {
        let Some(submission_id) = command.submission_id() else {
            return;
        };
        let turn_id = match command.command() {
            AgentCommand::StartTurn { turn, .. } | AgentCommand::SteerTurn { turn, .. } => {
                turn.turn_id()
            },
            AgentCommand::CreateSession { .. }
            | AgentCommand::InterruptTurn { .. }
            | AgentCommand::RespondToActivity { .. } => return,
        };
        self.submission_commands
            .insert(OperationId::from(submission_id), turn_id);
    }

    fn observe_event(&mut self, sequence: JournalSequence, event: &AgentEvent) {
        match event {
            AgentEvent::SessionCreated { .. } => self.session_created = true,
            AgentEvent::TurnFinished { turn, outcome } => {
                if matches!(outcome, TurnOutcome::Completed) {
                    self.completed_turns.insert(turn.turn_id(), sequence);
                } else {
                    self.completed_turns.remove(&turn.turn_id());
                }
            },
            AgentEvent::TurnStarted { turn } => {
                self.completed_turns.remove(&turn.turn_id());
            },
            AgentEvent::ActivityStarted { .. }
            | AgentEvent::ActivityUpdated { .. }
            | AgentEvent::ActivityFinished { .. } => {},
        }
    }

    fn observe_exchange(
        &mut self,
        sequence: JournalSequence,
        exchange: &super::super::BackendExchangeObserved,
    ) -> Result<(), JournalCodecError> {
        if self.open_epoch != Some(exchange.epoch()) {
            return Err(JournalCodecError::new(
                "backend_exchange_observed requires its open binding epoch",
            ));
        }
        let correlation = exchange.correlation_sequence();
        match exchange.kind() {
            ExchangeKind::Request | ExchangeKind::ServerRequest | ExchangeKind::Notification => {
                if correlation.is_some() {
                    return Err(JournalCodecError::new(
                        "root request, server request, and notification exchanges cannot have a correlation edge",
                    ));
                }
                if !self.operation_roots.insert(exchange.operation_id()) {
                    return Err(JournalCodecError::new(
                        "one operation_id cannot begin a second root exchange",
                    ));
                }
            },
            ExchangeKind::Response => {
                let target = self.exchange_target(exchange)?;
                if !matches!(
                    target.kind,
                    ExchangeKind::Request | ExchangeKind::ServerRequest
                ) || target.direction == exchange.direction()
                {
                    return Err(JournalCodecError::new(
                        "response must reference an opposite-direction request or server request",
                    ));
                }
            },
            ExchangeKind::Retry => {
                let target = self.exchange_target(exchange)?;
                if !matches!(
                    target.kind,
                    ExchangeKind::Request | ExchangeKind::ServerRequest | ExchangeKind::Retry
                ) || target.direction != exchange.direction()
                {
                    return Err(JournalCodecError::new(
                        "retry must reference a same-direction request, server request, or retry",
                    ));
                }
            },
            ExchangeKind::TerminalOutcome => {
                let target = self.exchange_target(exchange)?;
                if !matches!(
                    target.kind,
                    ExchangeKind::Request
                        | ExchangeKind::ServerRequest
                        | ExchangeKind::Retry
                        | ExchangeKind::Response
                ) {
                    return Err(JournalCodecError::new(
                        "terminal outcome has an invalid correlation target",
                    ));
                }
            },
        }
        if exchange.kind() == ExchangeKind::Request
            && exchange.direction() == ExchangeDirection::YoToBackend
        {
            self.latest_request_exchange
                .insert((exchange.epoch(), exchange.operation_id()), sequence);
        }
        Ok(())
    }

    fn exchange_target(
        &self,
        exchange: &super::super::BackendExchangeObserved,
    ) -> Result<IndexedExchange, JournalCodecError> {
        let sequence = exchange.correlation_sequence().ok_or_else(|| {
            JournalCodecError::new("correlated exchange is missing correlation_sequence")
        })?;
        let Some(ReferenceTarget::Exchange(target)) =
            self.reference_targets.get(&sequence).copied()
        else {
            return Err(JournalCodecError::new(
                "exchange correlation_sequence does not identify an earlier exchange",
            ));
        };
        if target.epoch != exchange.epoch() || target.operation_id != exchange.operation_id() {
            return Err(JournalCodecError::new(
                "correlated exchanges must share epoch and operation_id",
            ));
        }
        Ok(target)
    }

    fn observe_binding_open(
        &mut self,
        binding: &super::super::BackendBindingOpened,
    ) -> Result<(), JournalCodecError> {
        if self.open_epoch.is_some() {
            return Err(JournalCodecError::new(
                "at most one backend binding epoch may be open",
            ));
        }
        match self.last_epoch {
            None => {
                if binding.epoch() != 1
                    || !self.session_created
                    || binding.transition().mode() != TransitionMode::Initial
                    || binding.transition().cache() != CacheState::NotApplicable
                    || binding.transition().source_anchor_sequence().is_some()
                {
                    return Err(JournalCodecError::new(
                        "first backend binding must open epoch 1 after SessionCreated with an initial transition",
                    ));
                }
            },
            Some(previous) => {
                if binding.epoch() != previous.saturating_add(1)
                    || self.last_close_reason != Some(BindingCloseReason::Replaced)
                    || binding.transition().mode() == TransitionMode::Initial
                {
                    return Err(JournalCodecError::new(
                        "replacement binding must follow a replaced epoch with the next number",
                    ));
                }
                let source = binding
                    .transition()
                    .source_anchor_sequence()
                    .ok_or_else(|| {
                        JournalCodecError::new(
                            "replacement binding requires a source continuation Anchor",
                        )
                    })?;
                let Some(ReferenceTarget::Anchor { epoch }) =
                    self.reference_targets.get(&source).copied()
                else {
                    return Err(JournalCodecError::new(
                        "replacement binding source does not identify a continuation Anchor",
                    ));
                };
                let inherited_local_replay_anchor = binding.continuation_strategy()
                    == (ContinuationStrategy::ExactReplay {
                        executor: crate::ReplayExecutor::LocalClient,
                        replay_profile: crate::ReplayProfile::SemanticOnly,
                    })
                    && binding.transition().mode() == TransitionMode::ExactReplay;
                if self.replacement_source != Some((source, previous))
                    || (epoch != previous && !inherited_local_replay_anchor)
                {
                    return Err(JournalCodecError::new(
                        "replacement binding must use the valid resume Anchor carried by the immediately preceding epoch",
                    ));
                }
                if inherited_local_replay_anchor {
                    self.latest_anchor = Some(source);
                }
            },
        }
        self.open_epoch = Some(binding.epoch());
        self.open_strategy = Some(binding.continuation_strategy());
        if binding.continuation_strategy() == ContinuationStrategy::BackendManagedState
            || binding.transition().mode() != TransitionMode::ExactReplay
        {
            self.model_replay = ModelReplay::default();
        }
        self.last_epoch = Some(binding.epoch());
        self.last_close_reason = None;
        self.replacement_source = None;
        Ok(())
    }
}
