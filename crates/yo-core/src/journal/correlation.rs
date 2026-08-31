use super::{
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

    pub(crate) fn append_accepted_submission(
        &mut self,
        command: AgentCommand,
        submission_id: SubmissionId,
        events: &[AgentEvent],
        epoch: u64,
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
                SemanticRecord::BackendRequestAccepted(BackendRequestAccepted::new(
                    epoch,
                    turn.turn_id(),
                    operation_id,
                    exchange_sequence,
                    versioned(evidence.request_identity()),
                )),
            ])
            .collect();
        self.append_records(records);
        accepted_sequence
    }

    pub(crate) fn append_resumable_turn(
        &mut self,
        event: &AgentEvent,
        epoch: u64,
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
                records.push(SemanticRecord::ModelReplayDelta(
                    ModelReplayDeltaRecord::new(
                        epoch,
                        turn.turn_id(),
                        accepted_request_sequence,
                        delta,
                    ),
                ));
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
            SemanticRecord::BackendResumableOutcome(BackendResumableOutcome::new(
                epoch,
                turn.turn_id(),
                accepted_request_sequence,
                evidence.outcome_identity().map(versioned),
                replay_delta_sequence,
            )),
            SemanticRecord::ContinuationAnchor(ContinuationAnchor::new(
                epoch,
                accepted_request_sequence,
                outcome_sequence,
                outcome_sequence,
            )),
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
