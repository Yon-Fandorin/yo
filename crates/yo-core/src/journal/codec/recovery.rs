use std::collections::{BTreeMap, BTreeSet};

mod correlation;

use correlation::CorrelationRecovery;

use super::{
    JournalCodecError, JournalCommit, JournalCommitKind, JournalRecord, MessageEnded,
    MessageOutcome, MessageStream, ReplaySequence, SequencedJournalRecord,
};
use crate::{ActivityRef, AgentEvent, JournalSequence, SessionDescriptor, SubmissionId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecoveredJournal {
    records: Vec<SequencedJournalRecord>,
    journal_cutoff: Option<JournalSequence>,
    descriptor: Option<SessionDescriptor>,
    recovery_commit: Option<JournalCommit>,
    open_messages: BTreeMap<ActivityRef, OpenMessage>,
    ended_messages: BTreeSet<ActivityRef>,
    submission_ids: BTreeSet<SubmissionId>,
    head: Option<ReplaySequence>,
    correlation: CorrelationRecovery,
    discovery_states: Vec<RecoveredDiscovery>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RecoveredDiscovery {
    binding_epoch: Option<u64>,
    continuation_anchor: Option<JournalSequence>,
}

impl RecoveredDiscovery {
    pub(crate) const fn binding_epoch(self) -> Option<u64> {
        self.binding_epoch
    }

    pub(crate) const fn continuation_anchor(self) -> Option<JournalSequence> {
        self.continuation_anchor
    }
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
        match self.journal_cutoff {
            Some(cutoff) => JournalCommit::snapshot_through(cutoff, records),
            None => JournalCommit::descriptor(
                self.descriptor
                    .clone()
                    .expect("a cutoff-less recovered Journal contains its descriptor"),
            ),
        }
    }

    #[allow(
        dead_code,
        reason = "stored Session discovery consumes recovered descriptor metadata in its follow-up Slice"
    )]
    pub(crate) fn journal_cutoff(&self) -> Option<JournalSequence> {
        self.journal_cutoff
    }

    pub(crate) const fn descriptor(&self) -> Option<&SessionDescriptor> {
        self.descriptor.as_ref()
    }

    pub(crate) const fn binding_epoch(&self) -> Option<u64> {
        self.correlation.open_epoch()
    }

    pub(crate) const fn continuation_anchor(&self) -> Option<JournalSequence> {
        self.correlation.latest_anchor()
    }

    pub(crate) fn discovery_states(&self) -> &[RecoveredDiscovery] {
        &self.discovery_states
    }

    pub(crate) fn with_incremental(
        &self,
        commit: &JournalCommit,
    ) -> Result<Self, JournalCodecError> {
        if commit.kind() != JournalCommitKind::Incremental {
            return Err(JournalCodecError::new(
                "incremental recovery cannot apply a snapshot",
            ));
        }
        if let (Some(next), Some(current)) = (commit.journal_cutoff(), self.journal_cutoff)
            && next < current
        {
            return Err(JournalCodecError::new(
                "semantic Journal cutoff moved backwards",
            ));
        }
        let mut candidate = self.clone();
        candidate.recovery_commit = None;
        apply_commit(&mut candidate, commit)?;
        candidate.recovery_commit = recovery_seals(
            candidate.head,
            candidate.journal_cutoff,
            &candidate.open_messages,
        )?;
        Ok(candidate)
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
    commits
        .first()
        .ok_or_else(|| JournalCodecError::new("Journal recovery requires a semantic commit"))?;
    let mut recovered = RecoveredJournal {
        records: Vec::new(),
        journal_cutoff: None,
        descriptor: None,
        recovery_commit: None,
        open_messages: BTreeMap::new(),
        ended_messages: BTreeSet::new(),
        submission_ids: BTreeSet::new(),
        head: None,
        correlation: CorrelationRecovery::default(),
        discovery_states: Vec::new(),
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
    let preceding_cutoff = recovered.journal_cutoff;
    if let (Some(next), Some(current)) = (commit.journal_cutoff(), preceding_cutoff)
        && next < current
    {
        return Err(JournalCodecError::new(
            "semantic Journal cutoff moved backwards",
        ));
    }
    validate_semantic_sequences(
        commit,
        (commit.kind() == JournalCommitKind::Incremental)
            .then_some(preceding_cutoff)
            .flatten(),
    )?;
    if commit.kind() == JournalCommitKind::Snapshot {
        if !commit.records().starts_with(&recovered.records) {
            return Err(JournalCodecError::new(
                "a complete snapshot must preserve the recovered semantic prefix",
            ));
        }
        let first = commit.records().first().ok_or_else(|| {
            JournalCodecError::new("a recovery snapshot must contain Journal state")
        })?;
        if first.sequence().get() != 1 {
            return Err(JournalCodecError::new(
                "a complete Journal snapshot must begin at sequence 1",
            ));
        }
        recovered.records.clear();
        recovered.descriptor = None;
        recovered.open_messages.clear();
        recovered.ended_messages.clear();
        recovered.submission_ids.clear();
        recovered.head = None;
        recovered.correlation = CorrelationRecovery::default();
    }
    if let Some(cutoff) = commit.journal_cutoff() {
        recovered.journal_cutoff = Some(cutoff);
    }
    let mut previous_in_commit = None;
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
        if let Some(sequence) = entry.journal_sequence() {
            recovered
                .correlation
                .observe(sequence, entry.record(), previous_in_commit)?;
        }
        apply_record(
            entry.record(),
            &mut recovered.descriptor,
            &mut recovered.open_messages,
            &mut recovered.ended_messages,
            &mut recovered.submission_ids,
        )?;
        recovered.records.push(entry.clone());
        recovered.head = Some(entry.sequence());
        previous_in_commit = entry
            .journal_sequence()
            .map(|sequence| (sequence, entry.record()));
    }
    recovered.discovery_states.push(RecoveredDiscovery {
        binding_epoch: recovered.correlation.open_epoch(),
        continuation_anchor: recovered.correlation.latest_anchor(),
    });
    Ok(())
}

fn validate_semantic_sequences(
    commit: &JournalCommit,
    preceding_cutoff: Option<JournalSequence>,
) -> Result<(), JournalCodecError> {
    let mut previous = None;
    for entry in commit.records() {
        match (
            entry.record().requires_journal_sequence(),
            entry.journal_sequence(),
        ) {
            (true, Some(sequence)) => {
                if previous.is_some_and(|prior| sequence <= prior) {
                    return Err(JournalCodecError::new(
                        "semantic journal_sequence values must be strictly increasing",
                    ));
                }
                if preceding_cutoff.is_some_and(|cutoff| sequence <= cutoff) {
                    return Err(JournalCodecError::new(
                        "incremental journal_sequence must exceed the preceding journal_cutoff",
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
                previous = Some(sequence);
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
    Ok(())
}

fn apply_record(
    record: &JournalRecord,
    descriptor: &mut Option<SessionDescriptor>,
    open_messages: &mut BTreeMap<ActivityRef, OpenMessage>,
    ended_messages: &mut BTreeSet<ActivityRef>,
    submission_ids: &mut BTreeSet<SubmissionId>,
) -> Result<(), JournalCodecError> {
    if let JournalRecord::SessionDescriptor(candidate) = record {
        if descriptor.replace(candidate.clone()).is_some() {
            return Err(JournalCodecError::new(
                "a recovered Session cannot contain more than one descriptor",
            ));
        }
        return Ok(());
    }
    if let JournalRecord::CommandCommitted(committed) = record
        && let Some(submission_id) = committed.submission_id()
        && !submission_ids.insert(submission_id)
    {
        return Err(duplicate_submission_id());
    }
    apply_message_record(record, open_messages, ended_messages)
}

fn duplicate_submission_id() -> JournalCodecError {
    JournalCodecError::new("a SubmissionId may identify only one committed submission per Session")
}

fn apply_message_record(
    record: &JournalRecord,
    open_messages: &mut BTreeMap<ActivityRef, OpenMessage>,
    ended_messages: &mut BTreeSet<ActivityRef>,
) -> Result<(), JournalCodecError> {
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
        JournalRecord::EventCommitted(AgentEvent::ActivityStarted { activity, kind }) => {
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
        JournalRecord::EventCommitted(AgentEvent::ActivityFinished { activity, .. }) => {
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
        JournalRecord::SessionDescriptor(_) => unreachable!("descriptors are handled above"),
        JournalRecord::CommandCommitted(_)
        | JournalRecord::EventCommitted(_)
        | JournalRecord::BackendExchangeObserved(_)
        | JournalRecord::BackendBindingOpened(_)
        | JournalRecord::BackendBindingClosed(_)
        | JournalRecord::BackendRequestAccepted(_)
        | JournalRecord::BackendResumableOutcome(_)
        | JournalRecord::ContinuationAnchor(_) => {},
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
    journal_cutoff: Option<JournalSequence>,
    open_messages: &BTreeMap<ActivityRef, OpenMessage>,
) -> Result<Option<JournalCommit>, JournalCodecError> {
    if open_messages.is_empty() {
        return Ok(None);
    }
    let journal_cutoff = journal_cutoff.ok_or_else(|| {
        JournalCodecError::new("an open durable message requires a semantic Journal cutoff")
    })?;
    let mut next = head
        .map_or(1, ReplaySequence::get)
        .checked_add(u64::from(head.is_some()))
        .ok_or_else(|| JournalCodecError::new("Journal sequence is exhausted"))?;
    let mut records = Vec::with_capacity(open_messages.len());
    for (activity, state) in open_messages {
        records.push(SequencedJournalRecord::storage(
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
