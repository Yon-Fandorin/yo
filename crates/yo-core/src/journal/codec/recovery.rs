use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

mod correlation;

use correlation::CorrelationRecovery;

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

use super::{
    ForkHistoryCoordinate, ForkMessagePart, ForkSeed, ForkSource, ForkSourcePoint, InitialForkSeed,
    JournalCodecError, JournalCommit, JournalCommitKind, JournalRecord, MessageEnded,
    MessageOutcome, MessageStream, ReplaySequence, SequencedJournalRecord, TransitionMode, wire,
};
use crate::{
    ActivityRef, AgentCommand, AgentEvent, BackendBindingEvidence, BackendIdentity,
    ContinuationStrategy, JournalSequence, ReplayExecutor, SessionDescriptor, SessionId,
    SubmissionId,
};

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
    initial_fork_seed: Option<JournalSequence>,
}

impl RecoveredDiscovery {
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

    /// Captures one complete parent reconstruction without reading any ancestor.
    pub(crate) fn capture_fork(
        &self,
        child: SessionId,
    ) -> Result<InitialForkSeed, JournalCodecError> {
        let (parent, source) = self.fork_source()?;
        if child == parent {
            return Err(JournalCodecError::new(
                "fork child must differ from its parent",
            ));
        }
        let replay = self.correlation.fork_replay()?;
        let history = self.fork_history(
            parent,
            source
                .point()
                .expect("non-empty source selected")
                .journal_boundary(),
        )?;
        wire::prepare_initial_fork_seed(
            child,
            parent,
            source,
            ForkSeed::ExactReplay(replay),
            history,
        )
    }

    pub(crate) fn validate_fork_capture(&self) -> Result<(), JournalCodecError> {
        self.fork_source()?;
        self.correlation.fork_replay()?;
        Ok(())
    }

    fn fork_source(&self) -> Result<(SessionId, ForkSource), JournalCodecError> {
        if !self.open_messages.is_empty() || self.recovery_commit.is_some() {
            return Err(JournalCodecError::new(
                "fork source contains an incomplete message",
            ));
        }
        let parent = self
            .descriptor
            .as_ref()
            .ok_or_else(|| JournalCodecError::new("fork source has no durable descriptor"))?
            .session_id();
        let epoch = self
            .binding_epoch()
            .ok_or_else(|| JournalCodecError::new("fork source has no current binding"))?;
        let binding = self
            .records
            .iter()
            .find_map(|entry| match entry.record() {
                JournalRecord::BackendBindingOpened(binding) if binding.epoch() == epoch => {
                    Some(binding)
                },
                _ => None,
            })
            .ok_or_else(|| JournalCodecError::new("fork source binding is absent"))?;
        if !matches!(
            binding.continuation_strategy(),
            ContinuationStrategy::ExactReplay { .. }
        ) {
            return Err(JournalCodecError::new(
                "native fork requires adapter-supported exact boundary proof",
            ));
        }
        let context_epoch = self
            .context_epoch()
            .ok_or_else(|| JournalCodecError::new("fork source has no qualified context epoch"))?;
        let cutoff = self
            .journal_cutoff
            .ok_or_else(|| JournalCodecError::new("fork source has no committed cutoff"))?;
        let evidence = BackendBindingEvidence::new(
            binding.backend_kind(),
            binding.backend_version(),
            BackendIdentity::new(
                binding.binding_identity().schema(),
                binding.binding_identity().value(),
            ),
            BackendIdentity::new(
                binding.model_identity().schema(),
                binding.model_identity().value(),
            ),
            BackendIdentity::new(
                binding.session_locator().schema(),
                binding.session_locator().value(),
            ),
            binding.continuation_strategy(),
        );
        let current_request = self.records.iter().any(|entry| {
            matches!(entry.record(), JournalRecord::BackendRequestAccepted(request) if request.epoch() == epoch)
        });
        // A validated exact replacement may still own the preceding reconstruction.
        let transition = (!current_request
            && self.continuation_anchor().is_none()
            && self.context_checkpoint().is_none()
            && self.initial_fork_seed().is_none()
            && binding.transition().mode() == TransitionMode::ExactReplay)
            .then_some(binding.transition());
        let anchor = self
            .continuation_anchor()
            .or_else(|| transition.and_then(|transition| transition.source_anchor_sequence()));
        let checkpoint = self
            .context_checkpoint()
            .or_else(|| transition.and_then(|transition| transition.source_checkpoint_sequence()));
        let source = if let Some(sequence) = anchor {
            let anchor = self
                .records
                .iter()
                .find_map(|entry| match (entry.journal_sequence(), entry.record()) {
                    (Some(found), JournalRecord::ContinuationAnchor(anchor))
                        if found == sequence =>
                    {
                        Some(anchor)
                    },
                    _ => None,
                })
                .ok_or_else(|| JournalCodecError::new("fork source Anchor is absent"))?;
            if anchor.context_epoch() != Some(context_epoch) {
                return Err(JournalCodecError::new(
                    "fork source Anchor context is stale",
                ));
            }
            ForkSource::Anchor(ForkSourcePoint::new(
                anchor.epoch(),
                context_epoch,
                sequence,
                anchor.journal_boundary(),
                evidence,
            )?)
        } else if let Some(sequence) = checkpoint {
            let checkpoint = self
                .records
                .iter()
                .find_map(|entry| match (entry.journal_sequence(), entry.record()) {
                    (Some(found), JournalRecord::ContextCheckpoint(checkpoint))
                        if found == sequence =>
                    {
                        Some(checkpoint)
                    },
                    _ => None,
                })
                .ok_or_else(|| JournalCodecError::new("fork source checkpoint is absent"))?;
            if checkpoint.successor_context_epoch() != context_epoch {
                return Err(JournalCodecError::new(
                    "fork source checkpoint context is stale",
                ));
            }
            ForkSource::Checkpoint(ForkSourcePoint::new(
                checkpoint.epoch(),
                context_epoch,
                sequence,
                sequence,
                evidence,
            )?)
        } else if let Some(sequence) = self.initial_fork_seed() {
            ForkSource::InitialFork(ForkSourcePoint::new(
                epoch,
                context_epoch,
                sequence,
                cutoff,
                evidence,
            )?)
        } else {
            return Err(JournalCodecError::new(
                "fork source has no complete reconstruction",
            ));
        };
        let point = source.point().expect("non-empty source selected");
        if point.record_sequence() > cutoff {
            return Err(JournalCodecError::new(
                "fork source exceeds its durable cutoff",
            ));
        }
        Ok((parent, source))
    }

    /// Scans the fully validated history once; replay is reconstructed only after selection.
    pub(crate) fn historical_fork_boundaries(
        &self,
        limit: usize,
    ) -> Result<(Vec<HistoricalForkPoint>, bool), JournalCodecError> {
        let mut correlation = CorrelationRecovery::default();
        let mut descriptor = None;
        let mut open_messages = BTreeMap::new();
        let mut ended_messages = BTreeSet::new();
        let mut submission_ids = BTreeSet::new();
        let mut previous = None;
        let mut binding = None;
        let mut anchors = BTreeMap::new();
        let mut points = VecDeque::new();
        let mut truncated = false;
        let mut input_excerpt = None;
        for (index, entry) in self.records.iter().enumerate() {
            if let Some(sequence) = entry.journal_sequence() {
                correlation.observe(sequence, entry.record(), previous)?;
            }
            apply_record(
                entry.record(),
                &mut descriptor,
                &mut open_messages,
                &mut ended_messages,
                &mut submission_ids,
            )?;
            if let JournalRecord::BackendBindingOpened(opened) = entry.record() {
                binding = Some(opened);
            }
            if let JournalRecord::CommandCommitted(command) = entry.record()
                && let AgentCommand::StartTurn { input, .. } | AgentCommand::SteerTurn { input, .. } =
                    command.command()
            {
                let visible = input
                    .as_str()
                    .chars()
                    .take(120)
                    .map(|character| {
                        if character.is_control() {
                            ' '
                        } else {
                            character
                        }
                    })
                    .collect::<String>();
                input_excerpt = (!visible.is_empty()).then_some(visible);
            }
            if let (Some(sequence), JournalRecord::ContinuationAnchor(anchor)) =
                (entry.journal_sequence(), entry.record())
            {
                anchors.insert(sequence, anchor.journal_boundary());
            }
            previous = entry
                .journal_sequence()
                .map(|sequence| (sequence, entry.record()));
            let Some(cutoff) = entry.journal_sequence() else {
                continue;
            };
            if !matches!(
                entry.record(),
                JournalRecord::ContinuationAnchor(_)
                    | JournalRecord::ContextCheckpoint(_)
                    | JournalRecord::BackendBindingOpened(_)
                    | JournalRecord::ContextPolicyChanged(_)
            ) || !open_messages.is_empty()
                || !correlation.fork_boundary_is_idle()
            {
                continue;
            }
            let Some(binding) =
                binding.filter(|binding| correlation.open_epoch() == Some(binding.epoch()))
            else {
                continue;
            };
            if !matches!(
                binding.continuation_strategy(),
                ContinuationStrategy::ExactReplay { .. }
            ) {
                continue;
            }
            let Some(context_epoch) = correlation.context_epoch() else {
                continue;
            };
            // An initial fork point includes its complete bootstrap, including initial policy.
            if correlation.current_policy().is_none() {
                continue;
            }
            let transition = (!correlation.open_binding_has_accepted_request()
                && binding.transition().mode() == TransitionMode::ExactReplay)
                .then_some(binding.transition());
            let anchor = correlation
                .latest_anchor()
                .or_else(|| transition.and_then(|transition| transition.source_anchor_sequence()));
            let checkpoint = correlation.latest_checkpoint().or_else(|| {
                transition.and_then(|transition| transition.source_checkpoint_sequence())
            });
            let (source, logical_cutoff) = if let Some(anchor) = anchor {
                let Some(boundary) = anchors.get(&anchor).copied() else {
                    continue;
                };
                (HistoricalForkKind::Anchor, boundary)
            } else if let Some(checkpoint) = checkpoint {
                (HistoricalForkKind::Checkpoint, checkpoint)
            } else if correlation.initial_fork_seed().is_some() {
                (HistoricalForkKind::InitialFork, cutoff)
            } else {
                continue;
            };
            if matches!(entry.record(), JournalRecord::ContextPolicyChanged(_))
                && source != HistoricalForkKind::InitialFork
            {
                continue;
            }
            if points.len() == limit {
                points.pop_front();
                truncated = true;
            }
            points.push_back(HistoricalForkPoint {
                source,
                cutoff,
                logical_cutoff,
                binding_epoch: binding.epoch(),
                context_epoch,
                model_label: binding.model_identity().value().to_owned(),
                input_excerpt: input_excerpt.clone(),
                record_count: index + 1,
            });
        }
        Ok((points.into_iter().rev().collect(), truncated))
    }

    pub(crate) fn historical_fork_prefix(
        &self,
        record_count: usize,
        cutoff: JournalSequence,
    ) -> Result<Self, JournalCodecError> {
        let records = self.records.get(..record_count).ok_or_else(|| {
            JournalCodecError::new("historical fork cutoff is outside its capture")
        })?;
        let prefix = recover(&[JournalCommit::snapshot_through(cutoff, records.to_vec())])?;
        prefix.validate_fork_capture()?;
        Ok(prefix)
    }

    fn fork_history(
        &self,
        parent: SessionId,
        boundary: JournalSequence,
    ) -> Result<Vec<(SessionId, ForkHistoryCoordinate, &SequencedJournalRecord)>, JournalCodecError>
    {
        let mut history = self
            .records
            .iter()
            .find_map(|entry| match entry.record() {
                JournalRecord::InitialForkSeed(seed) => Some(
                    seed.history()
                        .iter()
                        .map(|entry| {
                            (
                                entry.source_session_id(),
                                entry.source_coordinate(),
                                entry.record(),
                            )
                        })
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .unwrap_or_default();
        let mut ordinals = HashMap::new();
        for entry in &self.records {
            if entry
                .journal_sequence()
                .is_some_and(|sequence| sequence > boundary)
            {
                break;
            }
            let message = match entry.record() {
                JournalRecord::MessageReset(reset) => {
                    Some((reset.activity(), ForkMessagePart::Reset))
                },
                JournalRecord::MessageSegment(segment) => {
                    Some((segment.activity(), ForkMessagePart::Segment))
                },
                JournalRecord::MessageEnded(terminal) => {
                    Some((terminal.ended().activity(), ForkMessagePart::Ended))
                },
                _ => None,
            };
            let coordinate = if let Some((activity, part)) = message {
                let next = ordinals.entry((activity, part)).or_insert(0u64);
                let coordinate = ForkHistoryCoordinate::Message {
                    activity,
                    part,
                    ordinal: *next,
                };
                *next = next
                    .checked_add(1)
                    .ok_or_else(|| JournalCodecError::new("fork message ordinal overflow"))?;
                coordinate
            } else {
                match entry.record() {
                    JournalRecord::CommandCommitted(command)
                        if matches!(
                            command.command(),
                            AgentCommand::CreateSession { .. }
                                | AgentCommand::CompactContext { .. }
                        ) =>
                    {
                        continue;
                    },
                    JournalRecord::CommandCommitted(_) | JournalRecord::EventCommitted(_) => {
                        ForkHistoryCoordinate::Journal {
                            sequence: entry.journal_sequence().ok_or_else(|| {
                                JournalCodecError::new("fork archival event has no coordinate")
                            })?,
                        }
                    },
                    _ => continue,
                }
            };
            if history.len() >= 4096 {
                return Err(JournalCodecError::new("fork history item limit exceeded"));
            }
            history.push((parent, coordinate, entry));
        }
        Ok(history)
    }

    pub(crate) const fn context_checkpoint(&self) -> Option<JournalSequence> {
        self.correlation.latest_checkpoint()
    }

    pub(crate) const fn context_epoch(&self) -> Option<u64> {
        self.correlation.context_epoch()
    }

    pub(crate) const fn context_policy(&self) -> Option<&super::ContextPolicyChanged> {
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

    pub(crate) fn semantic_entries(&self) -> Vec<crate::journal::JournalEntry> {
        self.records
            .iter()
            .filter_map(|entry| {
                let sequence = entry.journal_sequence()?;
                entry
                    .record()
                    .semantic_record()
                    .map(|record| crate::journal::JournalEntry::new(sequence, record))
            })
            .collect()
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
    validate_checkpoint_commit_boundary(commit)?;
    validate_fork_bootstrap(recovered, commit)?;
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
        initial_fork_seed: recovered.correlation.initial_fork_seed(),
    });
    Ok(())
}

fn validate_fork_bootstrap(
    recovered: &RecoveredJournal,
    commit: &JournalCommit,
) -> Result<(), JournalCodecError> {
    let mut seed_entry = None;
    let mut first_binding = None;
    for (index, entry) in commit.records().iter().enumerate() {
        match entry.record() {
            JournalRecord::InitialForkSeed(seed) => {
                if seed_entry
                    .replace((index, entry.journal_sequence(), seed))
                    .is_some()
                {
                    return Err(JournalCodecError::new("duplicate initial fork seed"));
                }
            },
            JournalRecord::BackendBindingOpened(binding) => {
                if binding.transition().mode() == TransitionMode::InitialFork
                    && first_binding.is_some()
                {
                    return Err(JournalCodecError::new(
                        "initial fork cannot replace a binding",
                    ));
                }
                first_binding.get_or_insert((index, binding));
            },
            _ => {},
        }
    }
    let Some((seed_index, seed_sequence, seed)) = seed_entry else {
        if first_binding
            .is_some_and(|(_, binding)| binding.transition().mode() == TransitionMode::InitialFork)
        {
            return Err(JournalCodecError::new(
                "initial fork binding has no seed in its commit",
            ));
        }
        return Ok(());
    };
    if recovered.head.is_some()
        && (commit.kind() != JournalCommitKind::Snapshot
            || !recovered
                .records
                .iter()
                .any(|entry| matches!(entry.record(), JournalRecord::InitialForkSeed(_))))
    {
        return Err(JournalCodecError::new(
            "late initial fork seed outside initial publication",
        ));
    }
    let Some(JournalRecord::SessionDescriptor(descriptor)) =
        commit.records().first().map(SequencedJournalRecord::record)
    else {
        return Err(JournalCodecError::new(
            "fork bootstrap requires its own leading descriptor",
        ));
    };
    seed.validate_child(descriptor.session_id())?;
    if !commit.records()[..seed_index].iter().any(|entry| {
        matches!(entry.record(), JournalRecord::EventCommitted(AgentEvent::SessionCreated { session_id })
            if *session_id == descriptor.session_id())
    }) || commit.records().iter().any(|entry| {
        entry.record().session_id().is_some_and(|id| id != descriptor.session_id())
    }) {
        return Err(JournalCodecError::new("fork bootstrap requires matching child session creation"));
    }
    let Some((binding_index, binding)) = first_binding else {
        return Err(JournalCodecError::new(
            "fork bootstrap has no first binding",
        ));
    };
    if seed_index >= binding_index
        || binding.epoch() != 1
        || binding.transition().mode() != TransitionMode::InitialFork
        || seed_sequence.is_none()
        || binding.transition().fork_seed_sequence() != seed_sequence
    {
        return Err(JournalCodecError::new(
            "fork bootstrap binding must follow its exact seed in epoch 1",
        ));
    }
    let first_request = commit
        .records()
        .iter()
        .position(|entry| matches!(entry.record(), JournalRecord::BackendRequestAccepted(_)))
        .unwrap_or(commit.records().len());
    if first_request <= binding_index {
        return Err(JournalCodecError::new(
            "fork bootstrap precedes all accepted child requests",
        ));
    }
    if matches!(binding.continuation_strategy(), ContinuationStrategy::ExactReplay {
        executor: ReplayExecutor::LocalClient, ..
    }) && !commit.records()[binding_index + 1..first_request].iter().any(|entry| {
        matches!(entry.record(), JournalRecord::ContextPolicyChanged(policy) if policy.policy_revision() == 1)
    }) {
        return Err(JournalCodecError::new("fork bootstrap requires initial context policy in the same commit"));
    }
    Ok(())
}

fn validate_checkpoint_commit_boundary(commit: &JournalCommit) -> Result<(), JournalCodecError> {
    if commit.kind() != JournalCommitKind::Incremental {
        return Ok(());
    }
    if commit.records().iter().enumerate().any(|(index, entry)| {
        matches!(entry.record(), JournalRecord::ContextCheckpoint(_))
            && index + 1 != commit.records().len()
    }) {
        return Err(JournalCodecError::new(
            "an incremental context checkpoint must be the final record in its physical commit",
        ));
    }
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
        | JournalRecord::ModelReplayDelta(_)
        | JournalRecord::BackendResumableOutcome(_)
        | JournalRecord::ContinuationAnchor(_)
        | JournalRecord::ContextPolicyChanged(_)
        | JournalRecord::ContextCheckpoint(_)
        | JournalRecord::InitialForkSeed(_) => {},
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
