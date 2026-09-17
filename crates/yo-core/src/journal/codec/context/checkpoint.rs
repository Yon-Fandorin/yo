use std::iter;

use super::{
    accounting::ContextSummaryUsage,
    policy::ContextStrategy,
    retention::{
        ContextArtifactReceipt, ContextLoss, ContextRetainedGroup, MAX_CONTEXT_ITEMS,
        validate_image_losses,
    },
};
use crate::{
    ContextAccounting, JournalSequence, ModelReplay, ModelReplayContract, ModelReplayItem,
    ModelReplayRole,
};

pub(crate) const CONTEXT_CHECKPOINT_PROFILE: &str = "yo.context-checkpoint/v1alpha1";
pub(crate) const IMAGE_CONTEXT_CHECKPOINT_PROFILE: &str = "yo.context-checkpoint/v2alpha1";
const MAX_CONTEXT_TEXT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContextCheckpoint {
    epoch: u64,
    previous_context_epoch: u64,
    successor_context_epoch: u64,
    source_anchor_sequence: JournalSequence,
    source_journal_boundary: JournalSequence,
    policy_revision: u64,
    strategy: ContextStrategy,
    input_token_limit: u64,
    input_tokens_before: u64,
    input_tokens_after: u64,
    replay_contract: ModelReplayContract,
    portable_body: String,
    retained_groups: Vec<ContextRetainedGroup>,
    first_retained_sequence: Option<JournalSequence>,
    artifact_receipts: Vec<ContextArtifactReceipt>,
    losses: Vec<ContextLoss>,
    summary_usage: ContextSummaryUsage,
    accounting: Option<(ContextAccounting, ContextAccounting)>,
}
impl ContextCheckpoint {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_new(
        epoch: u64,
        previous_context_epoch: u64,
        successor_context_epoch: u64,
        source_anchor_sequence: JournalSequence,
        source_journal_boundary: JournalSequence,
        policy_revision: u64,
        strategy: ContextStrategy,
        input_token_limit: u64,
        input_tokens_before: u64,
        input_tokens_after: u64,
        replay_contract: ModelReplayContract,
        portable_body: impl Into<String>,
        retained_groups: Vec<ContextRetainedGroup>,
        first_retained_sequence: Option<JournalSequence>,
        artifact_receipts: Vec<ContextArtifactReceipt>,
        losses: Vec<ContextLoss>,
        summary_usage: ContextSummaryUsage,
    ) -> Result<Self, &'static str> {
        let record = Self {
            epoch,
            previous_context_epoch,
            successor_context_epoch,
            source_anchor_sequence,
            source_journal_boundary,
            policy_revision,
            strategy,
            input_token_limit,
            input_tokens_before,
            input_tokens_after,
            replay_contract,
            portable_body: portable_body.into(),
            retained_groups,
            first_retained_sequence,
            artifact_receipts,
            losses,
            summary_usage,
            accounting: None,
        };
        record.validate()?;
        Ok(record)
    }

    fn validate(&self) -> Result<(), &'static str> {
        if self.epoch == 0
            || self.previous_context_epoch == 0
            || self.successor_context_epoch
                != self.previous_context_epoch.checked_add(1).unwrap_or(0)
            || self.policy_revision == 0
            || self.input_token_limit == 0
            || self.strategy != ContextStrategy::PortableSummaryV1Alpha1
        {
            return Err("context checkpoint scalar fields are invalid");
        }
        if self.portable_body.len() > MAX_CONTEXT_TEXT_BYTES
            || !valid_portable_body(&self.portable_body)
        {
            return Err("context checkpoint portable body is malformed or oversized");
        }
        if !self.replay_contract.is_valid() {
            return Err("context checkpoint replay contract is invalid");
        }
        if self.retained_groups.len() > MAX_CONTEXT_ITEMS
            || self.artifact_receipts.len() > MAX_CONTEXT_ITEMS
            || self.losses.len() > MAX_CONTEXT_ITEMS
        {
            return Err("context checkpoint collection bound was exceeded");
        }
        let expected_first = self
            .retained_groups
            .first()
            .map(ContextRetainedGroup::first_sequence);
        if self.first_retained_sequence != expected_first {
            return Err("context checkpoint first retained sequence is inconsistent");
        }
        let mut previous_last = None;
        let mut previous_import = None;
        let mut local_seen = false;
        let mut item_count = 1_usize;
        let mut replay_items = vec![ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: self.portable_body.clone(),
            refusal: None,
        }];
        for group in &self.retained_groups {
            if group.last_sequence() > self.source_journal_boundary {
                return Err(
                    "context checkpoint retained ranges overlap or exceed the source boundary",
                );
            }
            if let Some((seed, index)) = group.fork_import() {
                if local_seen
                    || previous_import.is_some_and(|(previous_seed, previous_index)| {
                        seed != previous_seed || index <= previous_index
                    })
                {
                    return Err(
                        "imported checkpoint groups must precede local groups with increasing indexes from one seed",
                    );
                }
                previous_import = Some((seed, index));
            } else {
                if previous_last.is_some_and(|last| group.first_sequence() <= last) {
                    return Err(
                        "context checkpoint retained ranges overlap or exceed the source boundary",
                    );
                }
                local_seen = true;
            }
            item_count = item_count.saturating_add(group.items().len());
            if item_count > MAX_CONTEXT_ITEMS {
                return Err("context checkpoint replay item bound was exceeded");
            }
            ModelReplay::from_checkpoint(self.replay_contract.clone(), group.items().to_vec())?;
            replay_items.extend(group.items().iter().cloned());
            previous_last = Some(group.last_sequence());
        }
        ModelReplay::from_checkpoint(self.replay_contract.clone(), replay_items)?;
        for receipt in &self.artifact_receipts {
            if receipt.source_context_epoch() != self.previous_context_epoch
                || receipt.source_journal_sequence() > self.source_journal_boundary
            {
                return Err("context artifact receipt is outside the checkpoint source boundary");
            }
        }
        for loss in &self.losses {
            match loss {
                ContextLoss::VisiblePrefixSummarized {
                    first_sequence,
                    last_sequence,
                } if first_sequence > last_sequence
                    || *last_sequence > self.source_journal_boundary =>
                {
                    return Err("context loss range is outside the checkpoint source boundary");
                },
                ContextLoss::ProviderPrivateDropped {
                    source_journal_sequence,
                    ..
                } if *source_journal_sequence > self.source_journal_boundary => {
                    return Err("provider-private loss is outside the checkpoint source boundary");
                },
                _ => {},
            }
        }
        Ok(())
    }

    pub(crate) fn with_accounting(
        mut self,
        before: ContextAccounting,
        after: ContextAccounting,
    ) -> Result<Self, &'static str> {
        if before.policy() != after.policy()
            || before.planning_tokens() != self.input_tokens_before
            || after.planning_tokens() != self.input_tokens_after
            || self.input_tokens_after >= self.input_tokens_before
        {
            return Err("checkpoint accounting does not match its planning totals");
        }
        self.accounting = Some((before, after));
        self.validate_profile()?;
        Ok(self)
    }
    pub(crate) const fn accounting(&self) -> Option<&(ContextAccounting, ContextAccounting)> {
        self.accounting.as_ref()
    }
    pub(crate) fn validate_binding_accounting(
        &self,
        binding_value: &str,
    ) -> Result<(), &'static str> {
        let binding = crate::CompleteModelBinding::from_durable_json(binding_value).ok();
        let accounting_policy = binding
            .as_ref()
            .and_then(|binding| binding.profile().image_accounting_policy());
        match (accounting_policy, self.accounting()) {
            (None, None) => Ok(()),
            (Some(profile), Some((before, after)))
                if before.policy() == profile.as_str()
                    && before.policy() == after.policy()
                    && binding.as_ref().is_some_and(|binding| {
                        binding.profile().context().input_token_limit() == self.input_token_limit
                    }) =>
            {
                Ok(())
            },
            _ => Err("checkpoint accounting profile differs from its owning binding epoch"),
        }
    }
    pub(crate) fn profile(&self) -> &'static str {
        if self.accounting.is_some() {
            IMAGE_CONTEXT_CHECKPOINT_PROFILE
        } else {
            CONTEXT_CHECKPOINT_PROFILE
        }
    }
    pub(crate) fn validate_profile(&self) -> Result<(), &'static str> {
        let image_losses = self
            .losses
            .iter()
            .filter_map(|loss| match loss {
                ContextLoss::ImageInputSummarized(loss) => Some(loss.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        if self.accounting.is_none()
            && (!image_losses.is_empty()
                || self
                    .retained_groups
                    .iter()
                    .flat_map(|group| group.items())
                    .any(|item| matches!(item, ModelReplayItem::MultimodalUser { .. })))
        {
            return Err("legacy checkpoint profile cannot carry image snapshots or losses");
        }
        if image_losses
            .iter()
            .any(|loss| loss.source().sequence() > self.source_journal_boundary.get())
        {
            return Err("image loss source is outside checkpoint boundary");
        }
        validate_image_losses(&image_losses)?;
        if let Some((before, after)) = &self.accounting {
            let retained_images = self
                .retained_groups
                .iter()
                .flat_map(|group| group.items())
                .any(|item| matches!(item, ModelReplayItem::MultimodalUser { .. }));
            if after.reserve_tokens() != if retained_images { 1024 } else { 0 }
                || before.reserve_tokens()
                    != if retained_images || !image_losses.is_empty() {
                        1024
                    } else {
                        0
                    }
            {
                return Err(
                    "checkpoint accounting reserve does not match complete image occurrence presence",
                );
            }
        }
        Ok(())
    }

    pub(crate) const fn epoch(&self) -> u64 {
        self.epoch
    }
    pub(crate) const fn previous_context_epoch(&self) -> u64 {
        self.previous_context_epoch
    }
    pub(crate) const fn successor_context_epoch(&self) -> u64 {
        self.successor_context_epoch
    }
    pub(crate) const fn source_anchor_sequence(&self) -> JournalSequence {
        self.source_anchor_sequence
    }
    pub(crate) const fn source_journal_boundary(&self) -> JournalSequence {
        self.source_journal_boundary
    }
    pub(crate) const fn policy_revision(&self) -> u64 {
        self.policy_revision
    }
    pub(crate) const fn strategy(&self) -> ContextStrategy {
        self.strategy
    }
    pub(crate) const fn input_token_limit(&self) -> u64 {
        self.input_token_limit
    }
    pub(crate) const fn input_tokens_before(&self) -> u64 {
        self.input_tokens_before
    }
    pub(crate) const fn input_tokens_after(&self) -> u64 {
        self.input_tokens_after
    }
    pub(crate) const fn replay_contract(&self) -> &ModelReplayContract {
        &self.replay_contract
    }
    pub(crate) fn portable_body(&self) -> &str {
        &self.portable_body
    }
    pub(crate) fn retained_groups(&self) -> &[ContextRetainedGroup] {
        &self.retained_groups
    }
    pub(crate) const fn first_retained_sequence(&self) -> Option<JournalSequence> {
        self.first_retained_sequence
    }
    pub(crate) fn artifact_receipts(&self) -> &[ContextArtifactReceipt] {
        &self.artifact_receipts
    }
    pub(crate) fn losses(&self) -> &[ContextLoss] {
        &self.losses
    }
    pub(crate) const fn summary_usage(&self) -> &ContextSummaryUsage {
        &self.summary_usage
    }

    pub(crate) fn replay_root(&self) -> Result<ModelReplay, &'static str> {
        let items = iter::once(ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: self.portable_body.clone(),
            refusal: None,
        })
        .chain(
            self.retained_groups
                .iter()
                .flat_map(|group| group.items().iter().cloned()),
        )
        .collect();
        ModelReplay::from_checkpoint(self.replay_contract.clone(), items)
    }
}

fn valid_portable_body(body: &str) -> bool {
    const HEADINGS: [&str; 9] = [
        "# Context Checkpoint",
        "## Current Objective",
        "## Active Constraints",
        "## Decisions",
        "## Verified Progress",
        "## Current State",
        "## Unknown or Unverified",
        "## Next Actions",
        "## Critical References",
    ];
    let mut heading_index = 0_usize;
    let mut section_has_content = false;
    for line in body.lines() {
        if line.starts_with('#') {
            if heading_index > 1 && !section_has_content {
                return false;
            }
            if HEADINGS.get(heading_index) != Some(&line) {
                return false;
            }
            heading_index += 1;
            section_has_content = false;
        } else if !line.trim().is_empty() {
            if heading_index >= 2 {
                section_has_content = true;
            } else {
                return false;
            }
        }
    }
    heading_index == HEADINGS.len() && section_has_content
}
