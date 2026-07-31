use std::collections::{BTreeMap, BTreeSet};

use super::{
    JournalCodecError, JournalCommit, JournalCommitKind, JournalRecord, MessageEnded,
    MessageOutcome, MessageStream, SequencedJournalRecord,
};
use crate::{ActivityRef, JournalSequence};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecoveredJournal {
    records: Vec<SequencedJournalRecord>,
    recovery_commit: Option<JournalCommit>,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "runtime recovery publication is explicitly outside this Slice"
    )
)]
impl RecoveredJournal {
    pub(crate) fn records(&self) -> &[SequencedJournalRecord] {
        &self.records
    }

    pub(crate) const fn recovery_commit(&self) -> Option<&JournalCommit> {
        self.recovery_commit.as_ref()
    }

    pub(crate) fn complete_snapshot(&self) -> JournalCommit {
        let mut records = self.records.clone();
        if let Some(recovery_commit) = &self.recovery_commit {
            records.extend(recovery_commit.records().iter().cloned());
        }
        JournalCommit::snapshot(records)
    }
}

#[derive(Clone, Copy)]
struct OpenMessage {
    stream: MessageStream,
    segment_count: u64,
    utf8_bytes: u64,
}

pub(crate) fn recover(commits: &[JournalCommit]) -> Result<RecoveredJournal, JournalCodecError> {
    let mut records = Vec::new();
    let mut open_messages = BTreeMap::<ActivityRef, OpenMessage>::new();
    let mut ended_messages = BTreeSet::<ActivityRef>::new();
    let mut head = None;

    for (commit_index, commit) in commits.iter().enumerate() {
        let result = (|| {
            if commit.kind() == JournalCommitKind::Snapshot {
                let first = commit.records().first().ok_or_else(|| {
                    JournalCodecError::new("a recovery snapshot must contain Journal state")
                })?;
                if first.sequence().get() != 1 {
                    return Err(JournalCodecError::new(
                        "a complete Journal snapshot must begin at sequence 1",
                    ));
                }
                records.clear();
                open_messages.clear();
                ended_messages.clear();
                head = None;
            }

            for entry in commit.records() {
                let expected = head.map_or(1, |head: JournalSequence| {
                    head.get().checked_add(1).unwrap_or(0)
                });
                if entry.sequence().get() != expected {
                    return Err(JournalCodecError::new(format!(
                        "expected Journal sequence {expected}, found {}",
                        entry.sequence().get()
                    )));
                }
                apply_message_record(entry.record(), &mut open_messages, &mut ended_messages)?;
                records.push(entry.clone());
                head = Some(entry.sequence());
            }
            Ok(())
        })();
        result.map_err(|error: JournalCodecError| error.with_commit_index(commit_index))?;
    }

    let recovery_commit = recovery_seals(head, &open_messages)?;
    Ok(RecoveredJournal {
        records,
        recovery_commit,
    })
}

fn apply_message_record(
    record: &JournalRecord,
    open_messages: &mut BTreeMap<ActivityRef, OpenMessage>,
    ended_messages: &mut BTreeSet<ActivityRef>,
) -> Result<(), JournalCodecError> {
    if !open_messages.is_empty()
        && matches!(
            record,
            JournalRecord::CommandCommitted(_) | JournalRecord::EventCommitted(_)
        )
    {
        return Err(JournalCodecError::new(
            "an unterminated durable message must be sealed before a later durable event",
        ));
    }
    match record {
        JournalRecord::MessageSegment(segment) => {
            apply_segment(segment, open_messages, ended_messages)?;
        },
        JournalRecord::MessageEnded(terminal) => {
            if let Some(final_segment) = terminal.final_segment() {
                apply_segment(final_segment, open_messages, ended_messages)?;
            }
            let ended = terminal.ended();
            if !ended_messages.insert(ended.activity()) {
                return Err(JournalCodecError::new(
                    "a message cannot have more than one terminal seal",
                ));
            }
            let observed = open_messages
                .remove(&ended.activity())
                .unwrap_or(OpenMessage {
                    stream: ended.stream(),
                    segment_count: 0,
                    utf8_bytes: 0,
                });
            if observed.stream != ended.stream()
                || observed.segment_count != ended.segment_count()
                || observed.utf8_bytes != ended.utf8_bytes()
            {
                return Err(JournalCodecError::new(
                    "MessageEnded does not match its durable segments",
                ));
            }
        },
        JournalRecord::CommandCommitted(_) | JournalRecord::EventCommitted(_) => {},
    }
    Ok(())
}

fn apply_segment(
    segment: &super::MessageSegment,
    open_messages: &mut BTreeMap<ActivityRef, OpenMessage>,
    ended_messages: &BTreeSet<ActivityRef>,
) -> Result<(), JournalCodecError> {
    if ended_messages.contains(&segment.activity()) {
        return Err(JournalCodecError::new(
            "a terminated message cannot accept another segment",
        ));
    }
    let state = open_messages
        .entry(segment.activity())
        .or_insert(OpenMessage {
            stream: segment.stream(),
            segment_count: 0,
            utf8_bytes: 0,
        });
    if state.stream != segment.stream() {
        return Err(JournalCodecError::new(
            "one message cannot change its stream kind",
        ));
    }
    let expected_index = state
        .segment_count
        .checked_add(1)
        .ok_or_else(|| JournalCodecError::new("Message segment count is exhausted"))?;
    if segment.index() != expected_index {
        return Err(JournalCodecError::new(format!(
            "expected MessageSegment index {expected_index}, found {}",
            segment.index()
        )));
    }
    state.segment_count = expected_index;
    state.utf8_bytes = state
        .utf8_bytes
        .checked_add(
            u64::try_from(segment.text().len())
                .map_err(|_| JournalCodecError::new("MessageSegment byte length exceeds u64"))?,
        )
        .ok_or_else(|| JournalCodecError::new("message byte count is exhausted"))?;
    Ok(())
}

fn recovery_seals(
    head: Option<JournalSequence>,
    open_messages: &BTreeMap<ActivityRef, OpenMessage>,
) -> Result<Option<JournalCommit>, JournalCodecError> {
    if open_messages.is_empty() {
        return Ok(None);
    }
    let mut next = head
        .map_or(1, JournalSequence::get)
        .checked_add(u64::from(head.is_some()))
        .ok_or_else(|| JournalCodecError::new("Journal sequence is exhausted"))?;
    let mut records = Vec::with_capacity(open_messages.len());
    for (activity, state) in open_messages {
        records.push(SequencedJournalRecord::new(
            JournalSequence::new(next),
            JournalRecord::MessageEnded(super::MessageTerminal::new(
                None,
                MessageEnded::new(
                    *activity,
                    state.stream,
                    MessageOutcome::Interrupted,
                    state.segment_count,
                    state.utf8_bytes,
                ),
            )),
        ));
        next = next
            .checked_add(1)
            .ok_or_else(|| JournalCodecError::new("Journal sequence is exhausted"))?;
    }
    Ok(Some(JournalCommit::incremental(records)))
}
