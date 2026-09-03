use std::{collections::BTreeSet, fmt::Write as _};

use sha2::Digest as _;

use super::{
    CommittedCommand, JournalSequence, SemanticRecord, SessionJournal,
    codec::{
        BackendBindingOpened, BackendExchangeObserved, BackendRequestAccepted,
        BackendResumableOutcome, BindingTransition, CacheState, ContextArtifactReceipt,
        ContextCheckpoint, ContextLoss, ContextPolicyChanged, ContextRetainedGroup,
        ContextStrategy, ContextSummaryUsage, ContinuationAnchor, DetailAvailability,
        ExchangeDirection, ExchangeKind, ModelReplayDeltaRecord, OperationId, TransitionMode,
        VersionedIdentity,
    },
    read_state,
};
use crate::{
    AgentCommand, AgentEvent, BackendBindingEvidence, BackendOutcomeEvidence,
    BackendRequestEvidence, BackendResumeSource, ContextCheckpointProposal, ContinuationStrategy,
    ModelReplay, ModelReplayItem, ModelReplayRole, SubmissionId, TurnOutcome, TurnRef,
};

#[derive(Clone)]
struct ContextSourceGroup {
    first_sequence: JournalSequence,
    last_sequence: JournalSequence,
    replay_sequence: JournalSequence,
    items: Vec<ModelReplayItem>,
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
                ContextRetainedGroup::try_new(
                    group.first_sequence,
                    group.last_sequence,
                    group.items.clone(),
                )
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
                        } if *candidate == turn => Some((entry.sequence(), input.as_str())),
                        _ => None,
                    },
                    _ => None,
                })?
            })?;
            let expected_input = ModelReplayItem::Message {
                role: ModelReplayRole::User,
                content: input.to_owned(),
                refusal: None,
            };
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
                summarized.first()?.first_sequence,
                summarized.last()?.last_sequence,
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
        let first_retained_sequence = retained.first().map(ContextRetainedGroup::first_sequence);
        let checkpoint = ContextCheckpoint::try_new(
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
    for (index, entry) in entries.iter().enumerate() {
        match entry.record() {
            SemanticRecord::ContextCheckpoint(checkpoint)
                if checkpoint.epoch() == epoch
                    && checkpoint.successor_context_epoch() == context_epoch =>
            {
                groups.clear();
                groups.push(ContextSourceGroup {
                    first_sequence: entry.sequence(),
                    last_sequence: entry.sequence(),
                    replay_sequence: entry.sequence(),
                    items: checkpoint.replay_root().ok()?.items().to_vec(),
                });
            },
            SemanticRecord::ContinuationAnchor(anchor)
                if anchor.epoch() == epoch && anchor.context_epoch() == Some(context_epoch) =>
            {
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
                    items: delta.delta().items().to_vec(),
                });
            },
            _ => {},
        }
    }
    (!groups.is_empty()).then_some(groups)
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
