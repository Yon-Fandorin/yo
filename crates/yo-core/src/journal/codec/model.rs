use crate::{ActivityRef, AgentCommand, AgentEvent, JournalSequence, SessionId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JournalCommitKind {
    Incremental,
    Snapshot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct JournalCommit {
    kind: JournalCommitKind,
    records: Vec<SequencedJournalRecord>,
}

impl JournalCommit {
    pub(crate) fn incremental(records: Vec<SequencedJournalRecord>) -> Self {
        Self {
            kind: JournalCommitKind::Incremental,
            records,
        }
    }

    pub(crate) fn snapshot(records: Vec<SequencedJournalRecord>) -> Self {
        Self {
            kind: JournalCommitKind::Snapshot,
            records,
        }
    }

    pub(crate) const fn kind(&self) -> JournalCommitKind {
        self.kind
    }

    pub(crate) fn records(&self) -> &[SequencedJournalRecord] {
        &self.records
    }

    pub(crate) fn journal_cutoff(&self) -> Option<JournalSequence> {
        self.records.last().map(SequencedJournalRecord::sequence)
    }

    pub(crate) fn session_id(&self) -> Option<SessionId> {
        self.records
            .first()
            .map(|entry| entry.record().session_id())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SequencedJournalRecord {
    sequence: JournalSequence,
    record: JournalRecord,
}

impl SequencedJournalRecord {
    pub(crate) const fn new(sequence: JournalSequence, record: JournalRecord) -> Self {
        Self { sequence, record }
    }

    pub(crate) const fn sequence(&self) -> JournalSequence {
        self.sequence
    }

    pub(crate) const fn record(&self) -> &JournalRecord {
        &self.record
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum JournalRecord {
    CommandCommitted(AgentCommand),
    EventCommitted(AgentEvent),
    MessageSegment(MessageSegment),
    MessageEnded(MessageTerminal),
}

impl JournalRecord {
    pub(crate) const fn session_id(&self) -> SessionId {
        match self {
            Self::CommandCommitted(command) => match command {
                AgentCommand::CreateSession { session_id } => *session_id,
                AgentCommand::StartTurn { turn, .. }
                | AgentCommand::SteerTurn { turn, .. }
                | AgentCommand::InterruptTurn { turn } => turn.session_id(),
                AgentCommand::RespondToActivity { request, .. } => request.activity().session_id(),
            },
            Self::EventCommitted(event) => match event {
                AgentEvent::SessionCreated { session_id } => *session_id,
                AgentEvent::TurnStarted { turn } | AgentEvent::TurnFinished { turn, .. } => {
                    turn.session_id()
                },
                AgentEvent::ActivityStarted { activity, .. }
                | AgentEvent::ActivityUpdated { activity, .. }
                | AgentEvent::ActivityFinished { activity, .. } => activity.session_id(),
            },
            Self::MessageSegment(segment) => segment.activity().session_id(),
            Self::MessageEnded(terminal) => terminal.ended().activity().session_id(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MessageStream {
    Agent,
    ToolOutput,
}

impl MessageStream {
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
    index: u64,
    text: String,
}

impl MessageSegment {
    pub(crate) fn new(
        activity: ActivityRef,
        stream: MessageStream,
        index: u64,
        text: String,
    ) -> Self {
        Self {
            activity,
            stream,
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
    Failed(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MessageEnded {
    activity: ActivityRef,
    stream: MessageStream,
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
    pub(crate) fn new(
        activity: ActivityRef,
        stream: MessageStream,
        outcome: MessageOutcome,
        segment_count: u64,
        utf8_bytes: u64,
    ) -> Self {
        Self {
            activity,
            stream,
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
