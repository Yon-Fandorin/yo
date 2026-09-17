use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OutputEnvelope, tool::ToolOutput};

/// 명시적으로 제공된 conversation summary의 origin입니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryKind {
    /// host가 context를 compact했습니다.
    Compaction,
    /// host가 branch를 summary했습니다.
    Branch,
    /// host가 명시적으로 노출한 public reasoning summary입니다.
    Reasoning,
}

/// host가 제공한 summary이며 presentation은 compaction을 실행하거나 branch를 만들지 않습니다.
/// 기존 journal record shape를 사용하는 ModelWork text snapshot으로 전달합니다.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivitySummary {
    /// host operation이 선택한 semantic heading입니다.
    pub kind: SummaryKind,
    /// view가 접혀도 보존하는 완전한 Markdown summary입니다.
    pub summary: String,
    /// host가 제공한 경우 compaction 전 관찰 token 수입니다.
    pub tokens_before: Option<u64>,
}

impl ActivitySummary {
    /// 정확한 presentation profile discriminator입니다.
    pub const SCHEMA: &str = "yo.activity-summary/v1";

    /// journal record variant를 바꾸지 않고 bounded profile을 인코딩합니다.
    pub fn to_snapshot(&self) -> Option<String> {
        if self.kind != SummaryKind::Compaction && self.tokens_before.is_some() {
            return None;
        }
        let text = serde_json::to_string(&OutputEnvelope {
            schema: Self::SCHEMA.to_owned(),
            output: self.clone(),
        })
        .ok()?;
        (text.len() <= ToolOutput::MAX_SNAPSHOT_BYTES).then_some(text)
    }

    /// 정확한 bounded profile만 허용하며 알 수 없는 field나 kind는 literal text로 남습니다.
    pub fn from_snapshot(text: &str) -> Option<Self> {
        if text.len() > ToolOutput::MAX_SNAPSHOT_BYTES {
            return None;
        }
        let envelope: OutputEnvelope<Self> = serde_json::from_str(text).ok()?;
        (envelope.schema == Self::SCHEMA
            && (envelope.output.kind == SummaryKind::Compaction
                || envelope.output.tokens_before.is_none()))
        .then_some(envelope.output)
    }
}

/// provider가 명시적으로 전달한 reasoning이며 public summary와 구별됩니다.
/// presentation visibility는 보존된 source를 제거하지 않습니다.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityReasoning {
    /// text stream snapshot 또는 원본 non-text content block입니다.
    pub content: Value,
}

impl ActivityReasoning {
    /// 기존 journal record를 사용하는 bounded ModelWork profile입니다.
    pub const SCHEMA: &str = "yo.activity-reasoning/v1";

    /// supplied content를 public summary로 재분류하지 않고 인코딩합니다.
    pub fn to_snapshot(&self) -> Option<String> {
        let text = serde_json::to_string(&OutputEnvelope {
            schema: Self::SCHEMA.to_owned(),
            output: self.clone(),
        })
        .ok()?;
        (text.len() <= ToolOutput::MAX_SNAPSHOT_BYTES).then_some(text)
    }

    /// 정확한 bounded profile만 허용하고 알려진 field만 읽습니다.
    pub fn from_snapshot(text: &str) -> Option<Self> {
        if text.len() > ToolOutput::MAX_SNAPSHOT_BYTES {
            return None;
        }
        let envelope: OutputEnvelope<Self> = serde_json::from_str(text).ok()?;
        (envelope.schema == Self::SCHEMA).then_some(envelope.output)
    }
}

/// 제안된 plan과 같은 명시적인 host 작성 Markdown document입니다.
/// presentation은 실행, approval 또는 attachment 권한을 부여하지 않습니다.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityDocument {
    /// Markdown body formatting과 독립적인 literal heading입니다.
    pub title: String,
    /// streaming replacement와 resize 중에도 보존하는 완전한 Markdown source입니다.
    pub markdown: String,
}

impl ActivityDocument {
    /// 기존 journal record shape를 사용하는 정확한 bounded ModelWork text profile입니다.
    pub const SCHEMA: &str = "yo.activity-document/v1";

    /// 비어 있지 않은 heading을 가진 bounded document를 인코딩합니다.
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

    /// 정확한 bounded profile과 알려진 field만 허용합니다.
    pub fn from_snapshot(text: &str) -> Option<Self> {
        if text.len() > ToolOutput::MAX_SNAPSHOT_BYTES {
            return None;
        }
        let envelope: OutputEnvelope<Self> = serde_json::from_str(text).ok()?;
        (envelope.schema == Self::SCHEMA && !envelope.output.title.is_empty())
            .then_some(envelope.output)
    }
}
