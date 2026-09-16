use std::{collections::BTreeSet, fmt::Write as _};

use sha2::{Digest, Sha256};

use super::{
    super::super::{
        ContextArtifactReceipt, ContextCheckpoint, ContextImageLoss, ContextImageSource,
        ContextLoss, ContextPolicyChanged, ContextRetainedGroup, ContextStrategy, ForkSeed,
        JournalCodecError, validate_image_losses,
    },
    CorrelationRecovery,
    model::{ReferenceTarget, ReplayGroup},
};
use crate::{
    ContinuationStrategy, JournalSequence, ModelReplay, ModelReplayItem, ReplayProfile,
    backend::{provider_private_schema, validate_provider_private_replay_sequence},
};

fn artifact_matches(item: &ModelReplayItem, receipt: &ContextArtifactReceipt) -> bool {
    if receipt.media_kind() != "text/plain" {
        return false;
    }
    let matches = |bytes: &[u8]| {
        let digest = Sha256::digest(bytes);
        let mut content_hash = String::from("sha256:");
        for byte in digest {
            write!(&mut content_hash, "{byte:02x}")
                .expect("writing a digest into a String cannot fail");
        }
        u64::try_from(bytes.len()) == Ok(receipt.byte_count())
            && content_hash == receipt.content_hash()
    };
    match item {
        ModelReplayItem::FunctionCallOutput { output, .. } => matches(output.as_bytes()),
        ModelReplayItem::Message { .. }
        | ModelReplayItem::MultimodalUser { .. }
        | ModelReplayItem::FunctionCall { .. }
        | ModelReplayItem::ProviderPrivateAssistant { .. } => false,
    }
}

impl CorrelationRecovery {
    pub(in super::super) const fn context_epoch(&self) -> Option<u64> {
        self.context_epoch
    }

    pub(in super::super) const fn current_policy(&self) -> Option<&ContextPolicyChanged> {
        self.current_policy.as_ref()
    }

    pub(in super::super) fn replay_groups(&self) -> Vec<Vec<ModelReplayItem>> {
        self.replay_groups
            .iter()
            .map(|group| group.items.clone())
            .collect()
    }

    pub(in super::super) const fn model_replay(&self) -> &ModelReplay {
        &self.model_replay
    }

    pub(in super::super) const fn replay_contract_rebind_required(&self) -> bool {
        self.replay_contract_rebind_required
    }

    pub(super) fn observe_context_epoch(
        &mut self,
        record_context_epoch: Option<u64>,
        record_kind: &'static str,
    ) -> Result<(), JournalCodecError> {
        match (self.context_epoch, record_context_epoch) {
            (None, None) => {
                self.saw_legacy_context_record = true;
                Ok(())
            },
            (None, Some(_)) => Err(JournalCodecError::new(format!(
                "{record_kind} declares context_epoch before context policy revision 1"
            ))),
            (Some(_), None) => Err(JournalCodecError::new(format!(
                "{record_kind} omits context_epoch in a current context graph"
            ))),
            (Some(current), Some(record)) if current != record => Err(JournalCodecError::new(
                format!("{record_kind} does not name the current context_epoch"),
            )),
            (Some(_), Some(_)) => Ok(()),
        }
    }

    pub(super) fn observe_checkpoint(
        &mut self,
        sequence: JournalSequence,
        checkpoint: &ContextCheckpoint,
    ) -> Result<(), JournalCodecError> {
        if self.saw_legacy_context_record {
            return Err(JournalCodecError::new(
                "context checkpoint cannot be introduced into a legacy context graph",
            ));
        }
        if self.open_epoch != Some(checkpoint.epoch())
            || !matches!(
                self.open_strategy,
                Some(ContinuationStrategy::ExactReplay {
                    executor: crate::ReplayExecutor::LocalClient,
                    ..
                })
            )
        {
            return Err(JournalCodecError::new(
                "context checkpoint requires its open local-client exact-replay binding",
            ));
        }
        checkpoint
            .validate_profile()
            .map_err(JournalCodecError::new)?;
        let binding = self
            .open_binding
            .as_ref()
            .ok_or_else(|| JournalCodecError::new("checkpoint has no owning binding evidence"))?;
        checkpoint
            .validate_binding_accounting(binding.binding_identity().value())
            .map_err(JournalCodecError::new)?;
        let Some(policy) = &self.current_policy else {
            return Err(JournalCodecError::new(
                "context checkpoint requires a current context policy",
            ));
        };
        if !policy.enabled()
            || policy.policy_revision() != checkpoint.policy_revision()
            || policy.strategy() != checkpoint.strategy()
            || checkpoint.strategy() != ContextStrategy::PortableSummaryV1Alpha1
        {
            return Err(JournalCodecError::new(
                "context checkpoint does not match its enabled portable policy",
            ));
        }
        if self.context_epoch != Some(checkpoint.previous_context_epoch()) {
            return Err(JournalCodecError::new(
                "context checkpoint does not advance the current context_epoch",
            ));
        }
        let Some(ReferenceTarget::Anchor {
            epoch,
            context_epoch,
            journal_boundary,
        }) = self
            .reference_targets
            .get(&checkpoint.source_anchor_sequence())
            .cloned()
        else {
            return Err(JournalCodecError::new(
                "context checkpoint source does not identify an earlier continuation Anchor",
            ));
        };
        if epoch != checkpoint.epoch()
            || context_epoch != Some(checkpoint.previous_context_epoch())
            || journal_boundary > checkpoint.source_journal_boundary()
            || self
                .record_coordinates
                .get(&checkpoint.source_journal_boundary())
                != Some(&(checkpoint.epoch(), checkpoint.previous_context_epoch()))
            || checkpoint.source_journal_boundary() >= sequence
        {
            return Err(JournalCodecError::new(
                "context checkpoint source Anchor or boundary is inconsistent",
            ));
        }
        if self.model_replay.contract() != Some(checkpoint.replay_contract()) {
            return Err(JournalCodecError::new(
                "context checkpoint replay contract differs from its source binding",
            ));
        }
        self.validate_checkpoint_sources(checkpoint)?;
        let retained_items = checkpoint
            .retained_groups()
            .iter()
            .flat_map(|group| group.items());
        let has_provider_private = retained_items
            .clone()
            .any(|item| matches!(item, ModelReplayItem::ProviderPrivateAssistant { .. }));
        match self.open_strategy {
            Some(ContinuationStrategy::ExactReplay {
                replay_profile: ReplayProfile::SemanticOnly,
                ..
            }) if has_provider_private => {
                return Err(JournalCodecError::new(
                    "semantic-only context checkpoint cannot retain provider-private items",
                ));
            },
            Some(ContinuationStrategy::ExactReplay {
                replay_profile: ReplayProfile::ProviderPrivateLocalPlaintext,
                ..
            }) => {
                let items = retained_items.cloned().collect::<Vec<_>>();
                if items.iter().any(|item| {
                    matches!(
                        item,
                        ModelReplayItem::Message {
                            role: crate::ModelReplayRole::Assistant,
                            ..
                        }
                    )
                }) {
                    validate_provider_private_replay_sequence(
                        &items,
                        provider_private_schema(ReplayProfile::ProviderPrivateLocalPlaintext)
                            .expect("the provider-private profile has an exact schema"),
                    )
                    .map_err(JournalCodecError::new)?;
                } else if items
                    .iter()
                    .any(|item| matches!(item, ModelReplayItem::ProviderPrivateAssistant { .. }))
                {
                    return Err(JournalCodecError::new(
                        "provider-private retained replay item has no assistant group",
                    ));
                }
            },
            _ => {},
        }
        let next_origins = self.checkpoint_origins(sequence, checkpoint)?;
        self.model_replay = checkpoint.replay_root().map_err(|detail| {
            JournalCodecError::new(format!(
                "context checkpoint cannot establish its replay root: {detail}"
            ))
        })?;
        self.model_replay_origins = next_origins;
        let root_items = self.model_replay.items().to_vec();
        let retained_image_losses = checkpoint
            .retained_groups()
            .iter()
            .enumerate()
            .map(|(group_index, group)| {
                ContextImageLoss::for_items(
                    group.items(),
                    checkpoint.successor_context_epoch(),
                    |item_index, part_index| ContextImageSource::RetainedCheckpoint {
                        sequence: sequence.get(),
                        group_index: group_index as u32,
                        item_index,
                        part_index,
                    },
                )
                .map_err(JournalCodecError::new)
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.context_epoch = Some(checkpoint.successor_context_epoch());
        self.latest_checkpoint = Some(sequence);
        self.request_after_checkpoint = false;
        self.latest_anchor = None;
        self.replay_deltas.clear();
        self.replay_groups.clear();
        self.replay_groups.push(ReplayGroup {
            first_sequence: sequence,
            last_sequence: sequence,
            replay_delta_sequence: sequence,
            epoch: checkpoint.epoch(),
            context_epoch: checkpoint.successor_context_epoch(),
            items: root_items,
            fork_group_index: None,
            image_losses: retained_image_losses.iter().flatten().cloned().collect(),
        });
        if checkpoint
            .retained_groups()
            .iter()
            .any(|group| group.fork_import().is_some())
        {
            // The synthetic body precedes imports; the local retained tail follows them.
            self.replay_groups[0].items.truncate(1);
            self.replay_groups[0].image_losses.clear();
            let mut local_tail = Vec::new();
            let mut local_losses = Vec::new();
            for (group_index, group) in checkpoint.retained_groups().iter().enumerate() {
                if let Some((seed_sequence, index)) = group.fork_import() {
                    self.replay_groups.push(ReplayGroup {
                        first_sequence: seed_sequence,
                        last_sequence: seed_sequence,
                        replay_delta_sequence: seed_sequence,
                        epoch: checkpoint.epoch(),
                        context_epoch: checkpoint.successor_context_epoch(),
                        items: group.items().to_vec(),
                        fork_group_index: Some(index),
                        image_losses: retained_image_losses[group_index].clone(),
                    });
                } else {
                    local_tail.extend(group.items().iter().cloned());
                    local_losses.extend(retained_image_losses[group_index].iter().cloned());
                }
            }
            if !local_tail.is_empty() {
                self.replay_groups.push(ReplayGroup {
                    first_sequence: sequence,
                    last_sequence: sequence,
                    replay_delta_sequence: sequence,
                    epoch: checkpoint.epoch(),
                    context_epoch: checkpoint.successor_context_epoch(),
                    items: local_tail,
                    fork_group_index: None,
                    image_losses: local_losses,
                });
            }
        }
        Ok(())
    }

    fn validate_checkpoint_sources(
        &self,
        checkpoint: &ContextCheckpoint,
    ) -> Result<(), JournalCodecError> {
        let expected_coordinates = (checkpoint.epoch(), checkpoint.previous_context_epoch());
        let source_groups = self
            .replay_groups
            .iter()
            .enumerate()
            .filter(|(_, group)| group.last_sequence <= checkpoint.source_journal_boundary())
            .collect::<Vec<_>>();
        let mut retained = BTreeSet::new();
        let imported_context = source_groups
            .iter()
            .any(|(_, group)| group.fork_group_index.is_some());
        let mut active_retained = Vec::new();
        for retained_group in checkpoint.retained_groups() {
            if retained_group.fork_import().is_some() {
                self.validate_fork_retained_group(retained_group, checkpoint.epoch())?;
            } else {
                self.validate_source_range(
                    retained_group.first_sequence(),
                    retained_group.last_sequence(),
                    expected_coordinates,
                    "context retained group",
                )?;
            }
            let source_group = source_groups.iter().find(|(index, source_group)| {
                source_group.first_sequence == retained_group.first_sequence()
                    && source_group.last_sequence == retained_group.last_sequence()
                    && source_group.fork_group_index
                        == retained_group.fork_import().map(|(_, index)| index)
                    && source_group.items == retained_group.items()
                    && !retained.contains(index)
            });
            let Some((index, source_group)) = source_group else {
                if retained_group.fork_import().is_none()
                    && retained_group.first_sequence() > checkpoint.source_anchor_sequence()
                    && retained_group.last_sequence() <= checkpoint.source_journal_boundary()
                {
                    if !self.active_input_group_matches(retained_group) {
                        return Err(JournalCodecError::new(
                            "context retained active group is not the exact Journal-backed submitted input",
                        ));
                    }
                    active_retained.push(retained_group);
                    continue;
                }
                return Err(JournalCodecError::new(
                    "context retained range does not identify one whole completed replay group with exact Journal-backed replay",
                ));
            };
            if source_group.epoch != checkpoint.epoch()
                || source_group.context_epoch != checkpoint.previous_context_epoch()
                || source_group.items != retained_group.items()
                || !retained.insert(*index)
            {
                return Err(JournalCodecError::new(
                    "context retained group is not the exact Journal-backed replay group",
                ));
            }
        }

        let mut summarized = BTreeSet::new();
        let mut declared_private_losses = Vec::new();
        let mut declared_image_losses = Vec::new();
        let mut image_losses_started = false;
        for loss in checkpoint.losses() {
            if !matches!(loss, ContextLoss::ImageInputSummarized(_)) && image_losses_started {
                return Err(JournalCodecError::new(
                    "image losses must follow all legacy loss entries",
                ));
            }
            match loss {
                ContextLoss::ImageInputSummarized(loss) => {
                    image_losses_started = true;
                    declared_image_losses.push(loss.clone());
                },
                ContextLoss::VisiblePrefixSummarized {
                    first_sequence,
                    last_sequence,
                } => {
                    if *last_sequence > checkpoint.source_anchor_sequence() {
                        return Err(JournalCodecError::new(
                            "visible summarized range crosses the mandatory active suffix",
                        ));
                    }
                    if !imported_context {
                        self.validate_source_range(
                            *first_sequence,
                            *last_sequence,
                            expected_coordinates,
                            "visible summarized range",
                        )?;
                    }
                    let covered = source_groups
                        .iter()
                        .filter(|(index, group)| {
                            group.first_sequence >= *first_sequence
                                && group.last_sequence <= *last_sequence
                                && (!imported_context || !retained.contains(index))
                        })
                        .collect::<Vec<_>>();
                    if covered.iter().map(|(_, group)| group.first_sequence).min()
                        != Some(*first_sequence)
                        || covered.iter().map(|(_, group)| group.last_sequence).max()
                            != Some(*last_sequence)
                        || covered.iter().any(|(_, group)| {
                            (group.epoch, group.context_epoch) != expected_coordinates
                        })
                    {
                        return Err(JournalCodecError::new(
                            "visible summarized range does not cover whole replay groups",
                        ));
                    }
                    for (index, _) in covered {
                        if retained.contains(index) || !summarized.insert(*index) {
                            return Err(JournalCodecError::new(
                                "context retained and summarized replay groups overlap",
                            ));
                        }
                    }
                },
                ContextLoss::ProviderPrivateDropped {
                    schema,
                    byte_count,
                    source_journal_sequence,
                } => {
                    if self.record_coordinates.get(source_journal_sequence)
                        != Some(&expected_coordinates)
                        && !source_groups.iter().any(|(_, group)| {
                            group.fork_group_index.is_some()
                                && group.replay_delta_sequence == *source_journal_sequence
                                && (group.epoch, group.context_epoch) == expected_coordinates
                        })
                    {
                        return Err(JournalCodecError::new(
                            "provider-private context loss source is outside its binding or context epoch",
                        ));
                    }
                    declared_private_losses.push((
                        schema.clone(),
                        *byte_count,
                        *source_journal_sequence,
                    ));
                },
            }
        }

        if source_groups
            .iter()
            .any(|(index, _)| !retained.contains(index) && !summarized.contains(index))
        {
            return Err(JournalCodecError::new(
                "context checkpoint silently omits a source replay group",
            ));
        }

        for (source_sequence, coordinates) in self
            .record_coordinates
            .range(..=checkpoint.source_journal_boundary())
            .filter(|(sequence, _)| **sequence > checkpoint.source_anchor_sequence())
        {
            if *coordinates != expected_coordinates
                || !active_retained.iter().any(|group| {
                    group.first_sequence() <= *source_sequence
                        && *source_sequence <= group.last_sequence()
                })
            {
                return Err(JournalCodecError::new(
                    "context checkpoint does not retain the complete active semantic suffix",
                ));
            }
        }
        for (source_sequence, input) in self
            .submitted_inputs
            .range(..=checkpoint.source_journal_boundary())
            .filter(|(sequence, _)| **sequence > checkpoint.source_anchor_sequence())
        {
            let retained_input = active_retained.iter().find(|group| {
                group.first_sequence() <= *source_sequence
                    && *source_sequence <= group.last_sequence()
            });
            if !retained_input.is_some_and(|group| group.items().iter().any(|item| item == input)) {
                return Err(JournalCodecError::new(
                    "context checkpoint does not retain an active submitted input",
                ));
            }
        }

        let expected_image_losses = source_groups
            .iter()
            .filter(|(index, _)| summarized.contains(index))
            .flat_map(|(_, group)| group.image_losses.iter().cloned())
            .collect::<Vec<_>>();
        validate_image_losses(&declared_image_losses).map_err(JournalCodecError::new)?;
        if declared_image_losses != expected_image_losses {
            return Err(JournalCodecError::new(
                "checkpoint image losses do not match exact ordered summarized occurrences",
            ));
        }
        let mut expected_private_losses = source_groups
            .iter()
            .filter(|(index, _)| summarized.contains(index))
            .flat_map(|(_, group)| {
                group.items.iter().filter_map(|item| match item {
                    ModelReplayItem::ProviderPrivateAssistant { envelope } => Some((
                        envelope.schema().to_owned(),
                        u64::try_from(envelope.payload().len())
                            .expect("provider-private replay byte bounds fit u64"),
                        group.replay_delta_sequence,
                    )),
                    _ => None,
                })
            })
            .collect::<Vec<_>>();
        expected_private_losses.sort();
        declared_private_losses.sort();
        if expected_private_losses != declared_private_losses {
            return Err(JournalCodecError::new(
                "provider-private loss disclosure does not exactly cover summarized private replay",
            ));
        }

        let mut artifact_identities = BTreeSet::new();
        for receipt in checkpoint.artifact_receipts() {
            if !source_groups.iter().any(|(group_index, group)| {
                group.replay_delta_sequence == receipt.source_journal_sequence()
                    && summarized.contains(group_index)
                    && group
                        .items
                        .iter()
                        .any(|item| artifact_matches(item, receipt))
            }) || !artifact_identities.insert((
                receipt.source_journal_sequence(),
                receipt.content_hash().to_owned(),
                receipt.byte_count(),
                receipt.media_kind().to_owned(),
            )) {
                return Err(JournalCodecError::new(
                    "context artifact receipt is duplicated or not bound to visible summarized replay",
                ));
            }
        }
        Ok(())
    }

    fn validate_fork_retained_group(
        &self,
        group: &ContextRetainedGroup,
        epoch: u64,
    ) -> Result<(), JournalCodecError> {
        let (seed_sequence, index) = group
            .fork_import()
            .expect("imported group checked by caller");
        let fork = self
            .fork_seed
            .as_ref()
            .filter(|fork| fork.sequence == seed_sequence && fork.owner_epoch == Some(epoch))
            .ok_or_else(|| {
                JournalCodecError::new("imported retained group has no current fork owner")
            })?;
        let ForkSeed::ExactReplay(replay) = fork.seed.seed() else {
            return Err(JournalCodecError::new(
                "imported retained group requires an exact seed",
            ));
        };
        let source = replay.groups().get(index).ok_or_else(|| {
            JournalCodecError::new("imported retained group index is outside its seed")
        })?;
        let items = &replay.items()[source.first_item()..source.end_item()];
        let private_epochs = items
            .iter()
            .zip(&replay.item_origins()[source.first_item()..source.end_item()])
            .filter_map(|(item, origin)| {
                matches!(item, ModelReplayItem::ProviderPrivateAssistant { .. })
                    .then_some(origin.original().binding_epoch())
            })
            .collect::<Vec<_>>();
        if items != group.items() || private_epochs != group.private_epochs() {
            return Err(JournalCodecError::new(
                "imported retained group changes seed items or original private epochs",
            ));
        }
        Ok(())
    }

    pub(super) fn active_input_group_matches(&self, group: &ContextRetainedGroup) -> bool {
        let Some(input) = self.submitted_inputs.get(&group.first_sequence()) else {
            return false;
        };
        if group.items().first() != Some(input)
            || self
                .submitted_inputs
                .range(group.first_sequence()..=group.last_sequence())
                .count()
                != 1
        {
            return false;
        }
        if group.first_sequence() == group.last_sequence() {
            return group.items().len() == 1;
        }
        self.completed_activity_boundaries
            .contains(&group.last_sequence())
            && group
                .items()
                .iter()
                .any(|item| matches!(item, ModelReplayItem::FunctionCall { .. }))
            && group
                .items()
                .iter()
                .any(|item| matches!(item, ModelReplayItem::FunctionCallOutput { .. }))
    }

    pub(super) fn validate_source_range(
        &self,
        first: JournalSequence,
        last: JournalSequence,
        expected: (u64, u64),
        label: &str,
    ) -> Result<(), JournalCodecError> {
        if first > last
            || self.record_coordinates.get(&first) != Some(&expected)
            || self.record_coordinates.get(&last) != Some(&expected)
            || self
                .record_coordinates
                .range(first..=last)
                .any(|(_, coordinates)| *coordinates != expected)
        {
            return Err(JournalCodecError::new(format!(
                "{label} crosses its binding or context epoch"
            )));
        }
        Ok(())
    }
}
