use std::mem;

use super::{
    super::super::{
        BackendBindingOpened, BindingCloseReason, CacheState, ContextImageLoss, ContextImageSource,
        ForkItemCoordinate, ForkItemOrigin, ForkSeed, JournalCodecError, TransitionMode,
    },
    CorrelationRecovery,
    model::{ReferenceTarget, ReplacementSource, ReplayGroup},
};
use crate::{
    BackendBindingEvidence, BackendIdentity, ContinuationStrategy, JournalSequence, ModelReplay,
    ReplayProfile,
};

impl CorrelationRecovery {
    pub(super) fn observe_binding_open(
        &mut self,
        sequence: JournalSequence,
        binding: &BackendBindingOpened,
    ) -> Result<(), JournalCodecError> {
        if self.open_epoch.is_some() {
            return Err(JournalCodecError::new(
                "at most one backend binding epoch may be open",
            ));
        }
        let is_replacement = self.last_epoch.is_some();
        let seed_items = self.model_replay.items().to_vec();
        let seed_image_losses = self
            .replay_groups
            .iter()
            .flat_map(|group| group.image_losses.iter().cloned())
            .collect::<Vec<_>>();
        match self.last_epoch {
            None => {
                let expected_mode = if self.fork_seed.is_some() {
                    TransitionMode::InitialFork
                } else {
                    TransitionMode::Initial
                };
                if binding.epoch() != 1
                    || !self.session_created
                    || binding.transition().mode() != expected_mode
                    || binding.transition().cache() != CacheState::NotApplicable
                    || binding.transition().source_anchor_sequence().is_some()
                    || binding.transition().source_checkpoint_sequence().is_some()
                    || binding
                        .transition()
                        .source_initial_fork_sequence()
                        .is_some()
                {
                    return Err(JournalCodecError::new(
                        "first backend binding must open epoch 1 after SessionCreated with an initial transition",
                    ));
                }
                if let Some(fork) = &mut self.fork_seed {
                    if binding.transition().fork_seed_sequence() != Some(fork.sequence) {
                        return Err(JournalCodecError::new(
                            "initial fork binding names another seed",
                        ));
                    }
                    match fork.seed.seed() {
                        ForkSeed::Empty => {},
                        ForkSeed::ExactReplay(replay) => {
                            let source = fork.seed.source().point().ok_or_else(|| {
                                JournalCodecError::new("exact fork requires a nonempty source")
                            })?;
                            if !fork_binding_matches(binding, source.binding()) {
                                return Err(JournalCodecError::new(
                                    "initial fork binding differs from its exact source identity or replay profile",
                                ));
                            }
                            self.model_replay = ModelReplay::from_checkpoint(
                                replay.contract().clone(),
                                replay.items().to_vec(),
                            )
                            .map_err(JournalCodecError::new)?;
                            let session_id = self.session_id.ok_or_else(|| {
                                JournalCodecError::new("fork import has no child Session")
                            })?;
                            self.model_replay_origins = replay
                                .item_origins()
                                .iter()
                                .enumerate()
                                .map(|(index, origin)| {
                                    ForkItemOrigin::new(
                                        *origin.original(),
                                        ForkItemCoordinate::new(
                                            session_id,
                                            1,
                                            1,
                                            fork.sequence,
                                            index,
                                        )?,
                                        origin.source_binding().clone(),
                                    )
                                })
                                .collect::<Result<Vec<_>, _>>()?;
                            self.replay_groups = replay
                                .groups()
                                .iter()
                                .enumerate()
                                .map(|(index, group)| {
                                    Ok(ReplayGroup {
                                        first_sequence: fork.sequence,
                                        last_sequence: fork.sequence,
                                        replay_delta_sequence: fork.sequence,
                                        epoch: 1,
                                        context_epoch: 1,
                                        items: replay.items()[group.first_item()..group.end_item()]
                                            .to_vec(),
                                        fork_group_index: Some(index),
                                        image_losses: ContextImageLoss::for_items(
                                            &replay.items()[group.first_item()..group.end_item()],
                                            1,
                                            |item_index, part_index| {
                                                ContextImageSource::InitialForkSeed {
                                                    sequence: fork.sequence.get(),
                                                    group_index: index as u32,
                                                    item_index,
                                                    part_index,
                                                }
                                            },
                                        )
                                        .map_err(JournalCodecError::new)?,
                                    })
                                })
                                .collect::<Result<Vec<_>, JournalCodecError>>()?;
                        },
                        ForkSeed::BackendNative { .. } => {
                            return Err(JournalCodecError::new(
                                "native fork source boundary schema has no supported verifier",
                            ));
                        },
                    }
                    fork.owner_epoch = Some(1);
                    self.context_epoch = Some(1);
                }
            },
            Some(previous) => {
                if let Some(fork) = &self.fork_seed
                    && self
                        .replay_groups
                        .iter()
                        .any(|group| group.fork_group_index.is_some())
                {
                    let source = fork.seed.source().point().ok_or_else(|| {
                        JournalCodecError::new("empty fork has no imported replacement root")
                    })?;
                    if fork.owner_epoch != Some(previous)
                        || binding.transition().mode() != TransitionMode::ExactReplay
                        || !fork_binding_matches(binding, source.binding())
                    {
                        return Err(JournalCodecError::new(
                            "fork replacement must preserve its current import owner identity and profile",
                        ));
                    }
                }
                if binding.epoch() != previous.saturating_add(1)
                    || self.last_close_reason != Some(BindingCloseReason::Replaced)
                    || matches!(
                        binding.transition().mode(),
                        TransitionMode::Initial | TransitionMode::InitialFork
                    )
                {
                    return Err(JournalCodecError::new(
                        "replacement binding must follow a replaced epoch with the next number",
                    ));
                }
                let anchor_source = binding.transition().source_anchor_sequence();
                let checkpoint_source = binding.transition().source_checkpoint_sequence();
                let fork_source = binding.transition().source_initial_fork_sequence();
                match (anchor_source, checkpoint_source, fork_source) {
                    (None, None, Some(source)) => {
                        if self.replacement_source
                            != Some(ReplacementSource::InitialFork {
                                sequence: source,
                                epoch: previous,
                            })
                            || self.fork_seed.as_ref().is_none_or(|fork| {
                                fork.sequence != source
                                    || fork.owner_epoch != Some(previous)
                                    || fork.has_accepted_request
                                    || self.latest_checkpoint.is_some()
                            })
                        {
                            return Err(JournalCodecError::new(
                                "replacement initial fork source has no complete current owner chain",
                            ));
                        }
                    },
                    (Some(source), None, None) => {
                        let Some(ReferenceTarget::Anchor {
                            epoch,
                            context_epoch,
                            ..
                        }) = self.reference_targets.get(&source).cloned()
                        else {
                            return Err(JournalCodecError::new(
                                "replacement binding source does not identify a continuation Anchor",
                            ));
                        };
                        let inherited_local_replay_anchor = (binding.continuation_strategy()
                            == (ContinuationStrategy::ExactReplay {
                                executor: crate::ReplayExecutor::LocalClient,
                                replay_profile: ReplayProfile::SemanticOnly,
                            })
                            || self
                                .replay_groups
                                .iter()
                                .any(|group| group.fork_group_index.is_some()))
                            && binding.transition().mode() == TransitionMode::ExactReplay;
                        let backend_native_model_rebind = binding.continuation_strategy()
                            == ContinuationStrategy::BackendManagedState
                            && binding.transition().mode()
                                == TransitionMode::BackendNativeModelRebind;
                        if self.replacement_source
                            != Some(ReplacementSource::Anchor {
                                sequence: source,
                                epoch: previous,
                            })
                            || (epoch != previous
                                && !inherited_local_replay_anchor
                                && !backend_native_model_rebind)
                            || context_epoch != self.context_epoch
                        {
                            return Err(JournalCodecError::new(
                                "replacement binding must use the valid resume Anchor carried by the immediately preceding epoch",
                            ));
                        }
                        if inherited_local_replay_anchor {
                            self.latest_anchor = Some(source);
                        }
                    },
                    (None, Some(source), None) => {
                        let Some(ReferenceTarget::Checkpoint {
                            epoch: _,
                            context_epoch,
                            binding_identity: source_binding_identity,
                            replay_profile: source_replay_profile,
                            has_provider_private,
                        }) = self.reference_targets.get(&source).cloned()
                        else {
                            return Err(JournalCodecError::new(
                                "replacement binding source does not identify a context checkpoint",
                            ));
                        };
                        let target_replay_profile = match binding.continuation_strategy() {
                            ContinuationStrategy::ExactReplay { replay_profile, .. } => {
                                Some(replay_profile)
                            },
                            ContinuationStrategy::BackendManagedState => None,
                        };
                        let private_seed_is_compatible = !has_provider_private
                            || (target_replay_profile == Some(source_replay_profile)
                                && binding.binding_identity() == &source_binding_identity);
                        if binding.transition().mode() != TransitionMode::ExactReplay
                            || self.context_epoch != Some(context_epoch)
                            || !private_seed_is_compatible
                            || self.replacement_source
                                != Some(ReplacementSource::Checkpoint {
                                    sequence: source,
                                    epoch: previous,
                                })
                        {
                            return Err(JournalCodecError::new(
                                "replacement binding must use the newest checkpoint-only reconstruction and preserve every retained private item",
                            ));
                        }
                    },
                    (None, None, None)
                        if binding.transition().mode()
                            == TransitionMode::BackendNativeModelRebind
                            && binding.continuation_strategy()
                                == ContinuationStrategy::BackendManagedState
                            && self.replacement_without_source_allowed => {},
                    _ => {
                        return Err(JournalCodecError::new(
                            "replacement binding requires an eligible Anchor, checkpoint, or source-free native model rebind",
                        ));
                    },
                }
            },
        }
        self.open_epoch = Some(binding.epoch());
        self.open_strategy = Some(binding.continuation_strategy());
        self.open_binding_identity = Some(binding.binding_identity().clone());
        self.open_binding = Some(BackendBindingEvidence::new(
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
        ));
        if binding.continuation_strategy() == ContinuationStrategy::BackendManagedState
            || !matches!(
                binding.transition().mode(),
                TransitionMode::ExactReplay | TransitionMode::InitialFork
            )
        {
            self.model_replay = ModelReplay::default();
            self.model_replay_origins.clear();
            self.replay_contract_rebind_required = false;
        } else {
            self.replay_contract_rebind_required = is_replacement;
        }
        if is_replacement
            && self
                .replay_groups
                .iter()
                .any(|group| group.fork_group_index.is_some())
        {
            self.rebind_fork_groups(sequence, binding.epoch());
        } else if is_replacement {
            self.replay_deltas.clear();
            self.replay_groups.clear();
            if matches!(
                binding.continuation_strategy(),
                ContinuationStrategy::ExactReplay { .. }
            ) && !seed_items.is_empty()
                && let Some(context_epoch) = self.context_epoch
            {
                self.replay_groups.push(ReplayGroup {
                    first_sequence: sequence,
                    last_sequence: sequence,
                    replay_delta_sequence: sequence,
                    epoch: binding.epoch(),
                    context_epoch,
                    items: seed_items,
                    fork_group_index: None,
                    image_losses: seed_image_losses,
                });
            }
        }
        if is_replacement && let Some(fork) = &mut self.fork_seed {
            fork.owner_epoch = Some(binding.epoch());
        }
        self.last_epoch = Some(binding.epoch());
        self.last_close_reason = None;
        self.replacement_source = None;
        self.replacement_without_source_allowed = false;
        self.open_epoch_has_accepted_request = false;
        Ok(())
    }

    pub(super) fn rebind_fork_groups(&mut self, sequence: JournalSequence, epoch: u64) {
        let context_epoch = self
            .context_epoch
            .expect("fork imports have an initialized context epoch");
        let mut local_items = Vec::new();
        let mut local_losses = Vec::new();
        let local_group = |items, image_losses| ReplayGroup {
            first_sequence: sequence,
            last_sequence: sequence,
            replay_delta_sequence: sequence,
            epoch,
            context_epoch,
            items,
            fork_group_index: None,
            image_losses,
        };
        for mut group in mem::take(&mut self.replay_groups) {
            if group.fork_group_index.is_some() {
                if !local_items.is_empty() {
                    self.replay_groups.push(local_group(
                        mem::take(&mut local_items),
                        mem::take(&mut local_losses),
                    ));
                }
                group.epoch = epoch;
                self.replay_groups.push(group);
            } else {
                local_items.extend(group.items);
                local_losses.extend(group.image_losses);
            }
        }
        if !local_items.is_empty() {
            self.replay_groups
                .push(local_group(local_items, local_losses));
        }
        self.replay_deltas.clear();
    }
}

fn fork_binding_matches(binding: &BackendBindingOpened, source: &BackendBindingEvidence) -> bool {
    binding.backend_kind() == source.backend_kind()
        && binding.binding_identity().schema() == source.binding_identity().schema()
        && binding.binding_identity().value() == source.binding_identity().value()
        && binding.model_identity().schema() == source.model_identity().schema()
        && binding.model_identity().value() == source.model_identity().value()
        && binding.continuation_strategy() == source.continuation_strategy()
}
