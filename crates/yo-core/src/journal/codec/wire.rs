mod command;
mod event;
mod identity;
mod message;

use std::fmt;

use command::WireCommand;
use event::WireEvent;
use message::{WireMessageEnded, WireMessageSegment};
use serde::{Deserialize, Serialize};

use super::{
    JournalCommit, JournalCommitKind, JournalRecord, MessageEnded, MessageSegment, MessageTerminal,
    SequencedJournalRecord, recover,
};
use crate::{AgentCommand, AgentEvent, JournalSequence};

const SCHEMA: &str = "yo.semantic-journal-commit/v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct JournalCodecError {
    message: String,
    commit_index: Option<usize>,
}

impl JournalCodecError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            commit_index: None,
        }
    }

    pub(crate) fn with_commit_index(mut self, commit_index: usize) -> Self {
        self.commit_index = Some(commit_index);
        self
    }

    pub(crate) const fn commit_index(&self) -> Option<usize> {
        self.commit_index
    }

    pub(crate) fn context(self, context: impl fmt::Display) -> Self {
        Self {
            message: format!("{context}: {}", self.message),
            commit_index: self.commit_index,
        }
    }
}

impl fmt::Display for JournalCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for JournalCodecError {}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireCommit {
    schema: String,
    kind: WireCommitKind,
    first_sequence: u64,
    records: Vec<WireRecord>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum WireCommitKind {
    Incremental,
    Snapshot,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireRecord {
    CommandCommitted {
        command: WireCommand,
    },
    EventCommitted {
        event: WireEvent,
    },
    MessageSegment {
        segment: WireMessageSegment,
    },
    MessageEnded {
        #[serde(skip_serializing_if = "Option::is_none")]
        final_segment: Option<WireMessageSegment>,
        ended: WireMessageEnded,
    },
}

pub(crate) fn encode(commit: &JournalCommit) -> Result<String, JournalCodecError> {
    validate_commit(commit)?;
    let first_sequence = commit
        .records()
        .first()
        .expect("validation rejects an empty commit")
        .sequence()
        .get();
    let wire = WireCommit {
        schema: SCHEMA.to_owned(),
        kind: match commit.kind() {
            JournalCommitKind::Incremental => WireCommitKind::Incremental,
            JournalCommitKind::Snapshot => WireCommitKind::Snapshot,
        },
        first_sequence,
        records: commit
            .records()
            .iter()
            .map(|record| WireRecord::from(record.record()))
            .collect(),
    };
    serde_json::to_string(&wire).map_err(|error| {
        JournalCodecError::new(format!("failed to encode Journal commit: {error}"))
    })
}

pub(crate) fn decode(payload: &str) -> Result<JournalCommit, JournalCodecError> {
    let wire: WireCommit = serde_json::from_str(payload)
        .map_err(|error| JournalCodecError::new(format!("invalid Journal commit JSON: {error}")))?;
    if wire.schema != SCHEMA {
        return Err(JournalCodecError::new(format!(
            "unsupported Journal commit schema {:?}",
            wire.schema
        )));
    }
    let records = wire
        .records
        .into_iter()
        .enumerate()
        .map(|(offset, record)| {
            let offset = u64::try_from(offset)
                .map_err(|_| JournalCodecError::new("Journal commit has too many records"))?;
            let sequence = wire
                .first_sequence
                .checked_add(offset)
                .ok_or_else(|| JournalCodecError::new("Journal sequence is exhausted"))?;
            Ok(SequencedJournalRecord::new(
                JournalSequence::new(sequence),
                JournalRecord::try_from(record)?,
            ))
        })
        .collect::<Result<Vec<_>, JournalCodecError>>()?;
    let commit = match wire.kind {
        WireCommitKind::Incremental => JournalCommit::incremental(records),
        WireCommitKind::Snapshot => JournalCommit::snapshot(records),
    };
    validate_commit(&commit)?;
    Ok(commit)
}

fn validate_commit(commit: &JournalCommit) -> Result<(), JournalCodecError> {
    let Some(first) = commit.records().first() else {
        return Err(JournalCodecError::new(
            "a semantic commit must contain at least one Journal record",
        ));
    };
    if first.sequence().get() == 0 {
        return Err(JournalCodecError::new(
            "Journal sequence must start with a positive value",
        ));
    }
    if commit.kind() == JournalCommitKind::Snapshot && first.sequence().get() != 1 {
        return Err(JournalCodecError::new(
            "a complete Journal snapshot must begin at sequence 1",
        ));
    }
    let session_id = first.record().session_id();
    if commit
        .records()
        .iter()
        .any(|entry| entry.record().session_id() != session_id)
    {
        return Err(JournalCodecError::new(
            "one semantic commit cannot contain records from different Sessions",
        ));
    }
    for pair in commit.records().windows(2) {
        let expected = pair[0]
            .sequence()
            .get()
            .checked_add(1)
            .ok_or_else(|| JournalCodecError::new("Journal sequence is exhausted"))?;
        if pair[1].sequence().get() != expected {
            return Err(JournalCodecError::new(
                "records inside one semantic commit must have contiguous Journal sequences",
            ));
        }
    }
    for entry in commit.records() {
        match entry.record() {
            JournalRecord::MessageSegment(segment) => validate_segment(segment)?,
            JournalRecord::MessageEnded(terminal) => {
                let ended = terminal.ended();
                if let Some(final_segment) = terminal.final_segment() {
                    validate_segment(final_segment)?;
                    let final_bytes = u64::try_from(final_segment.text().len()).map_err(|_| {
                        JournalCodecError::new("MessageSegment byte length exceeds u64")
                    })?;
                    if final_segment.activity() != ended.activity()
                        || final_segment.stream() != ended.stream()
                        || final_segment.index() != ended.segment_count()
                        || final_bytes > ended.utf8_bytes()
                    {
                        return Err(JournalCodecError::new(
                            "MessageEnded does not atomically describe its final tail",
                        ));
                    }
                } else if ended.segment_count() == 0 && ended.utf8_bytes() != 0 {
                    return Err(JournalCodecError::new(
                        "an empty message cannot report durable UTF-8 bytes",
                    ));
                }
            },
            JournalRecord::EventCommitted(AgentEvent::ActivityUpdated { .. }) => {
                return Err(JournalCodecError::new(
                    "durable text updates must be encoded as bounded MessageSegments",
                ));
            },
            JournalRecord::CommandCommitted(_) | JournalRecord::EventCommitted(_) => {},
        }
    }
    if commit.kind() == JournalCommitKind::Snapshot {
        let recovered = recover(std::slice::from_ref(commit))?;
        if recovered.recovery_commit().is_some() {
            return Err(JournalCodecError::new(
                "a complete Journal snapshot cannot require recovery repair",
            ));
        }
    }
    if commit.kind() == JournalCommitKind::Incremental {
        let command_positions = commit
            .records()
            .iter()
            .enumerate()
            .filter_map(|(index, record)| {
                matches!(record.record(), JournalRecord::CommandCommitted(_)).then_some(index)
            })
            .collect::<Vec<_>>();
        if command_positions.len() > 1 || command_positions.first().is_some_and(|index| *index != 0)
        {
            return Err(JournalCodecError::new(
                "an incremental command commit must contain one leading command at most",
            ));
        }
    }
    Ok(())
}

fn validate_segment(segment: &MessageSegment) -> Result<(), JournalCodecError> {
    if segment.index() == 0 || segment.text().is_empty() {
        return Err(JournalCodecError::new(
            "MessageSegment index and text must be non-empty",
        ));
    }
    if segment.text().len() > segment.stream().segment_limit() {
        return Err(JournalCodecError::new(
            "MessageSegment exceeds its UTF-8 byte bound",
        ));
    }
    Ok(())
}

impl From<&JournalRecord> for WireRecord {
    fn from(record: &JournalRecord) -> Self {
        match record {
            JournalRecord::CommandCommitted(command) => Self::CommandCommitted {
                command: WireCommand::from(command),
            },
            JournalRecord::EventCommitted(event) => Self::EventCommitted {
                event: WireEvent::from(event),
            },
            JournalRecord::MessageSegment(segment) => Self::MessageSegment {
                segment: WireMessageSegment::from(segment),
            },
            JournalRecord::MessageEnded(terminal) => Self::MessageEnded {
                final_segment: terminal.final_segment().map(WireMessageSegment::from),
                ended: WireMessageEnded::from(terminal.ended()),
            },
        }
    }
}

impl TryFrom<WireRecord> for JournalRecord {
    type Error = JournalCodecError;

    fn try_from(record: WireRecord) -> Result<Self, Self::Error> {
        match record {
            WireRecord::CommandCommitted { command } => {
                Ok(Self::CommandCommitted(AgentCommand::try_from(command)?))
            },
            WireRecord::EventCommitted { event } => {
                Ok(Self::EventCommitted(AgentEvent::try_from(event)?))
            },
            WireRecord::MessageSegment { segment } => {
                Ok(Self::MessageSegment(MessageSegment::try_from(segment)?))
            },
            WireRecord::MessageEnded {
                final_segment,
                ended,
            } => Ok(Self::MessageEnded(MessageTerminal::new(
                final_segment.map(MessageSegment::try_from).transpose()?,
                MessageEnded::try_from(ended)?,
            ))),
        }
    }
}
