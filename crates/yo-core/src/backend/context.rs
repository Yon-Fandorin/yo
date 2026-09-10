use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ModelReplayContract, ModelReplayItem, TurnRef, VersionedProfileId};

const CONTEXT_PRESSURE_SCHEMA: &str = "yo.context-pressure/v1alpha1";
const IMAGE_CONTEXT_PRESSURE_SCHEMA: &str = "yo.context-pressure/v2alpha1";
/// Reviewed advisory accounting identity, separate from input-image capability.
pub const KIMI_CODE_IMAGE_ACCOUNTING_PROFILE: &str = "kimi-code-image-advisory/v1";

/// Strength of a complete request's planning estimate.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextAccountingQuality {
    Exact,
    VerifiedUpperBound,
    AdvisoryEstimate,
}

/// Complete request planning evidence; neither count is measured provider usage.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContextAccounting {
    quality: ContextAccountingQuality,
    policy: String,
    input_estimate: u64,
    reserve_tokens: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountingWire {
    quality: ContextAccountingQuality,
    policy: String,
    input_estimate: u64,
    reserve_tokens: u64,
}

impl<'de> Deserialize<'de> for ContextAccounting {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let wire = AccountingWire::deserialize(decoder)?;
        Self::new(
            wire.quality,
            VersionedProfileId::new(wire.policy).map_err(serde::de::Error::custom)?,
            wire.input_estimate,
            wire.reserve_tokens,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl ContextAccounting {
    /// Validates the admitted policy, compatible quality and checked planning sum.
    pub fn new(
        quality: ContextAccountingQuality,
        policy: VersionedProfileId,
        input_estimate: u64,
        reserve_tokens: u64,
    ) -> Result<Self, &'static str> {
        if policy.as_str() != KIMI_CODE_IMAGE_ACCOUNTING_PROFILE
            || quality != ContextAccountingQuality::AdvisoryEstimate
            || !matches!(reserve_tokens, 0 | 1024)
            || input_estimate.checked_add(reserve_tokens).is_none()
        {
            return Err("context accounting policy, quality, reserve or planning sum is invalid");
        }
        Ok(Self {
            quality,
            policy: policy.as_str().to_owned(),
            input_estimate,
            reserve_tokens,
        })
    }
    /// Explicit quality of this estimate.
    pub const fn quality(&self) -> ContextAccountingQuality {
        self.quality
    }
    /// Admitted versioned accounting identity.
    pub fn policy(&self) -> &str {
        &self.policy
    }
    /// Complete request estimate before the once-per-request reserve.
    pub const fn input_estimate(&self) -> u64 {
        self.input_estimate
    }
    /// Reserve applied once to the complete request.
    pub const fn reserve_tokens(&self) -> u64 {
        self.reserve_tokens
    }
    /// Checked estimate plus reserve used for planning admission.
    pub const fn planning_tokens(&self) -> u64 {
        self.input_estimate + self.reserve_tokens
    }
}

fn optional_present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    decoder: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(decoder).map(Some)
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextPressureDecision {
    Admit,
    Compact,
    Reject,
}

/// Typed interpretation of one durable context-pressure Activity snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextPressureObservation {
    input_tokens: u64,
    input_token_limit: u64,
    warning_percent: u8,
    trigger_percent: u8,
    decision: ContextPressureDecision,
    accounting: Option<ContextAccounting>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ContextPressureWire {
    schema: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_present"
    )]
    input_tokens: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_present"
    )]
    accounting: Option<ContextAccounting>,
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
            accounting: None,
        })
    }

    pub fn from_snapshot_json(value: &str) -> Option<Self> {
        let wire = serde_json::from_str::<ContextPressureWire>(value).ok()?;
        let input_tokens = match (&*wire.schema, wire.input_tokens, &wire.accounting) {
            (CONTEXT_PRESSURE_SCHEMA, Some(tokens), None) => tokens,
            (IMAGE_CONTEXT_PRESSURE_SCHEMA, None, Some(accounting)) => accounting.planning_tokens(),
            _ => return None,
        };
        let mut observation = Self::new(
            input_tokens,
            wire.input_token_limit,
            wire.warning_percent,
            wire.trigger_percent,
            wire.decision,
        )
        .ok()?;
        observation.accounting = wire.accounting;
        Some(observation)
    }

    /// Preserves explicit accounting even when the binding currently carries no images.
    pub fn with_accounting(mut self, accounting: ContextAccounting) -> Result<Self, &'static str> {
        if self.input_tokens != accounting.planning_tokens() {
            return Err("pressure planning total differs from its accounting");
        }
        self.accounting = Some(accounting);
        Ok(self)
    }

    /// Optional image-aware accounting, absent only for the legacy exact scalar profile.
    pub const fn accounting(&self) -> Option<&ContextAccounting> {
        self.accounting.as_ref()
    }

    pub fn to_snapshot_json(&self) -> String {
        serde_json::to_string(&ContextPressureWire {
            schema: if self.accounting.is_some() {
                IMAGE_CONTEXT_PRESSURE_SCHEMA
            } else {
                CONTEXT_PRESSURE_SCHEMA
            }
            .to_owned(),
            input_tokens: self.accounting.is_none().then_some(self.input_tokens),
            accounting: self.accounting.clone(),
            input_token_limit: self.input_token_limit,
            warning_percent: self.warning_percent,
            trigger_percent: self.trigger_percent,
            decision: self.decision,
        })
        .expect("a bounded context pressure observation is JSON serializable")
    }

    pub const fn input_tokens(&self) -> u64 {
        self.input_tokens
    }

    pub const fn input_token_limit(&self) -> u64 {
        self.input_token_limit
    }

    pub const fn trigger_percent(&self) -> u8 {
        self.trigger_percent
    }

    pub const fn decision(&self) -> ContextPressureDecision {
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

    /// Binds before/after planning evidence under one admitted accounting policy.
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
    /// Before and after complete-request planning evidence for image-aware bindings.
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
    // 이미지가 0개라도 advisory policy와 reserve 0을 v2에 보존하고 legacy scalar를 섞지 않는다.
    #[test]
    fn image_pressure_preserves_advisory_metadata_even_without_images() {
        let accounting = ContextAccounting::new(
            ContextAccountingQuality::AdvisoryEstimate,
            VersionedProfileId::new(KIMI_CODE_IMAGE_ACCOUNTING_PROFILE).unwrap(),
            86,
            0,
        )
        .unwrap();
        let pressure =
            ContextPressureObservation::new(86, 100, 85, 90, ContextPressureDecision::Admit)
                .unwrap()
                .with_accounting(accounting)
                .unwrap();
        let wire = pressure.to_snapshot_json();
        assert_eq!(
            wire,
            r#"{"schema":"yo.context-pressure/v2alpha1","accounting":{"quality":"advisory_estimate","policy":"kimi-code-image-advisory/v1","input_estimate":86,"reserve_tokens":0},"input_token_limit":100,"warning_percent":85,"trigger_percent":90,"decision":"admit"}"#
        );
        assert_eq!(
            ContextPressureObservation::from_snapshot_json(&wire),
            Some(pressure)
        );
        for malformed in [
            wire.replace("\"accounting\":", "\"input_tokens\":86,\"accounting\":"),
            wire.replace("advisory_estimate", "exact"),
            wire.replace("\"reserve_tokens\":0", "\"reserve_tokens\":null"),
            wire.replace(
                "\"quality\":",
                "\"quality\":\"advisory_estimate\",\"quality\":",
            ),
        ] {
            assert!(ContextPressureObservation::from_snapshot_json(&malformed).is_none());
        }
    }

    // 완전한 요청 planning 합의 overflow·미승인 policy·호환되지 않는 quality는 거절한다.
    #[test]
    fn image_accounting_rejects_unknown_policy_quality_and_overflow() {
        let policy = || VersionedProfileId::new(KIMI_CODE_IMAGE_ACCOUNTING_PROFILE).unwrap();
        assert!(
            ContextAccounting::new(
                ContextAccountingQuality::AdvisoryEstimate,
                policy(),
                u64::MAX,
                1024
            )
            .is_err()
        );
        assert!(ContextAccounting::new(ContextAccountingQuality::Exact, policy(), 1, 0).is_err());
        assert!(
            ContextAccounting::new(ContextAccountingQuality::VerifiedUpperBound, policy(), 1, 0)
                .is_err()
        );
        assert!(
            ContextAccounting::new(
                ContextAccountingQuality::AdvisoryEstimate,
                VersionedProfileId::new("unknown/v1").unwrap(),
                1,
                0
            )
            .is_err()
        );
        assert!(
            ContextAccounting::new(
                ContextAccountingQuality::AdvisoryEstimate,
                policy(),
                u64::MAX,
                0
            )
            .is_ok()
        );
    }
}
