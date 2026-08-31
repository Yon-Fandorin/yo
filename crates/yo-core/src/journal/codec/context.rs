use serde_json::Value;

use crate::{JournalSequence, ModelReplay, ModelReplayContract, ModelReplayItem, ModelReplayRole};

pub(crate) const CONTEXT_POLICY_PROFILE: &str = "yo.context-policy/v1alpha1";
pub(crate) const CONTEXT_CHECKPOINT_PROFILE: &str = "yo.context-checkpoint/v1alpha1";
pub(crate) const CONTEXT_ARTIFACT_PROFILE: &str = "yo.context-artifact-receipt/v1alpha1";
const MAX_CONTEXT_ITEMS: usize = 4_096;
const MAX_CONTEXT_TEXT_BYTES: usize = 16 * 1024 * 1024;
const MAX_CONTEXT_USAGE_ID_BYTES: usize = 256;
const MAX_CONTEXT_USAGE_ENDPOINT_BYTES: usize = 2_048;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContextStrategy {
    PortableSummaryV1Alpha1,
    ExactReplayOnlyV1Alpha1,
}

impl ContextStrategy {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::PortableSummaryV1Alpha1 => "portable-summary/v1alpha1",
            Self::ExactReplayOnlyV1Alpha1 => "exact-replay-only/v1alpha1",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, &'static str> {
        match value {
            "portable-summary/v1alpha1" => Ok(Self::PortableSummaryV1Alpha1),
            "exact-replay-only/v1alpha1" => Ok(Self::ExactReplayOnlyV1Alpha1),
            _ => Err("context strategy is unsupported"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContextPolicyChanged {
    policy_revision: u64,
    enabled: bool,
    strategy: ContextStrategy,
    warning_percent: u8,
    trigger_percent: u8,
    retained_raw_percent: Option<u8>,
    retained_raw_max_tokens: Option<u64>,
}

impl ContextPolicyChanged {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_new(
        policy_revision: u64,
        enabled: bool,
        strategy: ContextStrategy,
        warning_percent: u8,
        trigger_percent: u8,
        retained_raw_percent: Option<u8>,
        retained_raw_max_tokens: Option<u64>,
    ) -> Result<Self, &'static str> {
        let record = Self {
            policy_revision,
            enabled,
            strategy,
            warning_percent,
            trigger_percent,
            retained_raw_percent,
            retained_raw_max_tokens,
        };
        record.validate()?;
        Ok(record)
    }

    fn validate(&self) -> Result<(), &'static str> {
        if self.policy_revision == 0 {
            return Err("context policy revision must be positive");
        }
        if !(1..=99).contains(&self.warning_percent)
            || !(2..=100).contains(&self.trigger_percent)
            || self.warning_percent >= self.trigger_percent
        {
            return Err("context policy warning and trigger bounds are invalid");
        }
        if self
            .retained_raw_percent
            .is_some_and(|value| !(1..=100).contains(&value))
            || self.retained_raw_max_tokens == Some(0)
        {
            return Err("context policy retained-raw bounds are invalid");
        }
        if self.strategy == ContextStrategy::ExactReplayOnlyV1Alpha1
            && (self.retained_raw_percent.is_some() || self.retained_raw_max_tokens.is_some())
        {
            return Err("exact-replay-only context policy forbids retained-raw bounds");
        }
        Ok(())
    }

    pub(crate) const fn policy_revision(&self) -> u64 {
        self.policy_revision
    }

    pub(crate) const fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) const fn strategy(&self) -> ContextStrategy {
        self.strategy
    }

    pub(crate) const fn warning_percent(&self) -> u8 {
        self.warning_percent
    }

    pub(crate) const fn trigger_percent(&self) -> u8 {
        self.trigger_percent
    }

    pub(crate) const fn retained_raw_percent(&self) -> Option<u8> {
        self.retained_raw_percent
    }

    pub(crate) const fn retained_raw_max_tokens(&self) -> Option<u64> {
        self.retained_raw_max_tokens
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContextRetainedGroup {
    first_sequence: JournalSequence,
    last_sequence: JournalSequence,
    items: Vec<ModelReplayItem>,
}

impl ContextRetainedGroup {
    pub(crate) fn try_new(
        first_sequence: JournalSequence,
        last_sequence: JournalSequence,
        items: Vec<ModelReplayItem>,
    ) -> Result<Self, &'static str> {
        if first_sequence > last_sequence || items.is_empty() {
            return Err("context retained group has an invalid source range or no items");
        }
        Ok(Self {
            first_sequence,
            last_sequence,
            items,
        })
    }

    pub(crate) const fn first_sequence(&self) -> JournalSequence {
        self.first_sequence
    }

    pub(crate) const fn last_sequence(&self) -> JournalSequence {
        self.last_sequence
    }

    pub(crate) fn items(&self) -> &[ModelReplayItem] {
        &self.items
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContextArtifactReceipt {
    content_hash: String,
    byte_count: u64,
    media_kind: String,
    source_context_epoch: u64,
    source_journal_sequence: JournalSequence,
}

impl ContextArtifactReceipt {
    pub(crate) fn try_new(
        content_hash: impl Into<String>,
        byte_count: u64,
        media_kind: impl Into<String>,
        source_context_epoch: u64,
        source_journal_sequence: JournalSequence,
    ) -> Result<Self, &'static str> {
        let record = Self {
            content_hash: content_hash.into(),
            byte_count,
            media_kind: media_kind.into(),
            source_context_epoch,
            source_journal_sequence,
        };
        if !valid_sha256(&record.content_hash)
            || record.byte_count == 0
            || record.source_context_epoch == 0
            || !valid_bounded_ascii(&record.media_kind, 128)
        {
            return Err("context artifact receipt is invalid");
        }
        Ok(record)
    }

    pub(crate) fn content_hash(&self) -> &str {
        &self.content_hash
    }

    pub(crate) const fn byte_count(&self) -> u64 {
        self.byte_count
    }

    pub(crate) fn media_kind(&self) -> &str {
        &self.media_kind
    }

    pub(crate) const fn source_context_epoch(&self) -> u64 {
        self.source_context_epoch
    }

    pub(crate) const fn source_journal_sequence(&self) -> JournalSequence {
        self.source_journal_sequence
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ContextLoss {
    VisiblePrefixSummarized {
        first_sequence: JournalSequence,
        last_sequence: JournalSequence,
    },
    ProviderPrivateDropped {
        schema: String,
        byte_count: u64,
        source_journal_sequence: JournalSequence,
    },
}

impl ContextLoss {
    pub(crate) fn visible_prefix_summarized(
        first_sequence: JournalSequence,
        last_sequence: JournalSequence,
    ) -> Result<Self, &'static str> {
        if first_sequence > last_sequence {
            return Err("summarized context loss range is invalid");
        }
        Ok(Self::VisiblePrefixSummarized {
            first_sequence,
            last_sequence,
        })
    }

    pub(crate) fn provider_private_dropped(
        schema: impl Into<String>,
        byte_count: u64,
        source_journal_sequence: JournalSequence,
    ) -> Result<Self, &'static str> {
        let schema = schema.into();
        if !valid_bounded_ascii(&schema, 128) || byte_count == 0 {
            return Err("provider-private context loss is invalid");
        }
        Ok(Self::ProviderPrivateDropped {
            schema,
            byte_count,
            source_journal_sequence,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContextSummaryUsage(Value);

impl ContextSummaryUsage {
    pub(crate) fn try_new(value: Value) -> Result<Self, &'static str> {
        validate_summary_usage(&value)?;
        Ok(Self(value))
    }

    pub(crate) const fn value(&self) -> &Value {
        &self.0
    }
}

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
        let mut item_count = 1_usize;
        let mut replay_items = vec![ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: self.portable_body.clone(),
            refusal: None,
        }];
        for group in &self.retained_groups {
            if group.last_sequence() > self.source_journal_boundary
                || previous_last.is_some_and(|last| group.first_sequence() <= last)
            {
                return Err(
                    "context checkpoint retained ranges overlap or exceed the source boundary",
                );
            }
            item_count = item_count.saturating_add(group.items().len());
            if item_count > MAX_CONTEXT_ITEMS {
                return Err("context checkpoint replay item bound was exceeded");
            }
            ModelReplay::from_checkpoint(self.replay_contract.clone(), group.items.clone())?;
            replay_items.extend(group.items.iter().cloned());
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
        let items = std::iter::once(ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: self.portable_body.clone(),
            refusal: None,
        })
        .chain(
            self.retained_groups
                .iter()
                .flat_map(|group| group.items.iter().cloned()),
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

fn valid_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

fn valid_bounded_ascii(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && value.is_ascii()
}

fn validate_summary_usage(value: &Value) -> Result<(), &'static str> {
    let object = closed_object(
        value,
        &[
            "schema",
            "response_id",
            "round",
            "provider",
            "account",
            "model",
            "connector",
            "api_dialect",
            "base_url",
            "usage",
            "cache_read_input_tokens",
        ],
    )?;
    if object.get("schema").and_then(Value::as_str) != Some("yo.model-usage-receipt/v1")
        || !object
            .get("round")
            .and_then(Value::as_u64)
            .is_some_and(|round| round > 0)
        || !object
            .get("response_id")
            .and_then(Value::as_str)
            .is_some_and(|value| valid_usage_attribution(value, MAX_CONTEXT_USAGE_ID_BYTES))
        || !object
            .get("provider")
            .and_then(Value::as_str)
            .is_some_and(|value| crate::ProviderId::new(value).is_ok())
        || !object
            .get("account")
            .and_then(Value::as_str)
            .is_some_and(|value| crate::AccountId::new(value).is_ok())
        || !object
            .get("model")
            .and_then(Value::as_str)
            .is_some_and(|value| crate::ModelId::new(value).is_ok())
        || !object
            .get("connector")
            .and_then(Value::as_str)
            .is_some_and(|value| crate::ConnectorId::new(value).is_ok())
        || !object
            .get("api_dialect")
            .and_then(Value::as_str)
            .is_some_and(|value| value.parse::<crate::ApiDialect>().is_ok())
        || !object
            .get("base_url")
            .and_then(Value::as_str)
            .is_some_and(|value| {
                value.len() <= MAX_CONTEXT_USAGE_ENDPOINT_BYTES
                    && crate::NormalizedEndpoint::parse(value)
                        .is_ok_and(|endpoint| endpoint.as_str() == value)
            })
    {
        return Err("context summary usage source is invalid");
    }
    let usage = object
        .get("usage")
        .ok_or("context summary usage is missing usage")?;
    let usage = closed_object(
        usage,
        &[
            "input_tokens",
            "output_tokens",
            "total_tokens",
            "reasoning_tokens",
        ],
    )?;
    let token = |field| {
        usage
            .get(field)
            .and_then(Value::as_u64)
            .ok_or("context summary usage token value is invalid")
    };
    let input_tokens = token("input_tokens")?;
    let output_tokens = token("output_tokens")?;
    let total_tokens = token("total_tokens")?;
    let reasoning_tokens = token("reasoning_tokens")?;
    if input_tokens.checked_add(output_tokens) != Some(total_tokens)
        || reasoning_tokens > output_tokens
    {
        return Err("context summary usage token relationship is invalid");
    }
    let cache = object
        .get("cache_read_input_tokens")
        .ok_or("context summary usage is missing cache availability")?;
    let cache = cache
        .as_object()
        .ok_or("context summary cache availability is invalid")?;
    match cache.get("availability").and_then(Value::as_str) {
        Some("reported") => {
            closed_map(cache, &["availability", "tokens", "source_profile"])?;
            if !cache
                .get("tokens")
                .and_then(Value::as_u64)
                .is_some_and(|tokens| tokens <= input_tokens)
                || !cache
                    .get("source_profile")
                    .and_then(Value::as_str)
                    .is_some_and(|value| crate::VersionedProfileId::new(value).is_ok())
            {
                return Err("reported context summary cache usage is invalid");
            }
        },
        Some("absent") => {
            closed_map(cache, &["availability", "source_profile"])?;
            if !cache
                .get("source_profile")
                .and_then(Value::as_str)
                .is_some_and(|value| crate::VersionedProfileId::new(value).is_ok())
            {
                return Err("absent context summary cache source is invalid");
            }
        },
        Some("unsupported") => closed_map(cache, &["availability"])?,
        _ => return Err("context summary cache availability is unsupported"),
    }
    Ok(())
}

fn valid_usage_attribution(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn closed_object<'a>(
    value: &'a Value,
    fields: &[&str],
) -> Result<&'a serde_json::Map<String, Value>, &'static str> {
    let object = value.as_object().ok_or("context value must be an object")?;
    closed_map(object, fields)?;
    Ok(object)
}

fn closed_map(
    object: &serde_json::Map<String, Value>,
    fields: &[&str],
) -> Result<(), &'static str> {
    if object.keys().any(|key| !fields.contains(&key.as_str())) {
        return Err("context value contains an unknown field");
    }
    Ok(())
}
