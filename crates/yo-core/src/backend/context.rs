use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ModelReplayContract, ModelReplayItem, TurnRef};

const CONTEXT_PRESSURE_SCHEMA: &str = "yo.context-pressure/v1alpha1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextPressureDecision {
    Admit,
    Compact,
    Reject,
}

/// Typed interpretation of one durable context-pressure Activity snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContextPressureObservation {
    input_tokens: u64,
    input_token_limit: u64,
    warning_percent: u8,
    trigger_percent: u8,
    decision: ContextPressureDecision,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ContextPressureWire {
    schema: String,
    input_tokens: u64,
    input_token_limit: u64,
    warning_percent: u8,
    trigger_percent: u8,
    decision: ContextPressureDecision,
}

impl ContextPressureObservation {
    pub fn new(
        input_tokens: u64,
        input_token_limit: u64,
        warning_percent: u8,
        trigger_percent: u8,
        decision: ContextPressureDecision,
    ) -> Result<Self, &'static str> {
        if input_token_limit == 0
            || !(1..=99).contains(&warning_percent)
            || !(2..=100).contains(&trigger_percent)
            || warning_percent >= trigger_percent
        {
            return Err("context pressure observation is invalid");
        }
        Ok(Self {
            input_tokens,
            input_token_limit,
            warning_percent,
            trigger_percent,
            decision,
        })
    }

    pub fn from_snapshot_json(value: &str) -> Option<Self> {
        let wire = serde_json::from_str::<ContextPressureWire>(value).ok()?;
        (wire.schema == CONTEXT_PRESSURE_SCHEMA)
            .then(|| {
                Self::new(
                    wire.input_tokens,
                    wire.input_token_limit,
                    wire.warning_percent,
                    wire.trigger_percent,
                    wire.decision,
                )
                .ok()
            })
            .flatten()
    }

    pub fn to_snapshot_json(self) -> String {
        serde_json::to_string(&ContextPressureWire {
            schema: CONTEXT_PRESSURE_SCHEMA.to_owned(),
            input_tokens: self.input_tokens,
            input_token_limit: self.input_token_limit,
            warning_percent: self.warning_percent,
            trigger_percent: self.trigger_percent,
            decision: self.decision,
        })
        .expect("a bounded context pressure observation is JSON serializable")
    }

    pub const fn input_tokens(self) -> u64 {
        self.input_tokens
    }

    pub const fn input_token_limit(self) -> u64 {
        self.input_token_limit
    }

    pub const fn trigger_percent(self) -> u8 {
        self.trigger_percent
    }

    pub const fn decision(self) -> ContextPressureDecision {
        self.decision
    }
}

/// Sequence-free compaction output proposed by the managed backend.
///
/// `yo-core` binds these exact replay groups to their Journal coordinates and is the only owner
/// allowed to publish the durable checkpoint.
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

#[cfg(test)]
mod tests {
    use super::*;

    // Durable pressure JSON의 생산자와 소비자가 하나의 닫힌 typed 문법을 공유하고
    // unknown field나 잘못된 policy 경계를 조용히 표시하지 않음을 검증합니다.
    #[test]
    fn context_pressure_snapshot_round_trips_only_the_closed_shape() {
        let observation =
            ContextPressureObservation::new(86, 100, 85, 90, ContextPressureDecision::Admit)
                .unwrap();
        let snapshot = observation.to_snapshot_json();

        assert_eq!(
            ContextPressureObservation::from_snapshot_json(&snapshot),
            Some(observation)
        );
        assert!(
            ContextPressureObservation::from_snapshot_json(
                &snapshot.replace("\"input_tokens\":86", "\"extra\":0,\"input_tokens\":86")
            )
            .is_none()
        );
        assert!(
            ContextPressureObservation::new(86, 100, 90, 90, ContextPressureDecision::Admit)
                .is_err()
        );
    }
}
