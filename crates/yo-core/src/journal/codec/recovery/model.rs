use std::collections::{BTreeMap, BTreeSet};

use super::{
    super::{
        ContextPolicyChanged, JournalCommit, MessageStream, ReplaySequence, SequencedJournalRecord,
    },
    correlation::CorrelationRecovery,
};
use crate::{ActivityRef, JournalSequence, SessionDescriptor, SubmissionId, journal::JournalEntry};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HistoricalForkKind {
    Anchor,
    Checkpoint,
    InitialFork,
}

pub(crate) struct HistoricalForkPoint {
    pub(crate) source: HistoricalForkKind,
    pub(crate) cutoff: JournalSequence,
    pub(crate) logical_cutoff: JournalSequence,
    pub(crate) binding_epoch: u64,
    pub(crate) context_epoch: u64,
    pub(crate) model_label: String,
    pub(crate) input_excerpt: Option<String>,
    pub(crate) record_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecoveredJournal {
    pub(super) records: Vec<SequencedJournalRecord>,
    pub(super) journal_cutoff: Option<JournalSequence>,
    pub(super) descriptor: Option<SessionDescriptor>,
    pub(super) recovery_commit: Option<JournalCommit>,
    pub(super) open_messages: BTreeMap<ActivityRef, OpenMessage>,
    pub(super) ended_messages: BTreeSet<ActivityRef>,
    pub(super) submission_ids: BTreeSet<SubmissionId>,
    pub(super) head: Option<ReplaySequence>,
    pub(super) correlation: CorrelationRecovery,
    pub(super) discovery_states: Vec<RecoveredDiscovery>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RecoveredDiscovery {
    binding_epoch: Option<u64>,
    continuation_anchor: Option<JournalSequence>,
    initial_fork_seed: Option<JournalSequence>,
}

impl RecoveredDiscovery {
    pub(super) const fn new(
        binding_epoch: Option<u64>,
        continuation_anchor: Option<JournalSequence>,
        initial_fork_seed: Option<JournalSequence>,
    ) -> Self {
        Self {
            binding_epoch,
            continuation_anchor,
            initial_fork_seed,
        }
    }

    pub(crate) const fn binding_epoch(self) -> Option<u64> {
        self.binding_epoch
    }

    pub(crate) const fn continuation_anchor(self) -> Option<JournalSequence> {
        self.continuation_anchor
    }

    pub(crate) const fn initial_fork_seed(self) -> Option<JournalSequence> {
        self.initial_fork_seed
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct OpenMessage {
    pub(super) stream: MessageStream,
    pub(super) revision: u64,
    pub(super) segment_count: u64,
    pub(super) utf8_bytes: u64,
}

impl RecoveredJournal {
    pub(super) fn new() -> Self {
        Self {
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
        }
    }
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

    pub(crate) fn initial_fork_seed(&self) -> Option<JournalSequence> {
        self.correlation.initial_fork_seed()
    }

    pub(crate) const fn context_checkpoint(&self) -> Option<JournalSequence> {
        self.correlation.latest_checkpoint()
    }

    pub(crate) const fn context_epoch(&self) -> Option<u64> {
        self.correlation.context_epoch()
    }

    pub(crate) const fn context_policy(&self) -> Option<&ContextPolicyChanged> {
        self.correlation.current_policy()
    }

    pub(crate) fn model_replay_groups(&self) -> Vec<Vec<crate::ModelReplayItem>> {
        self.correlation.replay_groups()
    }

    pub(crate) const fn model_replay(&self) -> &crate::ModelReplay {
        self.correlation.model_replay()
    }

    pub(crate) const fn replay_contract_rebind_required(&self) -> bool {
        self.correlation.replay_contract_rebind_required()
    }

    pub(crate) fn discovery_states(&self) -> &[RecoveredDiscovery] {
        &self.discovery_states
    }

    pub(crate) fn submission_ids(&self) -> &BTreeSet<SubmissionId> {
        &self.submission_ids
    }

    pub(crate) fn semantic_entries(&self) -> Vec<JournalEntry> {
        self.records
            .iter()
            .filter_map(|entry| {
                let sequence = entry.journal_sequence()?;
                entry
                    .record()
                    .semantic_record()
                    .map(|record| JournalEntry::new(sequence, record))
            })
            .collect()
    }
}
