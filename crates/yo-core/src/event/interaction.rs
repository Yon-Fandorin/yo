use serde::{Deserialize, Serialize};

use super::{OutputEnvelope, tool::ToolOutput};

/// Activity 또는 turn outcome을 바꾸지 않는 presentation severity입니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    /// 정보를 알리는 host observation입니다.
    Info,
    /// 사용자에게 보여야 하는 warning 또는 retry observation입니다.
    Warning,
}

/// ModelWork text snapshot으로 전달하는 명시적 host notice입니다.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityNotice {
    /// host가 제공한 notice heading입니다.
    pub title: String,
    /// 명령이나 Markdown으로 해석하지 않는 literal 설명 text입니다.
    pub message: String,
    /// heading의 semantic palette role입니다.
    pub level: NoticeLevel,
}

impl ActivityNotice {
    /// 정확한 presentation profile discriminator입니다.
    pub const SCHEMA: &str = "yo.activity-notice/v1";

    /// journal record type을 바꾸지 않고 bounded notice를 인코딩합니다.
    pub fn to_snapshot(&self) -> Option<String> {
        if self.title.is_empty() {
            return None;
        }
        let text = serde_json::to_string(&OutputEnvelope {
            schema: Self::SCHEMA.to_owned(),
            output: self.clone(),
        })
        .ok()?;
        (text.len() <= ToolOutput::MAX_SNAPSHOT_BYTES).then_some(text)
    }

    /// 이 정확한 bounded profile과 비어 있지 않은 heading만 허용합니다.
    pub fn from_snapshot(text: &str) -> Option<Self> {
        if text.len() > ToolOutput::MAX_SNAPSHOT_BYTES {
            return None;
        }
        let envelope: OutputEnvelope<Self> = serde_json::from_str(text).ok()?;
        (envelope.schema == Self::SCHEMA && !envelope.output.title.is_empty())
            .then_some(envelope.output)
    }
}

/// 하나의 활성 user-input request에 속한 선택 가능한 choice입니다.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionChoice {
    /// 그대로 보존하는 provider option label입니다.
    pub label: String,
    /// choice와 함께 표시하는 설명 text입니다.
    pub description: String,
}

/// user-input request를 위한 선택적 구조화 presentation이며 request ID가 권한을 유지합니다.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityQuestion {
    /// question 진행과 input hint를 포함한 완전한 읽기용 prompt입니다.
    pub plain_text: String,
    /// 순서가 있는 choice이며 선택하면 one-based ordinal을 제출합니다.
    pub choices: Vec<QuestionChoice>,
    /// host가 typed choice-plus-notes 응답을 지원하는지 나타냅니다.
    #[serde(default)]
    pub allow_notes: bool,
    /// 제출하지 않고 이전 question을 다시 볼 수 있는지 나타냅니다.
    #[serde(default)]
    pub previous_question: bool,
    /// 이전에 보존한 plain-text draft이며 없다고 해서 frontend input을 대체하지 않습니다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<String>,
    /// draft에 연결된 one-based choice이며 notes 지원이 필요합니다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_choice: Option<u32>,
}

impl ActivityQuestion {
    fn valid(&self) -> bool {
        !self.plain_text.is_empty()
            && self.choices.len() <= 64
            && self.draft_choice.is_none_or(|choice| {
                self.allow_notes
                    && self.draft.is_some()
                    && choice > 0
                    && choice as usize <= self.choices.len()
            })
    }

    /// 정확한 presentation profile discriminator입니다.
    pub const SCHEMA: &str = "yo.activity-question/v1";

    /// bounded question을 인코딩하며 더 큰 choice set은 plain-text 경로를 유지합니다.
    pub fn to_snapshot(&self) -> Option<String> {
        if !self.valid() {
            return None;
        }
        let text = serde_json::to_string(&OutputEnvelope {
            schema: Self::SCHEMA.to_owned(),
            output: self.clone(),
        })
        .ok()?;
        (text.len() <= ToolOutput::MAX_SNAPSHOT_BYTES).then_some(text)
    }

    /// response 권한을 부여하지 않고 이 정확한 bounded profile만 허용합니다.
    pub fn from_snapshot(text: &str) -> Option<Self> {
        if text.len() > ToolOutput::MAX_SNAPSHOT_BYTES {
            return None;
        }
        let envelope: OutputEnvelope<Self> = serde_json::from_str(text).ok()?;
        (envelope.schema == Self::SCHEMA && envelope.output.valid()).then_some(envelope.output)
    }
}

/// 원래 ordinal을 보존하는 host 제공 approval decision 하나입니다.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalChoice {
    /// approval 범위를 포함한 표시용 action입니다.
    pub label: String,
    /// 실행 가능한 input이 아닌 literal scope 설명입니다.
    pub description: String,
    /// adapter가 안전하게 해석할 수 없는 decision이면 false입니다.
    pub enabled: bool,
}

/// approval request에 묶인 choice presentation이며 decision 권한은 backend가 가집니다.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityApproval {
    /// 자체 Turn 안의 관찰된 file-change activity ID이며 presentation 전용입니다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_change: Option<u64>,
    /// 읽을 수 있는 완전한 request와 보고된 permission입니다.
    pub plain_text: String,
    /// 원래 제공된 순서이며 response는 one-based ordinal을 사용합니다.
    pub choices: Vec<ApprovalChoice>,
    /// selection과 Escape에서 선호하는 권한 부여 없는 decline/cancel choice입니다.
    pub decline_choice: Option<u32>,
}

impl ActivityApproval {
    /// 정확한 approval presentation discriminator입니다.
    pub const SCHEMA: &str = "yo.activity-approval/v1";

    fn valid(&self) -> bool {
        !self.plain_text.is_empty()
            && self.related_change.is_none_or(|id| id != 0)
            && self.choices.len() <= 64
            && self.choices.iter().all(|choice| !choice.label.is_empty())
            && self.decline_choice.is_none_or(|choice| {
                choice
                    .checked_sub(1)
                    .and_then(|index| self.choices.get(index as usize))
                    .is_some_and(|choice| choice.enabled)
            })
    }

    /// bounded profile을 인코딩하며 일반 approval으로 fallback해 grant하지 않습니다.
    pub fn to_snapshot(&self) -> Option<String> {
        if !self.valid() {
            return None;
        }
        let text = serde_json::to_string(&OutputEnvelope {
            schema: Self::SCHEMA.to_owned(),
            output: self.clone(),
        })
        .ok()?;
        (text.len() <= ToolOutput::MAX_SNAPSHOT_BYTES).then_some(text)
    }

    /// decision을 승인하지 않고 정확하고 유효한 profile만 허용합니다.
    pub fn from_snapshot(text: &str) -> Option<Self> {
        if text.len() > ToolOutput::MAX_SNAPSHOT_BYTES {
            return None;
        }
        let envelope: OutputEnvelope<Self> = serde_json::from_str(text).ok()?;
        (envelope.schema == Self::SCHEMA && envelope.output.valid()).then_some(envelope.output)
    }
}
