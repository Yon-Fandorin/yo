use super::{
    CommittedCommand, JournalSequence, SemanticRecord, SessionJournal,
    codec::{
        BackendBindingOpened, BackendExchangeObserved, BackendRequestAccepted,
        BackendResumableOutcome, BindingTransition, CacheState, ContinuationAnchor,
        DetailAvailability, ExchangeDirection, ExchangeKind, OperationId, TransitionMode,
        VersionedIdentity,
    },
    read_state,
};
use crate::{
    AgentCommand, AgentEvent, BackendBindingEvidence, BackendOutcomeEvidence,
    BackendRequestEvidence, SubmissionId, TurnOutcome, TurnRef,
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
            ),
        ));
        self.append_records(records);
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
        evidence: BackendOutcomeEvidence,
    ) {
        let AgentEvent::TurnFinished {
            turn,
            outcome: TurnOutcome::Completed,
        } = event
        else {
            panic!("only a completed Turn may publish a resumable outcome");
        };
        let outcome_sequence = read_state(&self.state).next_sequence().advance_by(1);
        self.append_records(vec![
            SemanticRecord::EventCommitted(event.clone()),
            SemanticRecord::BackendResumableOutcome(BackendResumableOutcome::new(
                epoch,
                turn.turn_id(),
                accepted_request_sequence,
                evidence.outcome_identity().map(versioned),
            )),
            SemanticRecord::ContinuationAnchor(ContinuationAnchor::new(
                epoch,
                accepted_request_sequence,
                outcome_sequence,
                outcome_sequence,
            )),
        ]);
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
