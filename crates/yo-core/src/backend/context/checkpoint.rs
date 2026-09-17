use serde_json::Value;

use super::accounting::ContextAccounting;
use crate::{ModelReplayContract, ModelReplayItem, TurnRef};

/// managed backend가 제안하는 sequence-free compaction output입니다.
///
/// `yo-core`는 이 replay groups를 정확한 Journal coordinates에 결합하며 durable checkpoint를
/// publish할 수 있는 유일한 owner입니다.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextCheckpointProposal {
    turn: Option<TurnRef>,
    policy_revision: u64,
    input_token_limit: u64,
    input_tokens_before: u64,
    input_tokens_after: u64,
    replay_contract: ModelReplayContract,
    portable_body: String,
    summarized_groups: Vec<Vec<ModelReplayItem>>,
    retained_groups: Vec<Vec<ModelReplayItem>>,
    active_group: Vec<ModelReplayItem>,
    summary_usage: Value,
    accounting: Option<(ContextAccounting, ContextAccounting)>,
}

impl ContextCheckpointProposal {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        turn: Option<TurnRef>,
        policy_revision: u64,
        input_token_limit: u64,
        input_tokens_before: u64,
        input_tokens_after: u64,
        replay_contract: ModelReplayContract,
        portable_body: impl Into<String>,
        summarized_groups: Vec<Vec<ModelReplayItem>>,
        retained_groups: Vec<Vec<ModelReplayItem>>,
        active_group: Vec<ModelReplayItem>,
        summary_usage: Value,
    ) -> Result<Self, &'static str> {
        let proposal = Self {
            turn,
            policy_revision,
            input_token_limit,
            input_tokens_before,
            input_tokens_after,
            replay_contract,
            portable_body: portable_body.into(),
            summarized_groups,
            retained_groups,
            active_group,
            summary_usage,
            accounting: None,
        };
        if proposal.policy_revision == 0
            || proposal.input_token_limit == 0
            || proposal.input_tokens_after >= proposal.input_tokens_before
            || proposal.portable_body.is_empty()
            || proposal.portable_body.len() > 16 * 1024 * 1024
            || proposal.summarized_groups.is_empty()
            || proposal.retained_groups.iter().any(Vec::is_empty)
            || proposal.active_group.is_empty() != proposal.turn.is_none()
            || !proposal.replay_contract.is_valid()
            || !proposal.summary_usage.is_object()
        {
            return Err("context checkpoint proposal is invalid or incomplete");
        }
        Ok(proposal)
    }

    /// 하나의 승인된 accounting policy 아래 before/after planning 근거를 결합합니다.
    pub fn with_accounting(
        mut self,
        before: ContextAccounting,
        after: ContextAccounting,
    ) -> Result<Self, &'static str> {
        if before.policy() != after.policy()
            || before.planning_tokens() != self.input_tokens_before
            || after.planning_tokens() != self.input_tokens_after
        {
            return Err(
                "checkpoint proposal accounting differs from its planning totals or policy",
            );
        }
        self.accounting = Some((before, after));
        Ok(self)
    }

    /// image-aware binding의 before/after complete-request planning 근거입니다.
    pub const fn accounting(&self) -> Option<&(ContextAccounting, ContextAccounting)> {
        self.accounting.as_ref()
    }

    pub const fn turn(&self) -> Option<TurnRef> {
        self.turn
    }

    pub const fn policy_revision(&self) -> u64 {
        self.policy_revision
    }

    pub const fn input_token_limit(&self) -> u64 {
        self.input_token_limit
    }

    pub const fn input_tokens_before(&self) -> u64 {
        self.input_tokens_before
    }

    pub const fn input_tokens_after(&self) -> u64 {
        self.input_tokens_after
    }

    pub const fn replay_contract(&self) -> &ModelReplayContract {
        &self.replay_contract
    }

    pub fn portable_body(&self) -> &str {
        &self.portable_body
    }

    pub fn summarized_groups(&self) -> &[Vec<ModelReplayItem>] {
        &self.summarized_groups
    }

    pub fn retained_groups(&self) -> &[Vec<ModelReplayItem>] {
        &self.retained_groups
    }

    pub fn active_group(&self) -> &[ModelReplayItem] {
        &self.active_group
    }

    pub const fn summary_usage(&self) -> &Value {
        &self.summary_usage
    }
}
