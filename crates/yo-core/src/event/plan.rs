use serde::{Deserialize, Serialize};

use super::{OutputEnvelope, tool::ToolOutput};

/// host가 소유한 plan step 하나의 관찰된 status입니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStatus {
    /// 아직 시작하지 않았습니다.
    Pending,
    /// 현재 작업 중입니다.
    InProgress,
    /// host가 완료를 보고했습니다.
    Completed,
}

/// 명령이나 Markdown으로 해석하지 않는 literal plan step입니다.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanStep {
    /// host가 제공한 step text입니다.
    pub text: String,
    /// host가 보고한 진행 상태입니다.
    pub status: PlanStepStatus,
}

/// 기존 ModelWork text snapshot으로 전달하는 mutable host plan입니다.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityPlan {
    /// plan update에 대한 선택적 literal 설명입니다.
    pub explanation: Option<String>,
    /// authoritative source 순서의 step이며 빈 plan도 명시적으로 유지합니다.
    pub steps: Vec<PlanStep>,
}

impl ActivityPlan {
    /// presentation profile의 정확한 discriminator이며 journal record variant를 바꾸지 않습니다.
    pub const SCHEMA: &str = "yo.activity-plan/v1";

    /// Activity 완료에서 status를 추론하지 않고 bounded snapshot을 인코딩합니다.
    pub fn to_snapshot(&self) -> Option<String> {
        let text = serde_json::to_string(&OutputEnvelope {
            schema: Self::SCHEMA.to_owned(),
            output: self.clone(),
        })
        .ok()?;
        (text.len() <= ToolOutput::MAX_SNAPSHOT_BYTES).then_some(text)
    }

    /// 정확한 bounded profile과 알려진 status 및 field만 허용합니다.
    pub fn from_snapshot(text: &str) -> Option<Self> {
        if text.len() > ToolOutput::MAX_SNAPSHOT_BYTES {
            return None;
        }
        let envelope: OutputEnvelope<Self> = serde_json::from_str(text).ok()?;
        (envelope.schema == Self::SCHEMA).then_some(envelope.output)
    }
}
