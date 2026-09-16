use super::{
    super::super::{
        BackendExchangeObserved, BindingCloseReason, ContextImageLoss, ContextImageSource,
        ExchangeDirection, ExchangeKind, JournalCodecError, JournalRecord, OperationId,
    },
    CorrelationRecovery,
    model::{IndexedExchange, ReferenceTarget, ReplacementSource, ReplayDeltaSource, ReplayGroup},
};
use crate::{
    AgentCommand, AgentEvent, ContinuationStrategy, JournalSequence, ModelReplayItem,
    ReplayProfile, TurnOutcome,
    backend::{provider_private_schema, validate_provider_private_replay_sequence},
    journal::CommittedCommand,
};

impl CorrelationRecovery {
    pub(super) fn observe(
        &mut self,
        sequence: JournalSequence,
        record: &JournalRecord,
        previous_in_commit: Option<(JournalSequence, &JournalRecord)>,
    ) -> Result<(), JournalCodecError> {
        let anchor_before_record = self.latest_anchor;
        let fork_before_record = self.initial_fork_seed();
        let preserves_anchor = matches!(
            record,
            JournalRecord::ContinuationAnchor(_) | JournalRecord::ContextPolicyChanged(_)
        ) || matches!(record, JournalRecord::CommandCommitted(command)
            if matches!(command.command(), AgentCommand::CompactContext { .. }));
        // A manual compaction command changes no replay until its checkpoint commits.
        // Ordinary submitted input and accepted requests still invalidate this source.
        if !preserves_anchor {
            self.latest_anchor = None;
        }

        match record {
            JournalRecord::InitialForkSeed(seed) => {
                if self.fork_seed.is_some() || self.last_epoch.is_some() || !self.session_created {
                    return Err(JournalCodecError::new(
                        "initial fork seed must precede the first child binding",
                    ));
                }
                self.fork_seed = Some(RegisteredFork {
                    sequence,
                    seed: seed.clone(),
                    owner_epoch: None,
                    has_accepted_request: false,
                });
            },
            JournalRecord::CommandCommitted(command) => self.observe_command(sequence, command),
            JournalRecord::EventCommitted(event) => self.observe_event(sequence, event),
            JournalRecord::BackendExchangeObserved(exchange) => {
                self.observe_exchange(sequence, exchange)?;
            },
            JournalRecord::BackendBindingOpened(binding) => {
                self.observe_binding_open(sequence, binding)?;
            },
            JournalRecord::BackendBindingClosed(binding) => {
                if self.open_epoch != Some(binding.epoch()) {
                    return Err(JournalCodecError::new(
                        "backend_binding_closed must close the current epoch",
                    ));
                }
                self.open_epoch = None;
                self.open_strategy = None;
                self.open_binding_identity = None;
                self.last_close_reason = Some(binding.reason());
                self.replacement_without_source_allowed = binding.reason()
                    == BindingCloseReason::Replaced
                    && !self.open_epoch_has_accepted_request;
                self.replacement_source = if binding.reason() == BindingCloseReason::Replaced {
                    anchor_before_record
                        .map(|sequence| ReplacementSource::Anchor {
                            sequence,
                            epoch: binding.epoch(),
                        })
                        .or_else(|| {
                            (!self.request_after_checkpoint)
                                .then_some(self.latest_checkpoint)
                                .flatten()
                                .map(|sequence| ReplacementSource::Checkpoint {
                                    sequence,
                                    epoch: binding.epoch(),
                                })
                        })
                        .or_else(|| {
                            fork_before_record.map(|sequence| ReplacementSource::InitialFork {
                                sequence,
                                epoch: binding.epoch(),
                            })
                        })
                } else {
                    None
                };
            },
            JournalRecord::BackendRequestAccepted(request) => {
                self.observe_context_epoch(request.context_epoch(), "backend_request_accepted")?;
                if self.open_epoch != Some(request.epoch()) {
                    return Err(JournalCodecError::new(
                        "backend_request_accepted requires its verified open epoch",
                    ));
                }
                let operation = request.operation_id();
                if let Some(command_turn) = self.submission_commands.get(&operation) {
                    if *command_turn != request.turn_id() {
                        return Err(JournalCodecError::new(
                            "backend_request_accepted turn does not match its submission command",
                        ));
                    }
                } else {
                    let is_internal_successor = self.session_id.is_some_and(|session_id| {
                        let has_prior_request = self
                            .latest_accepted_request
                            .contains_key(&(request.epoch(), request.turn_id()));
                        let is_first_post_checkpoint_request = self
                            .latest_checkpoint
                            .zip(self.active_turn_starts.get(&request.turn_id()).copied())
                            .is_some_and(|(checkpoint, start)| {
                                start < checkpoint && !self.request_after_checkpoint
                            });
                        self.active_turn_starts.contains_key(&request.turn_id())
                            && (has_prior_request || is_first_post_checkpoint_request)
                            && operation
                                == OperationId::for_internal_request(
                                    session_id,
                                    request.turn_id(),
                                    request.exchange_sequence(),
                                )
                    });
                    if !is_internal_successor {
                        return Err(JournalCodecError::new(
                            "backend_request_accepted has neither a matching submission command nor a valid writer-assigned successor identity",
                        ));
                    }
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
                self.open_epoch_has_accepted_request = true;
                if let Some(fork) = &mut self.fork_seed {
                    fork.has_accepted_request = true;
                }
                if self.latest_checkpoint.is_some() {
                    self.request_after_checkpoint = true;
                }
            },
            JournalRecord::ModelReplayDelta(replay) => {
                self.observe_context_epoch(replay.context_epoch(), "model_replay_delta")?;
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
                let apply_result = if self.replay_contract_rebind_required {
                    if self
                        .replay_groups
                        .iter()
                        .any(|group| group.fork_group_index.is_some())
                        && replay.delta().contract() != self.model_replay.contract()
                    {
                        return Err(JournalCodecError::new(
                            "fork replacement delta must declare the preserved replay contract",
                        ));
                    }
                    self.model_replay.apply_binding_replacement(replay.delta())
                } else {
                    self.model_replay.apply(replay.delta())
                };
                apply_result.map_err(|detail| {
                    JournalCodecError::new(format!(
                        "model_replay_delta cannot extend the replay chain: {detail}"
                    ))
                })?;
                self.replay_contract_rebind_required = false;
                if let Some(context_epoch) = replay.context_epoch() {
                    let origins = (0..replay.delta().items().len())
                        .map(|index| {
                            self.local_item_origin(replay.epoch(), context_epoch, sequence, index)
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    self.model_replay_origins.extend(origins);
                    self.replay_deltas.insert(
                        sequence,
                        ReplayDeltaSource {
                            epoch: replay.epoch(),
                            context_epoch,
                            turn_id: replay.turn_id(),
                            accepted_request_sequence: replay.accepted_request_sequence(),
                            items: replay.delta().items().to_vec(),
                        },
                    );
                }
            },
            JournalRecord::BackendResumableOutcome(outcome) => {
                self.observe_context_epoch(outcome.context_epoch(), "backend_resumable_outcome")?;
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
                self.observe_context_epoch(anchor.context_epoch(), "continuation_anchor")?;
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
                if let (Some(context_epoch), Some(replay_delta_sequence)) =
                    (anchor.context_epoch(), outcome.replay_delta_sequence())
                {
                    let delta =
                        self.replay_deltas
                            .get(&replay_delta_sequence)
                            .ok_or_else(|| {
                                JournalCodecError::new(
                                    "continuation_anchor replay group has no indexed replay delta",
                                )
                            })?;
                    let first_sequence = self
                        .completed_turns
                        .get(&delta.turn_id)
                        .copied()
                        .ok_or_else(|| {
                            JournalCodecError::new(
                                "continuation_anchor replay group has no completed Turn boundary",
                            )
                        })?;
                    if delta.epoch != anchor.epoch()
                        || delta.context_epoch != context_epoch
                        || delta.accepted_request_sequence != anchor.accepted_request_sequence()
                        || first_sequence > replay_delta_sequence
                        || replay_delta_sequence > sequence
                    {
                        return Err(JournalCodecError::new(
                            "continuation_anchor replay group coordinates are inconsistent",
                        ));
                    }
                    self.replay_groups.push(ReplayGroup {
                        first_sequence,
                        last_sequence: anchor.journal_boundary(),
                        replay_delta_sequence,
                        epoch: delta.epoch,
                        context_epoch,
                        image_losses: ContextImageLoss::for_items(
                            &delta.items,
                            context_epoch,
                            |item_index, part_index| ContextImageSource::ReplayDelta {
                                sequence: replay_delta_sequence.get(),
                                item_index,
                                part_index,
                            },
                        )
                        .map_err(JournalCodecError::new)?,
                        items: delta.items.clone(),
                        fork_group_index: None,
                    });
                }
                self.latest_anchor = Some(sequence);
                self.request_after_checkpoint = false;
            },
            JournalRecord::ContextPolicyChanged(policy) => {
                if self.saw_legacy_context_record {
                    return Err(JournalCodecError::new(
                        "context policy cannot be introduced into a legacy context graph",
                    ));
                }
                let expected_revision = self
                    .current_policy
                    .as_ref()
                    .map_or(1, |current| current.policy_revision().saturating_add(1));
                if policy.policy_revision() != expected_revision {
                    return Err(JournalCodecError::new(
                        "context policy revisions must start at 1 and increase by exactly one",
                    ));
                }
                if self.open_epoch.is_none()
                    || !matches!(
                        self.open_strategy,
                        Some(ContinuationStrategy::ExactReplay {
                            executor: crate::ReplayExecutor::LocalClient,
                            ..
                        })
                    )
                {
                    return Err(JournalCodecError::new(
                        "context policy requires an open local-client exact-replay binding",
                    ));
                }
                self.context_epoch.get_or_insert(1);
                self.current_policy = Some(policy.clone());
            },
            JournalRecord::ContextCheckpoint(checkpoint) => {
                self.observe_checkpoint(sequence, checkpoint)?;
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
                        context_epoch: anchor.context_epoch(),
                        journal_boundary: anchor.journal_boundary(),
                    },
                );
            },
            JournalRecord::ContextCheckpoint(checkpoint) => {
                let replay_profile = match self.open_strategy {
                    Some(ContinuationStrategy::ExactReplay { replay_profile, .. }) => {
                        replay_profile
                    },
                    _ => unreachable!("a validated checkpoint has an exact-replay binding"),
                };
                let binding_identity = self
                    .open_binding_identity
                    .clone()
                    .expect("a validated checkpoint has an open binding identity");
                let has_provider_private = checkpoint
                    .retained_groups()
                    .iter()
                    .flat_map(|group| group.items())
                    .any(|item| matches!(item, ModelReplayItem::ProviderPrivateAssistant { .. }));
                self.reference_targets.insert(
                    sequence,
                    ReferenceTarget::Checkpoint {
                        epoch: checkpoint.epoch(),
                        context_epoch: checkpoint.successor_context_epoch(),
                        binding_identity,
                        replay_profile,
                        has_provider_private,
                    },
                );
            },
            _ => {},
        }
        if let (Some(epoch), Some(context_epoch)) = (self.open_epoch, self.context_epoch) {
            self.record_coordinates
                .insert(sequence, (epoch, context_epoch));
        }
        Ok(())
    }

    fn observe_command(&mut self, sequence: JournalSequence, command: &CommittedCommand) {
        let Some(submission_id) = command.submission_id() else {
            return;
        };
        let turn_id = match command.command() {
            AgentCommand::StartTurn { turn, input } => {
                self.active_turn_starts.insert(turn.turn_id(), sequence);
                self.submitted_inputs
                    .insert(sequence, input.model_replay_item());
                turn.turn_id()
            },
            AgentCommand::SteerTurn { turn, input } => {
                self.submitted_inputs
                    .insert(sequence, input.model_replay_item());
                turn.turn_id()
            },
            AgentCommand::CreateSession { .. }
            | AgentCommand::InterruptTurn { .. }
            | AgentCommand::CompactContext { .. }
            | AgentCommand::RespondToActivity { .. } => return,
        };
        self.submission_commands
            .insert(OperationId::from(submission_id), turn_id);
    }

    fn observe_event(&mut self, sequence: JournalSequence, event: &AgentEvent) {
        match event {
            AgentEvent::SessionCreated { session_id } => {
                self.session_id = Some(*session_id);
                self.session_created = true;
            },
            AgentEvent::TurnFinished { turn, outcome } => {
                self.active_turn_starts.remove(&turn.turn_id());
                if matches!(outcome, TurnOutcome::Completed) {
                    self.completed_turns.insert(turn.turn_id(), sequence);
                } else {
                    self.completed_turns.remove(&turn.turn_id());
                }
            },
            AgentEvent::TurnStarted { turn } => {
                self.completed_turns.remove(&turn.turn_id());
            },
            AgentEvent::ActivityFinished { .. } => {
                self.completed_activity_boundaries.insert(sequence);
            },
            AgentEvent::ActivityStarted { .. } | AgentEvent::ActivityUpdated { .. } => {},
        }
    }

    fn observe_exchange(
        &mut self,
        sequence: JournalSequence,
        exchange: &BackendExchangeObserved,
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
        exchange: &BackendExchangeObserved,
    ) -> Result<IndexedExchange, JournalCodecError> {
        let sequence = exchange.correlation_sequence().ok_or_else(|| {
            JournalCodecError::new("correlated exchange is missing correlation_sequence")
        })?;
        let Some(ReferenceTarget::Exchange(target)) =
            self.reference_targets.get(&sequence).cloned()
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
}
