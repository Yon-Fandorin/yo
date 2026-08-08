use super::codec::{
    BackendBindingClosed, BackendBindingOpened, BackendExchangeObserved, BackendRequestAccepted,
    BackendResumableOutcome, ContinuationAnchor, ModelReplayDeltaRecord,
};
use crate::{AgentCommand, AgentEvent, SubmissionId};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct JournalSequence(u64);

impl JournalSequence {
    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }

    pub(super) fn from_index(index: usize) -> Self {
        let value = index
            .checked_add(1)
            .and_then(|value| u64::try_from(value).ok())
            .expect("a Journal cannot contain more entries than u64 can address");
        Self(value)
    }

    pub(super) fn advance_by(self, offset: usize) -> Self {
        let offset = u64::try_from(offset).expect("a Journal offset must fit u64");
        Self(
            self.0
                .checked_add(offset)
                .expect("a Journal sequence must not overflow u64"),
        )
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Internal Journal meaning mirrored deliberately by the public
/// `TranscriptRecord` projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SemanticRecord {
    CommandCommitted(CommittedCommand),
    EventCommitted(AgentEvent),
    BackendExchangeObserved(BackendExchangeObserved),
    BackendBindingOpened(BackendBindingOpened),
    BackendBindingClosed(BackendBindingClosed),
    BackendRequestAccepted(BackendRequestAccepted),
    ModelReplayDelta(ModelReplayDeltaRecord),
    BackendResumableOutcome(BackendResumableOutcome),
    ContinuationAnchor(ContinuationAnchor),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CommittedCommand {
    command: AgentCommand,
    correlation: CommandCorrelation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommandCorrelation {
    Submission(SubmissionId),
    Uncorrelated,
}

impl CommittedCommand {
    pub(crate) fn submission(command: AgentCommand, submission_id: SubmissionId) -> Option<Self> {
        matches!(
            command,
            AgentCommand::StartTurn { .. } | AgentCommand::SteerTurn { .. }
        )
        .then_some(Self {
            command,
            correlation: CommandCorrelation::Submission(submission_id),
        })
    }

    pub(crate) fn uncorrelated(command: AgentCommand) -> Option<Self> {
        (!matches!(
            command,
            AgentCommand::StartTurn { .. } | AgentCommand::SteerTurn { .. }
        ))
        .then_some(Self {
            command,
            correlation: CommandCorrelation::Uncorrelated,
        })
    }

    pub(crate) const fn command(&self) -> &AgentCommand {
        &self.command
    }

    pub(crate) const fn submission_id(&self) -> Option<SubmissionId> {
        match self.correlation {
            CommandCorrelation::Submission(submission_id) => Some(submission_id),
            CommandCorrelation::Uncorrelated => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct JournalEntry {
    sequence: JournalSequence,
    record: SemanticRecord,
}

impl JournalEntry {
    pub(super) const fn new(sequence: JournalSequence, record: SemanticRecord) -> Self {
        Self { sequence, record }
    }

    pub(crate) const fn sequence(&self) -> JournalSequence {
        self.sequence
    }

    pub(crate) const fn record(&self) -> &SemanticRecord {
        &self.record
    }
}
