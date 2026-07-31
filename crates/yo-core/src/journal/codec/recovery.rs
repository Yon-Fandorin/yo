use std::collections::{BTreeMap, BTreeSet};

use super::{
    JournalCodecError, JournalCommit, JournalCommitFormat, JournalCommitKind, JournalRecord,
    MessageEnded, MessageOutcome, MessageStream, ReplaySequence, SequencedJournalRecord,
};
use crate::{ActivityRef, AgentEvent, JournalSequence};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecoveredJournal {
    records: Vec<SequencedJournalRecord>,
    journal_cutoff: JournalSequence,
    recovery_commit: Option<JournalCommit>,
    open_messages: BTreeMap<ActivityRef, OpenMessage>,
    ended_messages: BTreeSet<ActivityRef>,
    head: Option<ReplaySequence>,
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
        JournalCommit::snapshot_through(self.journal_cutoff(), records)
    }

    pub(crate) fn journal_cutoff(&self) -> JournalSequence {
        self.journal_cutoff
    }

    pub(crate) fn validate_incremental(
        &self,
        commit: &JournalCommit,
    ) -> Result<(), JournalCodecError> {
        if commit.kind() != JournalCommitKind::Incremental {
            return Err(JournalCodecError::new(
                "incremental recovery cannot apply a snapshot",
            ));
        }
        if commit.semantic_cutoff() < self.journal_cutoff {
            return Err(JournalCodecError::new(
                "semantic Journal cutoff moved backwards",
            ));
        }
        let mut open_messages = self.open_messages.clone();
        let mut ended_messages = self.ended_messages.clone();
        let mut head = self.head;
        for entry in commit.records() {
            let expected = head.map_or(1, |value| value.get().checked_add(1).unwrap_or(0));
            if entry.sequence().get() != expected {
                return Err(JournalCodecError::new(format!(
                    "expected replay sequence {expected}, found {}",
                    entry.sequence().get()
                )));
            }
            apply_message_record(
                entry.record(),
                commit.format(),
                &mut open_messages,
                &mut ended_messages,
            )?;
            head = Some(entry.sequence());
        }
        recovery_seals(head, commit.semantic_cutoff(), &open_messages)?;
        Ok(())
    }

    pub(crate) fn append_validated(&mut self, commit: &JournalCommit) {
        self.recovery_commit = None;
        apply_commit(self, commit).expect("a prevalidated incremental commit remains valid");
        self.recovery_commit = recovery_seals(self.head, self.journal_cutoff, &self.open_messages)
            .expect("a prevalidated recovery seal remains valid");
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OpenMessage {
    stream: MessageStream,
    revision: u64,
    segment_count: u64,
    utf8_bytes: u64,
}

pub(crate) fn recover(commits: &[JournalCommit]) -> Result<RecoveredJournal, JournalCodecError> {
    let first = commits
        .first()
        .ok_or_else(|| JournalCodecError::new("Journal recovery requires a semantic commit"))?;
    let mut recovered = RecoveredJournal {
        records: Vec::new(),
        journal_cutoff: first.semantic_cutoff(),
        recovery_commit: None,
        open_messages: BTreeMap::new(),
        ended_messages: BTreeSet::new(),
        head: None,
    };

    for (commit_index, commit) in commits.iter().enumerate() {
        apply_commit(&mut recovered, commit)
            .map_err(|error| error.with_commit_index(commit_index))?;
    }

    recovered.recovery_commit = recovery_seals(
        recovered.head,
        recovered.journal_cutoff,
        &recovered.open_messages,
    )?;
    Ok(recovered)
}

fn apply_commit(
    recovered: &mut RecoveredJournal,
    commit: &JournalCommit,
) -> Result<(), JournalCodecError> {
    if !recovered.records.is_empty() && commit.semantic_cutoff() < recovered.journal_cutoff {
        return Err(JournalCodecError::new(
            "semantic Journal cutoff moved backwards",
        ));
    }
    if commit.kind() == JournalCommitKind::Snapshot {
        let first = commit.records().first().ok_or_else(|| {
            JournalCodecError::new("a recovery snapshot must contain Journal state")
        })?;
        if first.sequence().get() != 1 {
            return Err(JournalCodecError::new(
                "a complete Journal snapshot must begin at sequence 1",
            ));
        }
        recovered.records.clear();
        recovered.open_messages.clear();
        recovered.ended_messages.clear();
        recovered.head = None;
    }
    recovered.journal_cutoff = commit.semantic_cutoff();
    for entry in commit.records() {
        let expected = recovered
            .head
            .map_or(1, |head| head.get().checked_add(1).unwrap_or(0));
        if entry.sequence().get() != expected {
            return Err(JournalCodecError::new(format!(
                "expected replay sequence {expected}, found {}",
                entry.sequence().get()
            )));
        }
        apply_message_record(
            entry.record(),
            commit.format(),
            &mut recovered.open_messages,
            &mut recovered.ended_messages,
        )?;
        recovered.records.push(entry.clone());
        recovered.head = Some(entry.sequence());
    }
    Ok(())
}

fn apply_message_record(
    record: &JournalRecord,
    format: JournalCommitFormat,
    open_messages: &mut BTreeMap<ActivityRef, OpenMessage>,
    ended_messages: &mut BTreeSet<ActivityRef>,
) -> Result<(), JournalCodecError> {
    if format == JournalCommitFormat::LegacyV1
        && !open_messages.is_empty()
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
        JournalRecord::MessageReset(reset) => {
            if ended_messages.contains(&reset.activity()) {
                return Err(JournalCodecError::new(
                    "a terminated message cannot start another revision",
                ));
            }
            let state = open_messages.get_mut(&reset.activity()).ok_or_else(|| {
                JournalCodecError::new("MessageReset requires a started message activity")
            })?;
            if state.stream != reset.stream()
                || reset.revision() != state.revision.saturating_add(1)
            {
                return Err(JournalCodecError::new(
                    "MessageReset does not start the next revision",
                ));
            }
            state.revision = reset.revision();
            state.segment_count = 0;
            state.utf8_bytes = 0;
        },
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
            let mut observed = open_messages
                .remove(&ended.activity())
                .unwrap_or(OpenMessage {
                    stream: ended.stream(),
                    revision: ended.revision(),
                    segment_count: 0,
                    utf8_bytes: 0,
                });
            if terminal.final_segment().is_none()
                && ended.revision() == observed.revision.saturating_add(1)
                && ended.segment_count() == 0
                && ended.utf8_bytes() == 0
            {
                // An authoritative empty snapshot has no segment with which to announce its new
                // revision. Its zero-byte terminal is the complete durable representation.
                observed.revision = ended.revision();
                observed.segment_count = 0;
                observed.utf8_bytes = 0;
            }
            if observed.stream != ended.stream()
                || observed.revision != ended.revision()
                || observed.segment_count != ended.segment_count()
                || observed.utf8_bytes != ended.utf8_bytes()
            {
                return Err(JournalCodecError::new(
                    "MessageEnded does not match its durable segments",
                ));
            }
        },
        JournalRecord::EventCommitted(AgentEvent::ActivityStarted { activity, kind })
            if format == JournalCommitFormat::Current =>
        {
            if ended_messages.contains(activity)
                || open_messages
                    .insert(
                        *activity,
                        OpenMessage {
                            stream: MessageStream::for_activity(*kind),
                            revision: 1,
                            segment_count: 0,
                            utf8_bytes: 0,
                        },
                    )
                    .is_some()
            {
                return Err(JournalCodecError::new(
                    "a message activity cannot start more than once",
                ));
            }
        },
        JournalRecord::EventCommitted(AgentEvent::ActivityFinished { activity, .. })
            if format == JournalCommitFormat::Current =>
        {
            if open_messages.contains_key(activity) {
                return Err(JournalCodecError::new(
                    "a finished message activity requires a preceding MessageEnded record",
                ));
            }
            if !ended_messages.contains(activity) {
                return Err(JournalCodecError::new(
                    "a finished message activity has no durable message lifecycle",
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
            revision: segment.revision(),
            segment_count: 0,
            utf8_bytes: 0,
        });
    if state.stream != segment.stream() {
        return Err(JournalCodecError::new(
            "one message cannot change its stream kind",
        ));
    }
    if segment.revision() < state.revision || segment.revision() > state.revision.saturating_add(1)
    {
        return Err(JournalCodecError::new(
            "MessageSegment revision is not contiguous",
        ));
    }
    if segment.revision() > state.revision {
        if segment.index() != 1 {
            return Err(JournalCodecError::new(
                "a replacement revision must begin with MessageSegment index 1",
            ));
        }
        state.revision = segment.revision();
        state.segment_count = 0;
        state.utf8_bytes = 0;
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
    head: Option<ReplaySequence>,
    journal_cutoff: JournalSequence,
    open_messages: &BTreeMap<ActivityRef, OpenMessage>,
) -> Result<Option<JournalCommit>, JournalCodecError> {
    if open_messages.is_empty() {
        return Ok(None);
    }
    let mut next = head
        .map_or(1, ReplaySequence::get)
        .checked_add(u64::from(head.is_some()))
        .ok_or_else(|| JournalCodecError::new("Journal sequence is exhausted"))?;
    let mut records = Vec::with_capacity(open_messages.len());
    for (activity, state) in open_messages {
        records.push(SequencedJournalRecord::new(
            ReplaySequence::new(next),
            JournalRecord::MessageEnded(super::MessageTerminal::new(
                None,
                MessageEnded::for_revision(
                    *activity,
                    state.stream,
                    state.revision,
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
    Ok(Some(JournalCommit::incremental_through(
        journal_cutoff,
        records,
    )))
}
