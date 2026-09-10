use std::{collections::BTreeSet, fmt::Write as _, mem};

use sha2::Digest as _;

use super::{
    CommittedCommand, JournalSequence, SemanticRecord, SessionJournal,
    codec::{
        BackendBindingOpened, BackendExchangeObserved, BackendRequestAccepted,
        BackendResumableOutcome, BindingCloseReason, BindingTransition, CacheState,
        ContextArtifactReceipt, ContextCheckpoint, ContextImageLoss, ContextImageSource,
        ContextLoss, ContextPolicyChanged, ContextRetainedGroup, ContextStrategy,
        ContextSummaryUsage, ContinuationAnchor, DetailAvailability, ExchangeDirection,
        ExchangeKind, ForkSeed, ModelReplayDeltaRecord, OperationId, TransitionMode,
        VersionedIdentity, validate_image_losses,
    },
    read_state,
};
#[cfg(test)]
use crate::ModelReplayRole;
use crate::{
    AgentCommand, AgentEvent, BackendBindingEvidence, BackendOutcomeEvidence,
    BackendRequestEvidence, BackendResumeSource, ContextCheckpointProposal, ContinuationStrategy,
    ModelReplay, ModelReplayItem, SubmissionId, TurnOutcome, TurnRef,
};

#[derive(Clone)]
struct ContextSourceGroup {
    first_sequence: JournalSequence,
    last_sequence: JournalSequence,
    replay_sequence: JournalSequence,
    items: Vec<ModelReplayItem>,
    fork_import: Option<(JournalSequence, usize)>,
    private_epochs: Vec<u64>,
    image_losses: Vec<ContextImageLoss>,
}

#[derive(Clone)]
pub(crate) struct ContextActiveSource {
    turn: TurnRef,
    first_sequence: JournalSequence,
    last_sequence: JournalSequence,
    items: Vec<ModelReplayItem>,
}

impl ContextActiveSource {
    pub(crate) fn new(
        turn: TurnRef,
        first_sequence: JournalSequence,
        last_sequence: JournalSequence,
        items: Vec<ModelReplayItem>,
    ) -> Self {
        Self {
            turn,
            first_sequence,
            last_sequence,
            items,
        }
    }

    pub(crate) fn try_advance(
        &mut self,
        turn: TurnRef,
        last_sequence: JournalSequence,
        items: Vec<ModelReplayItem>,
    ) -> bool {
        if self.turn != turn
            || last_sequence <= self.last_sequence
            || items.len() <= self.items.len()
            || !items.starts_with(&self.items)
        {
            return false;
        }
        self.last_sequence = last_sequence;
        self.items = items;
        true
    }
}

impl SessionJournal {
    pub(crate) fn append_context_policy(&mut self, policy: ContextPolicyChanged) -> bool {
        let records = vec![SemanticRecord::ContextPolicyChanged(policy)];
        if self.durable.is_none() {
            self.append_records(records);
            true
        } else {
            self.append_records_transactionally(records)
        }
    }

    pub(crate) fn commit_context_checkpoint(
        &mut self,
        proposal: &ContextCheckpointProposal,
        policy: &ContextPolicyChanged,
        epoch: u64,
        previous_context_epoch: u64,
        source_anchor_sequence: JournalSequence,
        active_source: Option<&ContextActiveSource>,
    ) -> Option<(JournalSequence, ModelReplay)> {
        if !policy.enabled()
            || policy.strategy() != ContextStrategy::PortableSummaryV1Alpha1
            || policy.policy_revision() != proposal.policy_revision()
            || u128::from(proposal.input_tokens_after()) * 100
                >= u128::from(proposal.input_token_limit()) * u128::from(policy.trigger_percent())
        {
            return None;
        }
        let entries = self.semantic_entries();
        let groups = context_source_groups(&entries, epoch, previous_context_epoch)?;
        let expected_groups = proposal
            .summarized_groups()
            .iter()
            .chain(proposal.retained_groups())
            .collect::<Vec<_>>();
        if expected_groups.len() != groups.len()
            || expected_groups
                .iter()
                .zip(&groups)
                .any(|(expected, source)| expected.as_slice() != source.items)
        {
            return None;
        }
        let summarized_count = proposal.summarized_groups().len();
        if summarized_count == 0 || summarized_count > groups.len() {
            return None;
        }
        let anchor = entries.iter().find_map(|entry| {
            (entry.sequence() == source_anchor_sequence).then(|| match entry.record() {
                SemanticRecord::ContinuationAnchor(anchor) => Some(anchor),
                _ => None,
            })?
        })?;
        if anchor.epoch() != epoch || anchor.context_epoch() != Some(previous_context_epoch) {
            return None;
        }
        let mut retained = groups[summarized_count..]
            .iter()
            .map(|group| {
                match group.fork_import {
                    Some((seed, index)) => ContextRetainedGroup::try_imported(
                        seed,
                        index,
                        group.items.clone(),
                        group.private_epochs.clone(),
                    ),
                    None => ContextRetainedGroup::try_new(
                        group.first_sequence,
                        group.last_sequence,
                        group.items.clone(),
                    ),
                }
                .ok()
            })
            .collect::<Option<Vec<_>>>()?;
        let source_journal_boundary = if let Some(turn) = proposal.turn() {
            let source = active_source?;
            let (sequence, input) = entries.iter().find_map(|entry| {
                (entry.sequence() == source.first_sequence).then(|| match entry.record() {
                    SemanticRecord::CommandCommitted(committed) => match committed.command() {
                        AgentCommand::StartTurn {
                            turn: candidate,
                            input,
                        } if *candidate == turn => Some((entry.sequence(), input)),
                        _ => None,
                    },
                    _ => None,
                })?
            })?;
            let expected_input = input.model_replay_item();
            if source.turn != turn
                || source.first_sequence != sequence
                || source.last_sequence < source.first_sequence
                || source.items.first() != Some(&expected_input)
                || source.items.as_slice() != proposal.active_group()
                || entries
                    .iter()
                    .find(|entry| entry.sequence() == source.last_sequence)
                    .is_none_or(|entry| {
                        source.last_sequence != source.first_sequence
                            && !matches!(
                                entry.record(),
                                SemanticRecord::EventCommitted(AgentEvent::ActivityFinished {
                                    activity,
                                    ..
                                }) if activity.turn() == turn
                            )
                    })
            {
                return None;
            }
            retained.push(
                ContextRetainedGroup::try_new(
                    source.first_sequence,
                    source.last_sequence,
                    proposal.active_group().to_vec(),
                )
                .ok()?,
            );
            source.last_sequence
        } else {
            if active_source.is_some() || !proposal.active_group().is_empty() {
                return None;
            }
            anchor.journal_boundary()
        };
        let summarized = &groups[..summarized_count];
        let mut receipts = Vec::new();
        let mut receipt_identities = BTreeSet::new();
        let mut losses = vec![
            ContextLoss::visible_prefix_summarized(
                summarized.iter().map(|group| group.first_sequence).min()?,
                summarized.iter().map(|group| group.last_sequence).max()?,
            )
            .ok()?,
        ];
        for group in summarized {
            for item in &group.items {
                match item {
                    ModelReplayItem::FunctionCallOutput { output, .. } if !output.is_empty() => {
                        let digest = sha2::Sha256::digest(output.as_bytes());
                        let mut content_hash = String::from("sha256:");
                        for byte in digest {
                            write!(&mut content_hash, "{byte:02x}")
                                .expect("writing a digest into a String cannot fail");
                        }
                        let byte_count = u64::try_from(output.len()).ok()?;
                        if receipt_identities.insert((
                            group.replay_sequence,
                            content_hash.clone(),
                            byte_count,
                        )) {
                            receipts.push(
                                ContextArtifactReceipt::try_new(
                                    content_hash,
                                    byte_count,
                                    "text/plain",
                                    previous_context_epoch,
                                    group.replay_sequence,
                                )
                                .ok()?,
                            );
                        }
                    },
                    ModelReplayItem::ProviderPrivateAssistant { envelope } => {
                        losses.push(
                            ContextLoss::provider_private_dropped(
                                envelope.schema(),
                                u64::try_from(envelope.payload().len()).ok()?,
                                group.replay_sequence,
                            )
                            .ok()?,
                        );
                    },
                    _ => {},
                }
            }
        }
        let mut image_losses = Vec::new();
        for loss in summarized.iter().flat_map(|group| &group.image_losses) {
            if image_losses.len() == 64 {
                return None;
            }
            image_losses.push(loss.clone());
            validate_image_losses(&image_losses).ok()?;
        }
        losses.extend(
            image_losses
                .into_iter()
                .map(ContextLoss::ImageInputSummarized),
        );
        let first_retained_sequence = retained.first().map(ContextRetainedGroup::first_sequence);
        let mut checkpoint = ContextCheckpoint::try_new(
            epoch,
            previous_context_epoch,
            previous_context_epoch.checked_add(1)?,
            source_anchor_sequence,
            source_journal_boundary,
            policy.policy_revision(),
            policy.strategy(),
            proposal.input_token_limit(),
            proposal.input_tokens_before(),
            proposal.input_tokens_after(),
            proposal.replay_contract().clone(),
            proposal.portable_body(),
            retained,
            first_retained_sequence,
            receipts,
            losses,
            ContextSummaryUsage::try_new(proposal.summary_usage().clone()).ok()?,
        )
        .ok()?;
        if let Some((before, after)) = proposal.accounting() {
            checkpoint = checkpoint
                .with_accounting(before.clone(), after.clone())
                .ok()?;
        }
        checkpoint.validate_profile().ok()?;
        let binding = entries.iter().find_map(|entry| match entry.record() {
            SemanticRecord::BackendBindingOpened(binding) if binding.epoch() == epoch => {
                Some(binding)
            },
            _ => None,
        })?;
        checkpoint
            .validate_binding_accounting(binding.binding_identity().value())
            .ok()?;
        let replay = checkpoint.replay_root().ok()?;
        let sequence = read_state(&self.state).next_sequence();
        let records = vec![SemanticRecord::ContextCheckpoint(checkpoint)];
        let committed = if self.durable.is_none() {
            self.append_records(records);
            true
        } else {
            self.append_records_transactionally(records)
        };
        committed.then_some((sequence, replay))
    }

    pub(crate) fn append_initial_binding(
        &mut self,
        command: AgentCommand,
        events: &[AgentEvent],
        epoch: u64,
        evidence: BackendBindingEvidence,
    ) {
        let committed = CommittedCommand::uncorrelated(command)
            .expect("only an uncorrelated CreateSession may open the initial binding");
        let mut records = Vec::with_capacity(events.len() + 2);
        records.push(SemanticRecord::CommandCommitted(committed));
        records.extend(events.iter().cloned().map(SemanticRecord::EventCommitted));
        records.push(SemanticRecord::BackendBindingOpened(
            BackendBindingOpened::new(
                epoch,
                evidence.backend_kind(),
                evidence.backend_version(),
                versioned(evidence.binding_identity()),
                versioned(evidence.model_identity()),
                versioned(evidence.session_locator()),
                BindingTransition::new(TransitionMode::Initial, CacheState::NotApplicable, None),
                evidence.continuation_strategy(),
            ),
        ));
        self.append_records(records);
    }

    pub(crate) fn commit_exact_replay_replacement(
        &mut self,
        previous_epoch: u64,
        epoch: u64,
        source: BackendResumeSource,
        evidence: BackendBindingEvidence,
    ) -> bool {
        use super::codec::{BackendBindingClosed, BindingCloseReason};

        self.append_records_transactionally(vec![
            SemanticRecord::BackendBindingClosed(BackendBindingClosed::new(
                previous_epoch,
                BindingCloseReason::Replaced,
            )),
            SemanticRecord::BackendBindingOpened(BackendBindingOpened::new(
                epoch,
                evidence.backend_kind(),
                evidence.backend_version(),
                versioned(evidence.binding_identity()),
                versioned(evidence.model_identity()),
                versioned(evidence.session_locator()),
                match source {
                    BackendResumeSource::ContinuationAnchor(source_anchor_sequence) => {
                        BindingTransition::new(
                            TransitionMode::ExactReplay,
                            CacheState::Lost,
                            Some(source_anchor_sequence),
                        )
                    },
                    BackendResumeSource::ContextCheckpoint(source_checkpoint_sequence) => {
                        BindingTransition::new(TransitionMode::ExactReplay, CacheState::Lost, None)
                            .with_source_checkpoint_sequence(source_checkpoint_sequence)
                    },
                    BackendResumeSource::InitialFork(sequence) => {
                        BindingTransition::new(TransitionMode::ExactReplay, CacheState::Lost, None)
                            .with_source_initial_fork_sequence(sequence)
                    },
                },
                evidence.continuation_strategy(),
            )),
        ])
    }

    pub(crate) fn commit_native_model_rebind(
        &mut self,
        previous_epoch: u64,
        epoch: u64,
        source_anchor_sequence: Option<JournalSequence>,
        evidence: BackendBindingEvidence,
    ) -> bool {
        use super::codec::{BackendBindingClosed, BindingCloseReason};

        self.append_records_transactionally(vec![
            SemanticRecord::BackendBindingClosed(BackendBindingClosed::new(
                previous_epoch,
                BindingCloseReason::Replaced,
            )),
            SemanticRecord::BackendBindingOpened(BackendBindingOpened::new(
                epoch,
                evidence.backend_kind(),
                evidence.backend_version(),
                versioned(evidence.binding_identity()),
                versioned(evidence.model_identity()),
                versioned(evidence.session_locator()),
                BindingTransition::new(
                    TransitionMode::BackendNativeModelRebind,
                    CacheState::Unknown,
                    source_anchor_sequence,
                ),
                evidence.continuation_strategy(),
            )),
        ])
    }

    pub(crate) fn append_accepted_submission(
        &mut self,
        command: AgentCommand,
        submission_id: SubmissionId,
        events: &[AgentEvent],
        epoch: u64,
        context_epoch: Option<u64>,
        evidence: BackendRequestEvidence,
    ) -> JournalSequence {
        let turn = submission_turn(&command);
        let committed = CommittedCommand::submission(command, submission_id)
            .expect("only StartTurn or SteerTurn may carry accepted request evidence");
        let first_sequence = read_state(&self.state).next_sequence();
        let exchange_sequence = first_sequence.advance_by(events.len() + 1);
        let accepted_sequence = first_sequence.advance_by(events.len() + 2);
        let operation_id = OperationId::from(submission_id);
        let records = std::iter::once(SemanticRecord::CommandCommitted(committed))
            .chain(events.iter().cloned().map(SemanticRecord::EventCommitted))
            .chain([
                SemanticRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                    epoch,
                    operation_id,
                    ExchangeKind::Request,
                    ExchangeDirection::YoToBackend,
                    evidence.payload_schema(),
                    None,
                    Some(versioned(evidence.exchange_identity())),
                    DetailAvailability::Unpersisted,
                )),
                SemanticRecord::BackendRequestAccepted({
                    let accepted = BackendRequestAccepted::new(
                        epoch,
                        turn.turn_id(),
                        operation_id,
                        exchange_sequence,
                        versioned(evidence.request_identity()),
                    );
                    context_epoch
                        .map_or(accepted.clone(), |value| accepted.with_context_epoch(value))
                }),
            ])
            .collect();
        self.append_records(records);
        accepted_sequence
    }

    pub(crate) fn append_accepted_request(
        &mut self,
        turn: TurnRef,
        epoch: u64,
        context_epoch: u64,
        evidence: BackendRequestEvidence,
    ) -> JournalSequence {
        let first_sequence = read_state(&self.state).next_sequence();
        let exchange_sequence = first_sequence;
        let accepted_sequence = first_sequence.advance_by(1);
        let operation_id =
            OperationId::for_internal_request(turn.session_id(), turn.turn_id(), exchange_sequence);
        self.append_records(vec![
            SemanticRecord::BackendExchangeObserved(BackendExchangeObserved::new(
                epoch,
                operation_id,
                ExchangeKind::Request,
                ExchangeDirection::YoToBackend,
                evidence.payload_schema(),
                None,
                Some(versioned(evidence.exchange_identity())),
                DetailAvailability::Unpersisted,
            )),
            SemanticRecord::BackendRequestAccepted(
                BackendRequestAccepted::new(
                    epoch,
                    turn.turn_id(),
                    operation_id,
                    exchange_sequence,
                    versioned(evidence.request_identity()),
                )
                .with_context_epoch(context_epoch),
            ),
        ]);
        accepted_sequence
    }

    pub(crate) fn append_resumable_turn(
        &mut self,
        event: &AgentEvent,
        epoch: u64,
        context_epoch: Option<u64>,
        accepted_request_sequence: JournalSequence,
        continuation_strategy: ContinuationStrategy,
        evidence: BackendOutcomeEvidence,
    ) -> JournalSequence {
        let AgentEvent::TurnFinished {
            turn,
            outcome: TurnOutcome::Completed,
        } = event
        else {
            panic!("only a completed Turn may publish a resumable outcome");
        };
        let first_sequence = read_state(&self.state).next_sequence();
        let mut records = vec![SemanticRecord::EventCommitted(event.clone())];
        let replay_delta_sequence = match continuation_strategy {
            ContinuationStrategy::ExactReplay { .. } => {
                let delta = evidence
                    .model_replay()
                    .cloned()
                    .expect("exact replay completion requires a model replay delta");
                let sequence = first_sequence.advance_by(1);
                records.push(SemanticRecord::ModelReplayDelta({
                    let replay = ModelReplayDeltaRecord::new(
                        epoch,
                        turn.turn_id(),
                        accepted_request_sequence,
                        delta,
                    );
                    context_epoch.map_or(replay.clone(), |value| replay.with_context_epoch(value))
                }));
                Some(sequence)
            },
            ContinuationStrategy::BackendManagedState => {
                assert!(
                    evidence.model_replay().is_none(),
                    "backend-managed completion must not carry a model replay delta"
                );
                None
            },
        };
        let outcome_sequence = first_sequence.advance_by(records.len());
        records.extend([
            SemanticRecord::BackendResumableOutcome({
                let outcome = BackendResumableOutcome::new(
                    epoch,
                    turn.turn_id(),
                    accepted_request_sequence,
                    evidence.outcome_identity().map(versioned),
                    replay_delta_sequence,
                );
                context_epoch.map_or(outcome.clone(), |value| outcome.with_context_epoch(value))
            }),
            SemanticRecord::ContinuationAnchor({
                let anchor = ContinuationAnchor::new(
                    epoch,
                    accepted_request_sequence,
                    outcome_sequence,
                    outcome_sequence,
                );
                context_epoch.map_or(anchor.clone(), |value| anchor.with_context_epoch(value))
            }),
        ]);
        self.append_records(records);
        outcome_sequence.advance_by(1)
    }
}

fn context_source_groups(
    entries: &[super::JournalEntry],
    epoch: u64,
    context_epoch: u64,
) -> Option<Vec<ContextSourceGroup>> {
    let mut groups = Vec::new();
    let mut registered = None;
    let mut owner_epoch = None;
    let mut accepted_since_seed = false;
    let mut current_context = None;
    let mut closed_owner = None;
    let mut source_anchor = None;
    let mut source_checkpoint = None;
    for (index, entry) in entries.iter().enumerate() {
        match entry.record() {
            SemanticRecord::InitialForkSeed(seed) => {
                if registered.is_some() {
                    return None;
                }
                registered = Some((entry.sequence(), seed));
            },
            SemanticRecord::BackendRequestAccepted(_) if registered.is_some() => {
                accepted_since_seed = true;
                source_anchor = None;
                source_checkpoint = None;
            },
            SemanticRecord::BackendBindingClosed(binding) if registered.is_some() => {
                if owner_epoch != Some(binding.epoch()) {
                    return None;
                }
                closed_owner = Some((binding.epoch(), binding.reason()));
            },
            SemanticRecord::BackendBindingOpened(binding) if registered.is_some() => {
                let (seed_sequence, seed) = registered?;
                if matches!(seed.seed(), ForkSeed::Empty) {
                    // Empty forks carry no imported baseline; ordinary recovery owns bindings.
                    registered = None;
                    owner_epoch = Some(binding.epoch());
                    current_context = Some(1);
                    continue;
                }
                let ForkSeed::ExactReplay(replay) = seed.seed() else {
                    return None;
                };
                let source = seed.source().point()?.binding();
                if (owner_epoch.is_none()
                    || groups
                        .iter()
                        .any(|group: &ContextSourceGroup| group.fork_import.is_some()))
                    && (binding.backend_kind() != source.backend_kind()
                        || binding.binding_identity().schema()
                            != source.binding_identity().schema()
                        || binding.binding_identity().value() != source.binding_identity().value()
                        || binding.model_identity().schema() != source.model_identity().schema()
                        || binding.model_identity().value() != source.model_identity().value()
                        || binding.continuation_strategy() != source.continuation_strategy())
                {
                    return None;
                }
                if owner_epoch.is_none() {
                    if binding.epoch() != 1
                        || binding.transition().mode() != TransitionMode::InitialFork
                        || binding.transition().fork_seed_sequence() != Some(seed_sequence)
                    {
                        return None;
                    }
                    groups = replay
                        .groups()
                        .iter()
                        .enumerate()
                        .map(|(group_index, group)| {
                            let range = group.first_item()..group.end_item();
                            Some(ContextSourceGroup {
                                first_sequence: seed_sequence,
                                last_sequence: seed_sequence,
                                replay_sequence: seed_sequence,
                                image_losses: ContextImageLoss::for_items(
                                    &replay.items()[range.clone()],
                                    1,
                                    |item_index, part_index| ContextImageSource::InitialForkSeed {
                                        sequence: seed_sequence.get(),
                                        group_index: group_index as u32,
                                        item_index,
                                        part_index,
                                    },
                                )
                                .ok()?,
                                items: replay.items()[range.clone()].to_vec(),
                                fork_import: Some((seed_sequence, group_index)),
                                private_epochs: replay.items()[range.clone()]
                                    .iter()
                                    .zip(&replay.item_origins()[range])
                                    .filter_map(|(item, origin)| {
                                        matches!(
                                            item,
                                            ModelReplayItem::ProviderPrivateAssistant { .. }
                                        )
                                        .then_some(origin.original().binding_epoch())
                                    })
                                    .collect(),
                            })
                        })
                        .collect::<Option<Vec<_>>>()?;
                    current_context = Some(1);
                } else {
                    if closed_owner
                        != owner_epoch.map(|owner| (owner, BindingCloseReason::Replaced))
                        || owner_epoch.and_then(|owner: u64| owner.checked_add(1))
                            != Some(binding.epoch())
                    {
                        return None;
                    }
                    if binding.transition().mode() == TransitionMode::ExactReplay {
                        let valid_source = match (
                            binding.transition().source_anchor_sequence(),
                            binding.transition().source_checkpoint_sequence(),
                            binding.transition().source_initial_fork_sequence(),
                        ) {
                            (Some(sequence), None, None) => source_anchor == Some(sequence),
                            (None, Some(sequence), None) => source_checkpoint == Some(sequence),
                            (None, None, Some(sequence)) => {
                                sequence == seed_sequence
                                    && !accepted_since_seed
                                    && source_checkpoint.is_none()
                            },
                            _ => false,
                        };
                        if !valid_source {
                            return None;
                        }
                        groups = transfer_context_groups(groups, entry.sequence());
                    } else {
                        if groups.iter().any(|group| group.fork_import.is_some()) {
                            return None;
                        }
                        groups.clear();
                    }
                }
                owner_epoch = Some(binding.epoch());
                closed_owner = None;
            },
            SemanticRecord::BackendBindingOpened(binding) => {
                if owner_epoch.is_some() {
                    if binding.transition().mode() == TransitionMode::ExactReplay {
                        groups = transfer_context_groups(groups, entry.sequence());
                    } else {
                        groups.clear();
                    }
                } else {
                    current_context = Some(1);
                }
                owner_epoch = Some(binding.epoch());
                closed_owner = None;
            },
            SemanticRecord::ContextCheckpoint(checkpoint)
                if Some(checkpoint.epoch()) == owner_epoch
                    || (registered.is_none()
                        && checkpoint.epoch() == epoch
                        && checkpoint.successor_context_epoch() == context_epoch) =>
            {
                let root = checkpoint.replay_root().ok()?;
                if owner_epoch.is_some() {
                    if current_context != Some(checkpoint.previous_context_epoch()) {
                        return None;
                    }
                    current_context = Some(checkpoint.successor_context_epoch());
                    source_checkpoint = Some(entry.sequence());
                    source_anchor = None;
                }
                groups.clear();
                let retained_image_losses = checkpoint
                    .retained_groups()
                    .iter()
                    .enumerate()
                    .map(|(group_index, group)| {
                        ContextImageLoss::for_items(
                            group.items(),
                            checkpoint.successor_context_epoch(),
                            |item_index, part_index| ContextImageSource::RetainedCheckpoint {
                                sequence: entry.sequence().get(),
                                group_index: group_index as u32,
                                item_index,
                                part_index,
                            },
                        )
                        .ok()
                    })
                    .collect::<Option<Vec<_>>>()?;
                if checkpoint
                    .retained_groups()
                    .iter()
                    .any(|group| group.fork_import().is_some())
                {
                    groups.push(local_context_group(
                        entry.sequence(),
                        root.items()[..1].to_vec(),
                        Vec::new(),
                    ));
                    let mut local_tail = Vec::new();
                    let mut local_losses = Vec::new();
                    for (group_index, retained) in checkpoint.retained_groups().iter().enumerate() {
                        if let Some(import) = retained.fork_import() {
                            if !local_tail.is_empty() {
                                return None;
                            }
                            groups.push(ContextSourceGroup {
                                first_sequence: import.0,
                                last_sequence: import.0,
                                replay_sequence: import.0,
                                items: retained.items().to_vec(),
                                fork_import: Some(import),
                                private_epochs: retained.private_epochs().to_vec(),
                                image_losses: retained_image_losses[group_index].clone(),
                            });
                        } else {
                            local_tail.extend_from_slice(retained.items());
                            local_losses.extend(retained_image_losses[group_index].iter().cloned());
                        }
                    }
                    if !local_tail.is_empty() {
                        groups.push(local_context_group(
                            entry.sequence(),
                            local_tail,
                            local_losses,
                        ));
                    }
                    if !groups
                        .iter()
                        .flat_map(|group| &group.items)
                        .eq(root.items())
                    {
                        return None;
                    }
                } else {
                    groups.push(local_context_group(
                        entry.sequence(),
                        root.items().to_vec(),
                        retained_image_losses.into_iter().flatten().collect(),
                    ));
                }
            },
            SemanticRecord::ContinuationAnchor(anchor)
                if (Some(anchor.epoch()) == owner_epoch
                    && anchor.context_epoch() == current_context)
                    || (registered.is_none()
                        && anchor.epoch() == epoch
                        && anchor.context_epoch() == Some(context_epoch)) =>
            {
                source_anchor = Some(entry.sequence());
                source_checkpoint = None;
                let outcome = entries.iter().find(|candidate| {
                    candidate.sequence() == anchor.resumable_outcome_sequence()
                })?;
                let SemanticRecord::BackendResumableOutcome(outcome) = outcome.record() else {
                    return None;
                };
                let replay_sequence = outcome.replay_delta_sequence()?;
                let replay_index = entries
                    .iter()
                    .position(|candidate| candidate.sequence() == replay_sequence)?;
                let SemanticRecord::ModelReplayDelta(delta) = entries[replay_index].record() else {
                    return None;
                };
                let first = replay_index
                    .checked_sub(1)
                    .and_then(|position| entries.get(position))?;
                if !matches!(
                    first.record(),
                    SemanticRecord::EventCommitted(AgentEvent::TurnFinished {
                        outcome: TurnOutcome::Completed,
                        ..
                    })
                ) || index <= replay_index
                {
                    return None;
                }
                groups.push(ContextSourceGroup {
                    first_sequence: first.sequence(),
                    last_sequence: anchor.journal_boundary(),
                    replay_sequence,
                    image_losses: ContextImageLoss::for_items(
                        delta.delta().items(),
                        delta.context_epoch()?,
                        |item_index, part_index| ContextImageSource::ReplayDelta {
                            sequence: replay_sequence.get(),
                            item_index,
                            part_index,
                        },
                    )
                    .ok()?,
                    items: delta.delta().items().to_vec(),
                    fork_import: None,
                    private_epochs: Vec::new(),
                });
            },
            _ => {},
        }
    }
    if registered.is_some()
        && (owner_epoch != Some(epoch)
            || current_context != Some(context_epoch)
            || closed_owner.is_some())
    {
        return None;
    }
    (!groups.is_empty()).then_some(groups)
}

fn transfer_context_groups(
    groups: Vec<ContextSourceGroup>,
    sequence: JournalSequence,
) -> Vec<ContextSourceGroup> {
    let mut transferred = Vec::new();
    let mut local = Vec::new();
    let mut local_losses = Vec::new();
    for group in groups {
        if group.fork_import.is_some() {
            if !local.is_empty() {
                transferred.push(local_context_group(
                    sequence,
                    mem::take(&mut local),
                    mem::take(&mut local_losses),
                ));
            }
            transferred.push(group);
        } else {
            local.extend(group.items);
            local_losses.extend(group.image_losses);
        }
    }
    if !local.is_empty() {
        transferred.push(local_context_group(sequence, local, local_losses));
    }
    transferred
}

fn local_context_group(
    sequence: JournalSequence,
    items: Vec<ModelReplayItem>,
    image_losses: Vec<ContextImageLoss>,
) -> ContextSourceGroup {
    ContextSourceGroup {
        first_sequence: sequence,
        last_sequence: sequence,
        replay_sequence: sequence,
        items,
        fork_import: None,
        private_epochs: Vec::new(),
        image_losses,
    }
}

fn submission_turn(command: &AgentCommand) -> TurnRef {
    match command {
        AgentCommand::StartTurn { turn, .. } | AgentCommand::SteerTurn { turn, .. } => *turn,
        _ => unreachable!("only a submission command reaches accepted request persistence"),
    }
}

fn versioned(identity: &crate::BackendIdentity) -> VersionedIdentity {
    VersionedIdentity::new(identity.schema(), identity.value())
}

#[cfg(test)]
// 두 번째 압축의 source는 새 요약 본문, 원본 import, child local tail 순서를 유지한다.
// 동일 checkpoint 좌표의 앞·뒤 local group을 합쳐 import 앞으로 옮기면 이 검증이 실패한다.
#[test]
fn imported_checkpoint_sources_preserve_root_order_and_private_origin_epochs() {
    use super::JournalEntry;
    use crate::{
        ModelReplayContract, ProviderPrivateReplayEnvelope, ReplayProfile, provider_private_schema,
    };

    let private = ModelReplayItem::ProviderPrivateAssistant {
        envelope: ProviderPrivateReplayEnvelope::new(
            provider_private_schema(ReplayProfile::ProviderPrivateLocalPlaintext).unwrap(),
            br#"{"reasoning_content":"original private bytes","content":"inherited"}"#.to_vec(),
        )
        .unwrap(),
    };
    let inherited = vec![
        ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "inherited".into(),
            refusal: None,
        },
        private,
    ];
    let tail = ModelReplayItem::Message {
        role: ModelReplayRole::User,
        content: "child tail".into(),
        refusal: None,
    };
    let body = "# Context Checkpoint\n## Current Objective\nContinue.\n## Active Constraints\nNone.\n## Decisions\nKeep imports.\n## Verified Progress\nDone.\n## Current State\nIdle.\n## Unknown or Unverified\nNone.\n## Next Actions\nContinue.\n## Critical References\nNone.";
    let usage = ContextSummaryUsage::try_new(serde_json::json!({
        "schema":"yo.model-usage-receipt/v1", "response_id":"summary", "round":1,
        "provider":"test", "account":"default", "model":"test", "connector":"openai-responses",
        "api_dialect":"openai-responses", "base_url":"https://example.invalid/",
        "usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120,"reasoning_tokens":0},
        "cache_read_input_tokens":{"availability":"unsupported"}
    }))
    .unwrap();
    let checkpoint = ContextCheckpoint::try_new(
        1,
        1,
        2,
        JournalSequence::new(19),
        JournalSequence::new(18),
        1,
        ContextStrategy::PortableSummaryV1Alpha1,
        1000,
        900,
        100,
        ModelReplayContract::new("system", vec![]),
        body,
        vec![
            ContextRetainedGroup::try_imported(
                JournalSequence::new(2),
                1,
                inherited.clone(),
                vec![3],
            )
            .unwrap(),
            ContextRetainedGroup::try_new(
                JournalSequence::new(15),
                JournalSequence::new(18),
                vec![tail.clone()],
            )
            .unwrap(),
        ],
        Some(JournalSequence::new(2)),
        vec![],
        vec![
            ContextLoss::visible_prefix_summarized(
                JournalSequence::new(2),
                JournalSequence::new(2),
            )
            .unwrap(),
        ],
        usage,
    )
    .unwrap();
    let expected = checkpoint.replay_root().unwrap();
    let entries = vec![JournalEntry::new(
        JournalSequence::new(20),
        SemanticRecord::ContextCheckpoint(checkpoint),
    )];
    let groups = context_source_groups(&entries, 1, 2).unwrap();
    assert_eq!(groups.len(), 3);
    assert_eq!(groups[0].items, expected.items()[..1]);
    assert_eq!(groups[0].first_sequence, JournalSequence::new(20));
    assert_eq!(groups[1].items, inherited);
    assert_eq!(groups[1].fork_import, Some((JournalSequence::new(2), 1)));
    assert_eq!(groups[1].private_epochs, vec![3]);
    assert_eq!(groups[2].items, vec![tail]);
    assert_eq!(groups[2].first_sequence, JournalSequence::new(20));
    assert!(
        groups
            .iter()
            .flat_map(|group| &group.items)
            .eq(expected.items())
    );
    assert_eq!(
        groups[..2].iter().map(|group| group.first_sequence).min(),
        Some(JournalSequence::new(2))
    );
    assert_eq!(
        groups[..2].iter().map(|group| group.last_sequence).max(),
        Some(JournalSequence::new(20))
    );
    // exact replacement를 두 번 거쳐도 import를 local run에 흡수하지 않는다.
    let transferred = transfer_context_groups(groups, JournalSequence::new(30));
    assert_eq!(transferred.len(), 3);
    assert_eq!(transferred[0].first_sequence, JournalSequence::new(30));
    assert_eq!(
        transferred[1].fork_import,
        Some((JournalSequence::new(2), 1))
    );
    assert_eq!(transferred[1].private_epochs, vec![3]);
    assert_eq!(transferred[2].first_sequence, JournalSequence::new(30));
    assert!(
        transferred
            .iter()
            .flat_map(|group| &group.items)
            .eq(expected.items())
    );
    let twice = transfer_context_groups(transferred, JournalSequence::new(40));
    assert_eq!(twice[0].first_sequence, JournalSequence::new(40));
    assert_eq!(twice[1].fork_import, Some((JournalSequence::new(2), 1)));
    assert_eq!(twice[2].first_sequence, JournalSequence::new(40));
    assert!(
        twice
            .iter()
            .flat_map(|group| &group.items)
            .eq(expected.items())
    );
}
