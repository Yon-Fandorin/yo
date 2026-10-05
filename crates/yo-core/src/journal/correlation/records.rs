use std::iter;

use super::super::{
    CommittedCommand, JournalSequence, SemanticRecord, SessionJournal,
    codec::{
        BackendBindingOpened, BackendExchangeObserved, BackendRequestAccepted,
        BackendResumableOutcome, BindingTransition, CacheState, ContinuationAnchor,
        DetailAvailability, ExchangeDirection, ExchangeKind, ModelReplayDeltaRecord, OperationId,
        TransitionMode, VersionedIdentity,
    },
    read_state,
};
use crate::{
    AgentCommand, AgentEvent, BackendBindingEvidence, BackendOutcomeEvidence,
    BackendRequestEvidence, BackendResumeSource, ContinuationStrategy, SubmissionId, TurnOutcome,
    TurnRef,
};

impl SessionJournal {
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
        use super::super::codec::{BackendBindingClosed, BindingCloseReason};

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
        use super::super::codec::{BackendBindingClosed, BindingCloseReason};

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
        let records = iter::once(SemanticRecord::CommandCommitted(committed))
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

    /// Failed, 선택적 차분, 로컬 정산, Anchor를 한 물리 추기로 확정합니다.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn commit_local_failure(
        &mut self,
        event: &AgentEvent,
        epoch: u64,
        context_epoch: u64,
        accepted_request_sequence: JournalSequence,
        source: super::super::codec::LocalFailureSource,
        replay: Option<crate::ModelReplayDelta>,
    ) -> Option<JournalSequence> {
        let AgentEvent::TurnFinished {
            turn,
            outcome: TurnOutcome::Failed(_),
        } = event
        else {
            return None;
        };
        if matches!(
            source,
            super::super::codec::LocalFailureSource::ActiveSuffix { .. }
        ) != replay.is_some()
        {
            return None;
        }
        let first = read_state(&self.state).next_sequence();
        let mut records = vec![SemanticRecord::EventCommitted(event.clone())];
        let replay_sequence = replay.map(|delta| {
            records.push(SemanticRecord::ModelReplayDelta(
                ModelReplayDeltaRecord::new(
                    epoch,
                    turn.turn_id(),
                    accepted_request_sequence,
                    delta,
                )
                .with_context_epoch(context_epoch),
            ));
            first.advance_by(1)
        });
        let outcome_sequence = first.advance_by(records.len());
        records.push(SemanticRecord::BackendResumableOutcome(
            BackendResumableOutcome::local_failure(
                epoch,
                context_epoch,
                turn.turn_id(),
                accepted_request_sequence,
                replay_sequence,
                source,
            ),
        ));
        records.push(SemanticRecord::ContinuationAnchor(
            ContinuationAnchor::new(
                epoch,
                accepted_request_sequence,
                outcome_sequence,
                outcome_sequence,
            )
            .with_context_epoch(context_epoch),
        ));
        self.append_records_transactionally(records)
            .then_some(outcome_sequence.advance_by(1))
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
fn submission_turn(command: &AgentCommand) -> TurnRef {
    match command {
        AgentCommand::StartTurn { turn, .. } | AgentCommand::SteerTurn { turn, .. } => *turn,
        _ => unreachable!("only a submission command reaches accepted request persistence"),
    }
}

fn versioned(identity: &crate::BackendIdentity) -> VersionedIdentity {
    VersionedIdentity::new(identity.schema(), identity.value())
}
