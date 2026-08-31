mod command;
mod correlation;
mod descriptor;
mod event;
mod identity;
mod input;
mod message;
mod record;

use std::fmt;

use record::WireRecord;
use serde::{Deserialize, Serialize};

use super::{
    BindingTransition, CacheState, JournalCommit, JournalCommitKind, JournalRecord, MessageSegment,
    ReplaySequence, SequencedJournalRecord, TransitionMode, recover,
};
use crate::{AgentEvent, JournalSequence};

const SCHEMA: &str = "yo.semantic-journal-commit/v1";
const FORMAT: &str = "anchored-session";

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
    format: String,
    kind: WireCommitKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    journal_cutoff: Option<u64>,
    first_sequence: u64,
    records: Vec<WireRecord>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum WireCommitKind {
    Incremental,
    Snapshot,
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
        format: FORMAT.to_owned(),
        kind: match commit.kind() {
            JournalCommitKind::Incremental => WireCommitKind::Incremental,
            JournalCommitKind::Snapshot => WireCommitKind::Snapshot,
        },
        journal_cutoff: commit.journal_cutoff().map(JournalSequence::get),
        first_sequence,
        records: commit
            .records()
            .iter()
            .map(WireRecord::try_from)
            .collect::<Result<Vec<_>, _>>()?,
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
    if wire.format != FORMAT {
        return Err(JournalCodecError::new(format!(
            "unsupported Journal commit format {:?}",
            wire.format
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
            let (journal_sequence, record) = record.try_into()?;
            Ok(SequencedJournalRecord::decoded(
                ReplaySequence::new(sequence),
                journal_sequence,
                record,
            ))
        })
        .collect::<Result<Vec<_>, JournalCodecError>>()?;
    let journal_cutoff = wire.journal_cutoff.map(JournalSequence::new);
    let kind = match wire.kind {
        WireCommitKind::Incremental => JournalCommitKind::Incremental,
        WireCommitKind::Snapshot => JournalCommitKind::Snapshot,
    };
    let commit = JournalCommit::decoded(kind, journal_cutoff, records);
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
    if let Some(cutoff) = commit.journal_cutoff() {
        if cutoff.get() == 0 {
            return Err(JournalCodecError::new(
                "Journal cutoff must be a positive semantic sequence",
            ));
        }
    } else if commit.kind() != JournalCommitKind::Incremental
        || commit.records().len() != 1
        || !matches!(first.record(), JournalRecord::SessionDescriptor(_))
        || first.sequence().get() != 1
    {
        return Err(JournalCodecError::new(
            "only the initial descriptor commit may omit its semantic cutoff",
        ));
    }
    if commit.kind() == JournalCommitKind::Snapshot && first.sequence().get() != 1 {
        return Err(JournalCodecError::new(
            "a complete Journal snapshot must begin at sequence 1",
        ));
    }
    let session_id = commit
        .records()
        .iter()
        .find_map(|entry| entry.record().session_id());
    if commit
        .records()
        .iter()
        .filter_map(|entry| entry.record().session_id())
        .any(|candidate| Some(candidate) != session_id)
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
                "records inside one semantic commit must have contiguous ReplaySequences",
            ));
        }
    }
    let mut previous_journal_sequence = None;
    for entry in commit.records() {
        match (
            entry.record().requires_journal_sequence(),
            entry.journal_sequence(),
        ) {
            (true, Some(sequence)) => {
                if sequence.get() == 0 {
                    return Err(JournalCodecError::new("journal_sequence must be positive"));
                }
                if previous_journal_sequence
                    .is_some_and(|previous: JournalSequence| sequence <= previous)
                {
                    return Err(JournalCodecError::new(
                        "semantic journal_sequence values must be strictly increasing",
                    ));
                }
                if commit
                    .journal_cutoff()
                    .is_some_and(|cutoff| sequence > cutoff)
                {
                    return Err(JournalCodecError::new(
                        "semantic journal_sequence cannot exceed journal_cutoff",
                    ));
                }
                previous_journal_sequence = Some(sequence);
            },
            (true, None) => {
                return Err(JournalCodecError::new(
                    "semantic Journal record is missing journal_sequence",
                ));
            },
            (false, Some(_)) => {
                return Err(JournalCodecError::new(
                    "storage-only Journal record cannot contain journal_sequence",
                ));
            },
            (false, None) => {},
        }
    }
    validate_correlation_commit_order(commit)?;
    let descriptor_count = commit
        .records()
        .iter()
        .filter(|entry| matches!(entry.record(), JournalRecord::SessionDescriptor(_)))
        .count();
    if descriptor_count > 0
        && (descriptor_count != 1
            || first.sequence().get() != 1
            || !matches!(first.record(), JournalRecord::SessionDescriptor(_)))
    {
        return Err(JournalCodecError::new(
            "a Session descriptor must be the single replay-sequence-one prefix record",
        ));
    }
    for entry in commit.records() {
        match entry.record() {
            JournalRecord::SessionDescriptor(_) => {},
            JournalRecord::MessageReset(reset) => {
                if reset.revision() == 0 {
                    return Err(JournalCodecError::new(
                        "MessageReset revision must be positive",
                    ));
                }
            },
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
            JournalRecord::CommandCommitted(_) => {},
            JournalRecord::EventCommitted(_) => {},
            JournalRecord::BackendExchangeObserved(exchange) => {
                correlation::positive(exchange.epoch(), "epoch")?;
                correlation::validate_ascii(exchange.payload_schema(), "payload_schema")?;
                if let Some(identity) = exchange.exchange_identity() {
                    correlation::encode_identity(identity)?;
                }
            },
            JournalRecord::BackendBindingOpened(binding) => {
                correlation::positive(binding.epoch(), "epoch")?;
                correlation::validate_ascii(binding.backend_kind(), "backend_kind")?;
                correlation::validate_value(binding.backend_version(), "backend_version")?;
                correlation::encode_identity(binding.binding_identity())?;
                correlation::encode_identity(binding.model_identity())?;
                correlation::encode_identity(binding.session_locator())?;
                validate_transition(binding.transition())?;
            },
            JournalRecord::BackendBindingClosed(binding) => {
                correlation::positive(binding.epoch(), "epoch")?;
            },
            JournalRecord::BackendRequestAccepted(request) => {
                correlation::positive(request.epoch(), "epoch")?;
                if let Some(context_epoch) = request.context_epoch() {
                    correlation::positive(context_epoch, "context_epoch")?;
                }
                correlation::encode_identity(request.request_identity())?;
            },
            JournalRecord::ModelReplayDelta(replay) => {
                correlation::positive(replay.epoch(), "epoch")?;
                if let Some(context_epoch) = replay.context_epoch() {
                    correlation::positive(context_epoch, "context_epoch")?;
                }
                if !replay.delta().is_valid() {
                    return Err(JournalCodecError::new(
                        "model_replay_delta is invalid or exceeds its bounds",
                    ));
                }
            },
            JournalRecord::BackendResumableOutcome(outcome) => {
                correlation::positive(outcome.epoch(), "epoch")?;
                if let Some(context_epoch) = outcome.context_epoch() {
                    correlation::positive(context_epoch, "context_epoch")?;
                }
                if let Some(identity) = outcome.outcome_identity() {
                    correlation::encode_identity(identity)?;
                }
            },
            JournalRecord::ContinuationAnchor(anchor) => {
                correlation::positive(anchor.epoch(), "epoch")?;
                if let Some(context_epoch) = anchor.context_epoch() {
                    correlation::positive(context_epoch, "context_epoch")?;
                }
            },
            JournalRecord::ContextPolicyChanged(_) | JournalRecord::ContextCheckpoint(_) => {},
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

fn validate_transition(transition: &BindingTransition) -> Result<(), JournalCodecError> {
    let valid = matches!(
        (
            transition.mode(),
            transition.cache(),
            transition.source_anchor_sequence(),
            transition.source_checkpoint_sequence(),
        ),
        (
            TransitionMode::Initial,
            CacheState::NotApplicable,
            None,
            None
        ) | (TransitionMode::ExactReplay, CacheState::Lost, Some(_), None)
            | (TransitionMode::ExactReplay, CacheState::Lost, None, Some(_))
            | (
                TransitionMode::LossyHandoff,
                CacheState::Lost | CacheState::Unknown,
                Some(_),
                None,
            )
    );
    if valid {
        Ok(())
    } else {
        Err(JournalCodecError::new(
            "binding transition mode, cache, and source are inconsistent",
        ))
    }
}

fn validate_correlation_commit_order(commit: &JournalCommit) -> Result<(), JournalCodecError> {
    for (index, entry) in commit.records().iter().enumerate() {
        if let JournalRecord::ModelReplayDelta(replay) = entry.record() {
            let Some((previous, next)) = index
                .checked_sub(1)
                .and_then(|previous| commit.records().get(previous))
                .zip(commit.records().get(index + 1))
            else {
                return Err(JournalCodecError::new(
                    "model_replay_delta must sit between its completed Turn and resumable outcome",
                ));
            };
            if !matches!(
                previous.record(),
                JournalRecord::EventCommitted(AgentEvent::TurnFinished { turn, outcome: crate::TurnOutcome::Completed })
                    if turn.turn_id() == replay.turn_id()
            ) {
                return Err(JournalCodecError::new(
                    "model_replay_delta must immediately follow its completed Turn",
                ));
            }
            let Some(replay_sequence) = entry.journal_sequence() else {
                return Err(JournalCodecError::new(
                    "model_replay_delta is missing journal_sequence",
                ));
            };
            if !matches!(
                next.record(),
                JournalRecord::BackendResumableOutcome(outcome)
                    if outcome.replay_delta_sequence() == Some(replay_sequence)
                        && outcome.epoch() == replay.epoch()
                        && outcome.turn_id() == replay.turn_id()
                        && outcome.accepted_request_sequence() == replay.accepted_request_sequence()
            ) {
                return Err(JournalCodecError::new(
                    "model_replay_delta must be referenced by its immediately following outcome",
                ));
            }
        }
        if let JournalRecord::BackendResumableOutcome(outcome) = entry.record() {
            let completed_in_commit = commit.records()[..index].iter().any(|candidate| {
                matches!(
                    candidate.record(),
                    JournalRecord::EventCommitted(AgentEvent::TurnFinished { turn, outcome: crate::TurnOutcome::Completed })
                        if turn.turn_id() == outcome.turn_id()
                )
            });
            if !completed_in_commit {
                return Err(JournalCodecError::new(
                    "backend_resumable_outcome requires its completed Turn in the same commit",
                ));
            }
            let valid_predecessor = match outcome.replay_delta_sequence() {
                Some(sequence) => index.checked_sub(1).is_some_and(|previous| {
                    let previous = &commit.records()[previous];
                    previous.journal_sequence() == Some(sequence)
                        && matches!(previous.record(), JournalRecord::ModelReplayDelta(_))
                }),
                None => index.checked_sub(1).is_some_and(|previous| {
                    matches!(
                        commit.records()[previous].record(),
                        JournalRecord::EventCommitted(AgentEvent::TurnFinished { turn, outcome: crate::TurnOutcome::Completed })
                            if turn.turn_id() == outcome.turn_id()
                    )
                }),
            };
            if !valid_predecessor {
                return Err(JournalCodecError::new(
                    "backend_resumable_outcome predecessor does not match its replay strategy evidence",
                ));
            }
            let Some(next) = commit.records().get(index + 1) else {
                return Err(JournalCodecError::new(
                    "backend_resumable_outcome must be followed immediately by continuation_anchor",
                ));
            };
            let Some(outcome_sequence) = entry.journal_sequence() else {
                return Err(JournalCodecError::new(
                    "backend_resumable_outcome is missing journal_sequence",
                ));
            };
            let JournalRecord::ContinuationAnchor(anchor) = next.record() else {
                return Err(JournalCodecError::new(
                    "backend_resumable_outcome must be followed immediately by continuation_anchor",
                ));
            };
            if anchor.resumable_outcome_sequence() != outcome_sequence
                || anchor.journal_boundary() != outcome_sequence
            {
                return Err(JournalCodecError::new(
                    "continuation_anchor boundary must identify its immediately preceding outcome",
                ));
            }
        }
        if matches!(entry.record(), JournalRecord::ContinuationAnchor(_))
            && (index == 0
                || !matches!(
                    commit.records()[index - 1].record(),
                    JournalRecord::BackendResumableOutcome(_)
                ))
        {
            return Err(JournalCodecError::new(
                "continuation_anchor must immediately follow backend_resumable_outcome",
            ));
        }
    }
    Ok(())
}
