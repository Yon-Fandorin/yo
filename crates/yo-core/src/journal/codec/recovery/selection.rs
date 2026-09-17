use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use super::{
    super::{
        ForkHistoryCoordinate, ForkMessagePart, ForkSeed, ForkSource, ForkSourcePoint,
        InitialForkSeed, JournalCodecError, JournalCommit, JournalCommitKind, JournalRecord,
        SequencedJournalRecord, TransitionMode, wire,
    },
    apply::apply_record,
    correlation::CorrelationRecovery,
    model::{HistoricalForkKind, HistoricalForkPoint, RecoveredJournal},
    recover,
};
use crate::{
    AgentCommand, AgentEvent, BackendBindingEvidence, BackendIdentity, ContinuationStrategy,
    JournalSequence, ReplayExecutor, SessionId,
};

impl RecoveredJournal {
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
}

pub(super) fn validate_fork_bootstrap(
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
