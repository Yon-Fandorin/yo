use std::{
    fmt,
    sync::{Arc, RwLock},
};

use super::{
    JournalDurability, JournalEntry, JournalSequence, SemanticRecord, SessionJournalState,
    read_state,
};
use crate::{AgentCommand, AgentEvent, ContextPolicyChanged};

const READ_LIMIT: usize = 256;

/// Read-only access to the semantic history of one live Session.
///
/// The reader copies a bounded suffix and never exposes the Journal's lock or
/// storage layout. A future remote implementation can therefore preserve the
/// same sequence-based contract without copying the local implementation.
/// Keeping a reader alive also retains the in-memory Session history it reads.
#[derive(Clone)]
pub struct TranscriptReader {
    pub(super) state: Arc<RwLock<SessionJournalState>>,
}

impl fmt::Debug for TranscriptReader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TranscriptReader")
            .finish_non_exhaustive()
    }
}

impl TranscriptReader {
    /// Reports whether the currently visible semantic suffix is durable.
    #[must_use]
    pub fn durability(&self) -> JournalDurability {
        read_state(&self.state).durability
    }

    /// Copies the next bounded group of records after `sequence`.
    ///
    /// `None` starts before the first record. Callers continue by passing the
    /// sequence of the last returned entry.
    #[must_use]
    pub fn read_after(&self, sequence: Option<JournalSequence>) -> TranscriptSlice {
        let state = read_state(&self.state);
        let head = state
            .entries
            .iter()
            .rev()
            .find_map(TranscriptEntry::from_journal)
            .map(|entry| entry.sequence());
        let start = sequence.map_or(0, |sequence| {
            usize::try_from(sequence.get())
                .unwrap_or(usize::MAX)
                .min(state.entries.len())
        });
        let entries = state
            .entries
            .iter()
            .skip(start)
            .filter_map(TranscriptEntry::from_journal)
            .take(READ_LIMIT)
            .collect::<Vec<_>>();
        drop(state);

        TranscriptSlice {
            from: entries.first().map(TranscriptEntry::sequence),
            head,
            entries,
        }
    }

    /// Copies the next ordered group of records and durability transitions.
    ///
    /// A durability transition precedes the semantic records whose publication observed it, so a
    /// frontend never has to infer whether an already-delivered suffix was volatile.
    #[must_use]
    pub fn read_observations_after(
        &self,
        sequence: Option<TranscriptObservationSequence>,
    ) -> TranscriptObservationSlice {
        let state = read_state(&self.state);
        let head = state
            .observations
            .last()
            .map(JournalObservationEntry::sequence);
        let start = sequence.map_or(0, |sequence| {
            usize::try_from(sequence.get())
                .unwrap_or(usize::MAX)
                .min(state.observations.len())
        });
        let entries = state
            .observations
            .iter()
            .skip(start)
            .take(READ_LIMIT)
            .map(TranscriptObservationEntry::from)
            .collect();
        TranscriptObservationSlice { head, entries }
    }

    /// Returns the newest available sequence without copying records.
    #[must_use]
    pub fn head_sequence(&self) -> Option<JournalSequence> {
        read_state(&self.state)
            .entries
            .iter()
            .rev()
            .find_map(TranscriptEntry::from_journal)
            .map(|entry| entry.sequence())
    }
}

/// Opaque order shared by semantic records and durability transitions.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TranscriptObservationSequence(u64);

impl TranscriptObservationSequence {
    pub(super) fn from_index(index: usize) -> Self {
        let value = index
            .checked_add(1)
            .and_then(|value| u64::try_from(value).ok())
            .expect("a Transcript cannot contain more observations than u64 can address");
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct JournalObservationEntry {
    sequence: TranscriptObservationSequence,
    observation: TranscriptObservation,
}

impl JournalObservationEntry {
    pub(super) fn durability(
        sequence: TranscriptObservationSequence,
        durability: JournalDurability,
    ) -> Self {
        Self {
            sequence,
            observation: TranscriptObservation::Durability(durability),
        }
    }

    pub(super) fn record(
        sequence: TranscriptObservationSequence,
        record: TranscriptRecord,
    ) -> Self {
        Self {
            sequence,
            observation: TranscriptObservation::Record(record),
        }
    }

    pub(super) const fn sequence(&self) -> TranscriptObservationSequence {
        self.sequence
    }
}

/// One ordered frontend observation from a live Session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TranscriptObservation {
    Durability(JournalDurability),
    Record(TranscriptRecord),
}

/// One ordered observation with an opaque continuation coordinate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriptObservationEntry {
    sequence: TranscriptObservationSequence,
    observation: TranscriptObservation,
}

impl TranscriptObservationEntry {
    #[must_use]
    pub const fn sequence(&self) -> TranscriptObservationSequence {
        self.sequence
    }

    #[must_use]
    pub const fn observation(&self) -> &TranscriptObservation {
        &self.observation
    }
}

impl From<&JournalObservationEntry> for TranscriptObservationEntry {
    fn from(entry: &JournalObservationEntry) -> Self {
        Self {
            sequence: entry.sequence,
            observation: entry.observation.clone(),
        }
    }
}

/// One bounded ordered read of semantic records and durability transitions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriptObservationSlice {
    head: Option<TranscriptObservationSequence>,
    entries: Vec<TranscriptObservationEntry>,
}

impl TranscriptObservationSlice {
    #[must_use]
    pub const fn head(&self) -> Option<TranscriptObservationSequence> {
        self.head
    }

    #[must_use]
    pub fn into_entries(self) -> Vec<TranscriptObservationEntry> {
        self.entries
    }
}

/// One bounded, immutable Transcript read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriptSlice {
    from: Option<JournalSequence>,
    head: Option<JournalSequence>,
    entries: Vec<TranscriptEntry>,
}

impl TranscriptSlice {
    /// The first sequence returned by this read, if any.
    ///
    /// Keeping this value explicit lets a future repository report that the
    /// requested suffix begins after an unavailable durable range.
    #[must_use]
    pub const fn from(&self) -> Option<JournalSequence> {
        self.from
    }

    /// The newest sequence available when this read was captured.
    #[must_use]
    pub const fn head(&self) -> Option<JournalSequence> {
        self.head
    }

    #[must_use]
    pub fn entries(&self) -> &[TranscriptEntry] {
        &self.entries
    }

    #[must_use]
    pub fn into_entries(self) -> Vec<TranscriptEntry> {
        self.entries
    }
}

/// One committed semantic record displayed by Transcript projections.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriptEntry {
    sequence: JournalSequence,
    record: TranscriptRecord,
}

impl TranscriptEntry {
    #[must_use]
    pub const fn sequence(&self) -> JournalSequence {
        self.sequence
    }

    #[must_use]
    pub const fn record(&self) -> &TranscriptRecord {
        &self.record
    }
}

impl TranscriptEntry {
    fn from_journal(entry: &JournalEntry) -> Option<Self> {
        let record = TranscriptRecord::from_journal(entry)?;
        Some(Self {
            sequence: entry.sequence(),
            record,
        })
    }
}

impl TranscriptRecord {
    pub(super) fn from_journal(entry: &JournalEntry) -> Option<Self> {
        Some(match entry.record() {
            SemanticRecord::CommandCommitted(command) => {
                Self::CommandCommitted(command.command().clone())
            },
            SemanticRecord::EventCommitted(event) => Self::EventCommitted(event.clone()),
            SemanticRecord::BackendExchangeObserved(_)
            | SemanticRecord::BackendBindingOpened(_)
            | SemanticRecord::BackendBindingClosed(_)
            | SemanticRecord::BackendRequestAccepted(_)
            | SemanticRecord::ModelReplayDelta(_)
            | SemanticRecord::BackendResumableOutcome(_)
            | SemanticRecord::ContinuationAnchor(_) => return None,
            SemanticRecord::ContextPolicyChanged(policy) => {
                Self::ContextPolicyChanged(policy.clone())
            },
            SemanticRecord::ContextCheckpoint(checkpoint) => {
                Self::ContextCheckpointCommitted(ContextCheckpointObservation::from(checkpoint))
            },
        })
    }
}

/// Redacted operator-facing facts about one durably committed lossy checkpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContextCheckpointObservation {
    source_anchor_sequence: JournalSequence,
    source_journal_boundary: JournalSequence,
    policy_revision: u64,
    previous_context_epoch: u64,
    successor_context_epoch: u64,
    input_token_limit: u64,
    input_tokens_before: u64,
    input_tokens_after: u64,
    retained_group_count: u64,
    artifact_receipt_count: u64,
    visible_prefix_loss_count: u64,
    provider_private_loss_count: u64,
}

impl From<&super::codec::ContextCheckpoint> for ContextCheckpointObservation {
    fn from(checkpoint: &super::codec::ContextCheckpoint) -> Self {
        let visible_prefix_loss_count = checkpoint
            .losses()
            .iter()
            .filter(|loss| {
                matches!(
                    loss,
                    super::codec::ContextLoss::VisiblePrefixSummarized { .. }
                )
            })
            .count();
        let provider_private_loss_count = checkpoint
            .losses()
            .len()
            .saturating_sub(visible_prefix_loss_count);
        Self {
            source_anchor_sequence: checkpoint.source_anchor_sequence(),
            source_journal_boundary: checkpoint.source_journal_boundary(),
            policy_revision: checkpoint.policy_revision(),
            previous_context_epoch: checkpoint.previous_context_epoch(),
            successor_context_epoch: checkpoint.successor_context_epoch(),
            input_token_limit: checkpoint.input_token_limit(),
            input_tokens_before: checkpoint.input_tokens_before(),
            input_tokens_after: checkpoint.input_tokens_after(),
            retained_group_count: u64::try_from(checkpoint.retained_groups().len())
                .expect("bounded retained group count fits u64"),
            artifact_receipt_count: u64::try_from(checkpoint.artifact_receipts().len())
                .expect("bounded artifact receipt count fits u64"),
            visible_prefix_loss_count: u64::try_from(visible_prefix_loss_count)
                .expect("bounded loss count fits u64"),
            provider_private_loss_count: u64::try_from(provider_private_loss_count)
                .expect("bounded loss count fits u64"),
        }
    }
}

impl ContextCheckpointObservation {
    #[must_use]
    pub const fn source_anchor_sequence(self) -> JournalSequence {
        self.source_anchor_sequence
    }

    #[must_use]
    pub const fn source_journal_boundary(self) -> JournalSequence {
        self.source_journal_boundary
    }

    #[must_use]
    pub const fn policy_revision(self) -> u64 {
        self.policy_revision
    }

    #[must_use]
    pub const fn previous_context_epoch(self) -> u64 {
        self.previous_context_epoch
    }

    #[must_use]
    pub const fn successor_context_epoch(self) -> u64 {
        self.successor_context_epoch
    }

    #[must_use]
    pub const fn input_token_limit(self) -> u64 {
        self.input_token_limit
    }

    #[must_use]
    pub const fn input_tokens_before(self) -> u64 {
        self.input_tokens_before
    }

    #[must_use]
    pub const fn input_tokens_after(self) -> u64 {
        self.input_tokens_after
    }

    #[must_use]
    pub const fn retained_group_count(self) -> u64 {
        self.retained_group_count
    }

    #[must_use]
    pub const fn artifact_receipt_count(self) -> u64 {
        self.artifact_receipt_count
    }

    #[must_use]
    pub const fn visible_prefix_loss_count(self) -> u64 {
        self.visible_prefix_loss_count
    }

    #[must_use]
    pub const fn provider_private_loss_count(self) -> u64 {
        self.provider_private_loss_count
    }
}

/// Frontend-independent meaning retained in the chronological Transcript.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TranscriptRecord {
    CommandCommitted(AgentCommand),
    EventCommitted(AgentEvent),
    ContextPolicyChanged(ContextPolicyChanged),
    ContextCheckpointCommitted(ContextCheckpointObservation),
}

#[cfg(test)]
mod tests;
