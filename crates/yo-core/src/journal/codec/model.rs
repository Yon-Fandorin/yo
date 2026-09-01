use super::{
    BackendBindingClosed, BackendBindingOpened, BackendExchangeObserved, BackendRequestAccepted,
    BackendResumableOutcome, ContextCheckpoint, ContextPolicyChanged, ContinuationAnchor,
    ModelReplayDeltaRecord,
};
use crate::{
    ActivityKind, ActivityRef, AgentCommand, AgentEvent, JournalSequence, SessionDescriptor,
    SessionId, journal::CommittedCommand,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JournalCommitKind {
    Incremental,
    Snapshot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct JournalCommit {
    kind: JournalCommitKind,
    journal_cutoff: Option<JournalSequence>,
    records: Vec<SequencedJournalRecord>,
}

impl JournalCommit {
    #[cfg(test)]
    pub(crate) fn incremental(records: Vec<SequencedJournalRecord>) -> Self {
        let cutoff = inferred_test_cutoff(&records);
        Self::incremental_through(cutoff, records)
    }

    pub(crate) fn incremental_through(
        journal_cutoff: JournalSequence,
        records: Vec<SequencedJournalRecord>,
    ) -> Self {
        Self {
            kind: JournalCommitKind::Incremental,
            journal_cutoff: Some(journal_cutoff),
            records,
        }
    }

    #[cfg(test)]
    pub(crate) fn snapshot(records: Vec<SequencedJournalRecord>) -> Self {
        let cutoff = inferred_test_cutoff(&records);
        Self::snapshot_through(cutoff, records)
    }

    pub(crate) fn snapshot_through(
        journal_cutoff: JournalSequence,
        records: Vec<SequencedJournalRecord>,
    ) -> Self {
        Self {
            kind: JournalCommitKind::Snapshot,
            journal_cutoff: Some(journal_cutoff),
            records,
        }
    }

    pub(crate) fn descriptor(descriptor: SessionDescriptor) -> Self {
        Self {
            kind: JournalCommitKind::Incremental,
            journal_cutoff: None,
            records: vec![SequencedJournalRecord::storage(
                ReplaySequence::new(1),
                JournalRecord::SessionDescriptor(descriptor),
            )],
        }
    }

    pub(super) fn decoded(
        kind: JournalCommitKind,
        journal_cutoff: Option<JournalSequence>,
        records: Vec<SequencedJournalRecord>,
    ) -> Self {
        Self {
            kind,
            journal_cutoff,
            records,
        }
    }

    pub(crate) const fn kind(&self) -> JournalCommitKind {
        self.kind
    }

    pub(crate) fn records(&self) -> &[SequencedJournalRecord] {
        &self.records
    }

    pub(crate) const fn journal_cutoff(&self) -> Option<JournalSequence> {
        self.journal_cutoff
    }
}

#[cfg(test)]
fn inferred_test_cutoff(records: &[SequencedJournalRecord]) -> JournalSequence {
    records
        .iter()
        .rev()
        .find_map(SequencedJournalRecord::journal_sequence)
        .unwrap_or_else(|| {
            JournalSequence::new(
                records
                    .last()
                    .expect("a Journal commit cannot be empty")
                    .sequence()
                    .get(),
            )
        })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SequencedJournalRecord {
    sequence: ReplaySequence,
    journal_sequence: Option<JournalSequence>,
    record: JournalRecord,
}

impl SequencedJournalRecord {
    #[cfg(test)]
    pub(crate) fn new(sequence: impl Into<ReplaySequence>, record: JournalRecord) -> Self {
        let sequence = sequence.into();
        let journal_sequence = record
            .requires_journal_sequence()
            .then_some(JournalSequence::new(sequence.get()));
        Self {
            sequence,
            journal_sequence,
            record,
        }
    }

    pub(crate) fn storage(sequence: ReplaySequence, record: JournalRecord) -> Self {
        assert!(
            !record.requires_journal_sequence(),
            "a semantic Journal record requires an explicit JournalSequence"
        );
        Self {
            sequence,
            journal_sequence: None,
            record,
        }
    }

    pub(crate) const fn with_journal_sequence(
        sequence: ReplaySequence,
        journal_sequence: JournalSequence,
        record: JournalRecord,
    ) -> Self {
        Self {
            sequence,
            journal_sequence: Some(journal_sequence),
            record,
        }
    }

    pub(super) const fn decoded(
        sequence: ReplaySequence,
        journal_sequence: Option<JournalSequence>,
        record: JournalRecord,
    ) -> Self {
        Self {
            sequence,
            journal_sequence,
            record,
        }
    }

    pub(crate) const fn sequence(&self) -> ReplaySequence {
        self.sequence
    }

    pub(crate) const fn record(&self) -> &JournalRecord {
        &self.record
    }

    pub(crate) const fn journal_sequence(&self) -> Option<JournalSequence> {
        self.journal_sequence
    }
}

/// Private order of normalized persistence records.
///
/// This coordinate is deliberately not `JournalSequence`: segment boundaries are a storage
/// detail and must not alter the semantic cutoff observed by frontends.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ReplaySequence(u64);

impl ReplaySequence {
    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }

    pub(crate) const fn get(self) -> u64 {
        self.0
    }
}

impl From<JournalSequence> for ReplaySequence {
    fn from(value: JournalSequence) -> Self {
        Self(value.get())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum JournalRecord {
    SessionDescriptor(SessionDescriptor),
    CommandCommitted(CommittedCommand),
    EventCommitted(AgentEvent),
    BackendExchangeObserved(BackendExchangeObserved),
    BackendBindingOpened(BackendBindingOpened),
    BackendBindingClosed(BackendBindingClosed),
    BackendRequestAccepted(BackendRequestAccepted),
    ModelReplayDelta(ModelReplayDeltaRecord),
    BackendResumableOutcome(BackendResumableOutcome),
    ContinuationAnchor(ContinuationAnchor),
    ContextPolicyChanged(ContextPolicyChanged),
    ContextCheckpoint(ContextCheckpoint),
    MessageReset(MessageReset),
    MessageSegment(MessageSegment),
    MessageEnded(MessageTerminal),
}

impl JournalRecord {
    pub(crate) fn semantic_record(&self) -> Option<crate::journal::SemanticRecord> {
        use crate::journal::SemanticRecord;
        match self {
            Self::CommandCommitted(record) => {
                Some(SemanticRecord::CommandCommitted(record.clone()))
            },
            Self::EventCommitted(record) => Some(SemanticRecord::EventCommitted(record.clone())),
            Self::BackendExchangeObserved(record) => {
                Some(SemanticRecord::BackendExchangeObserved(record.clone()))
            },
            Self::BackendBindingOpened(record) => {
                Some(SemanticRecord::BackendBindingOpened(record.clone()))
            },
            Self::BackendBindingClosed(record) => {
                Some(SemanticRecord::BackendBindingClosed(record.clone()))
            },
            Self::BackendRequestAccepted(record) => {
                Some(SemanticRecord::BackendRequestAccepted(record.clone()))
            },
            Self::ModelReplayDelta(record) => {
                Some(SemanticRecord::ModelReplayDelta(record.clone()))
            },
            Self::BackendResumableOutcome(record) => {
                Some(SemanticRecord::BackendResumableOutcome(record.clone()))
            },
            Self::ContinuationAnchor(record) => {
                Some(SemanticRecord::ContinuationAnchor(record.clone()))
            },
            Self::ContextPolicyChanged(record) => {
                Some(SemanticRecord::ContextPolicyChanged(record.clone()))
            },
            Self::ContextCheckpoint(record) => {
                Some(SemanticRecord::ContextCheckpoint(record.clone()))
            },
            Self::SessionDescriptor(_)
            | Self::MessageReset(_)
            | Self::MessageSegment(_)
            | Self::MessageEnded(_) => None,
        }
    }

    pub(crate) const fn session_id(&self) -> Option<SessionId> {
        match self {
            Self::SessionDescriptor(descriptor) => Some(descriptor.session_id()),
            Self::CommandCommitted(committed) => match committed.command() {
                AgentCommand::CreateSession { session_id } => Some(*session_id),
                AgentCommand::StartTurn { turn, .. }
                | AgentCommand::SteerTurn { turn, .. }
                | AgentCommand::InterruptTurn { turn } => Some(turn.session_id()),
                AgentCommand::RespondToActivity { request, .. } => {
                    Some(request.activity().session_id())
                },
                AgentCommand::CompactContext { .. } => None,
            },
            Self::EventCommitted(event) => match event {
                AgentEvent::SessionCreated { session_id } => Some(*session_id),
                AgentEvent::TurnStarted { turn } | AgentEvent::TurnFinished { turn, .. } => {
                    Some(turn.session_id())
                },
                AgentEvent::ActivityStarted { activity, .. }
                | AgentEvent::ActivityUpdated { activity, .. }
                | AgentEvent::ActivityFinished { activity, .. } => Some(activity.session_id()),
            },
            Self::MessageReset(reset) => Some(reset.activity().session_id()),
            Self::MessageSegment(segment) => Some(segment.activity().session_id()),
            Self::MessageEnded(terminal) => Some(terminal.ended().activity().session_id()),
            Self::BackendExchangeObserved(_)
            | Self::BackendBindingOpened(_)
            | Self::BackendBindingClosed(_)
            | Self::BackendRequestAccepted(_)
            | Self::ModelReplayDelta(_)
            | Self::BackendResumableOutcome(_)
            | Self::ContinuationAnchor(_)
            | Self::ContextPolicyChanged(_)
            | Self::ContextCheckpoint(_) => None,
        }
    }

    pub(crate) const fn requires_journal_sequence(&self) -> bool {
        !matches!(
            self,
            Self::SessionDescriptor(_)
                | Self::MessageReset(_)
                | Self::MessageSegment(_)
                | Self::MessageEnded(_)
        )
    }
}

/// Durable declaration that an authoritative replacement revision is empty.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MessageReset {
    activity: ActivityRef,
    stream: MessageStream,
    revision: u64,
}

impl MessageReset {
    pub(crate) const fn new(activity: ActivityRef, stream: MessageStream, revision: u64) -> Self {
        Self {
            activity,
            stream,
            revision,
        }
    }

    pub(crate) const fn activity(&self) -> ActivityRef {
        self.activity
    }

    pub(crate) const fn stream(&self) -> MessageStream {
        self.stream
    }

    pub(crate) const fn revision(&self) -> u64 {
        self.revision
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MessageStream {
    Agent,
    ToolOutput,
}

impl MessageStream {
    pub(crate) const fn for_activity(kind: ActivityKind) -> Self {
        match kind {
            ActivityKind::AgentMessage => Self::Agent,
            ActivityKind::ModelWork
            | ActivityKind::ToolCall
            | ActivityKind::ToolResult
            | ActivityKind::FileChange
            | ActivityKind::ApprovalRequest { .. }
            | ActivityKind::ApprovalResponse { .. }
            | ActivityKind::UserInputRequest { .. }
            | ActivityKind::UserInputResponse { .. } => Self::ToolOutput,
        }
    }

    pub(crate) const fn segment_limit(self) -> usize {
        match self {
            Self::Agent => 16 * 1024,
            Self::ToolOutput => 64 * 1024,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MessageSegment {
    activity: ActivityRef,
    stream: MessageStream,
    revision: u64,
    index: u64,
    text: String,
}

impl MessageSegment {
    #[cfg(test)]
    pub(crate) fn new(
        activity: ActivityRef,
        stream: MessageStream,
        index: u64,
        text: String,
    ) -> Self {
        Self::for_revision(activity, stream, 1, index, text)
    }

    pub(crate) fn for_revision(
        activity: ActivityRef,
        stream: MessageStream,
        revision: u64,
        index: u64,
        text: String,
    ) -> Self {
        Self {
            activity,
            stream,
            revision,
            index,
            text,
        }
    }

    pub(crate) const fn activity(&self) -> ActivityRef {
        self.activity
    }

    pub(crate) const fn stream(&self) -> MessageStream {
        self.stream
    }

    pub(crate) const fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) const fn index(&self) -> u64 {
        self.index
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MessageOutcome {
    Completed,
    Interrupted,
    Failed(crate::Failure),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MessageEnded {
    activity: ActivityRef,
    stream: MessageStream,
    revision: u64,
    outcome: MessageOutcome,
    segment_count: u64,
    utf8_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MessageTerminal {
    final_segment: Option<MessageSegment>,
    ended: MessageEnded,
}

impl MessageTerminal {
    pub(crate) const fn new(final_segment: Option<MessageSegment>, ended: MessageEnded) -> Self {
        Self {
            final_segment,
            ended,
        }
    }

    pub(crate) const fn final_segment(&self) -> Option<&MessageSegment> {
        self.final_segment.as_ref()
    }

    pub(crate) const fn ended(&self) -> &MessageEnded {
        &self.ended
    }
}

impl MessageEnded {
    #[cfg(test)]
    pub(crate) fn new(
        activity: ActivityRef,
        stream: MessageStream,
        outcome: MessageOutcome,
        segment_count: u64,
        utf8_bytes: u64,
    ) -> Self {
        Self::for_revision(activity, stream, 1, outcome, segment_count, utf8_bytes)
    }

    pub(crate) fn for_revision(
        activity: ActivityRef,
        stream: MessageStream,
        revision: u64,
        outcome: MessageOutcome,
        segment_count: u64,
        utf8_bytes: u64,
    ) -> Self {
        Self {
            activity,
            stream,
            revision,
            outcome,
            segment_count,
            utf8_bytes,
        }
    }

    pub(crate) const fn activity(&self) -> ActivityRef {
        self.activity
    }

    pub(crate) const fn stream(&self) -> MessageStream {
        self.stream
    }

    pub(crate) const fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) const fn outcome(&self) -> &MessageOutcome {
        &self.outcome
    }

    pub(crate) const fn segment_count(&self) -> u64 {
        self.segment_count
    }

    pub(crate) const fn utf8_bytes(&self) -> u64 {
        self.utf8_bytes
    }
}
