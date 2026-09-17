use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::OutputEnvelope;

/// 명시적으로 버전이 지정된 텍스트 snapshot으로 전달하는 backend 중립 도구 출력입니다.
/// 원본 JSON 필드는 host에서 계속 사용할 수 있으며 `plain_text`는 읽을 수 있는 fallback입니다.
/// journal record variant를 추가하지 않으며 실행 또는 첨부 권한을 부여하지 않습니다.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolOutput {
    /// adapter가 보고한 도구 식별자입니다.
    pub tool: String,
    /// 선택적인 도구 server 식별자입니다.
    pub server: Option<String>,
    /// schema별 해석을 하지 않은 원본 제안 인자입니다.
    pub arguments: Option<Value>,
    /// content, 구조화된 출력과 확장 metadata를 포함한 원본 결과 객체입니다.
    pub result: Option<Value>,
    /// adapter가 해당 결과 형태를 사용할 때의 원본 dynamic-tool content 배열입니다.
    pub content_items: Option<Value>,
    /// 원본 오류 세부 정보입니다.
    pub error: Option<Value>,
    /// rich presentation과 독립적인 adapter 생성 사람 읽기용 projection입니다.
    pub plain_text: String,
}

/// 새 journal record kind를 추가하지 않고 전달하는 backend 중립 non-text answer block입니다.
/// 알 수 없는 field는 renderer와 source export에서 사용할 수 있으며 URI를 가져오지 않습니다.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageContent {
    /// provider 확장 metadata를 포함한 원본 content block입니다.
    pub block: Value,
}

impl MessageContent {
    /// 이 presentation 전용 profile의 정확한 discriminator입니다.
    pub const SCHEMA: &str = "yo.message-content/v1";

    /// bounded snapshot을 인코딩하며 초과 입력은 caller의 literal fallback을 사용합니다.
    pub fn to_snapshot(&self) -> Option<String> {
        let text = serde_json::to_string(&OutputEnvelope {
            schema: Self::SCHEMA.to_owned(),
            output: self.clone(),
        })
        .ok()?;
        (text.len() <= ToolOutput::MAX_SNAPSHOT_BYTES).then_some(text)
    }

    /// 이 정확한 bounded profile만 허용하며 나머지 입력은 일반 텍스트로 남습니다.
    pub fn from_snapshot(text: &str) -> Option<Self> {
        if text.len() > ToolOutput::MAX_SNAPSHOT_BYTES {
            return None;
        }
        let envelope: OutputEnvelope<Self> = serde_json::from_str(text).ok()?;
        (envelope.schema == Self::SCHEMA).then_some(envelope.output)
    }
}

impl ToolOutput {
    /// 이 presentation 전용 output profile의 정확한 discriminator입니다.
    pub const SCHEMA: &str = "yo.tool-output/v1";
    /// 이 profile이 허용하는 최대 인코딩 byte 수입니다.
    pub const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;

    /// 완전한 snapshot을 인코딩하며 잘못된 식별자 또는 크기 초과 시 caller의 fallback을 사용합니다.
    pub fn to_snapshot(&self) -> Option<String> {
        if self.tool.is_empty() {
            return None;
        }
        let text = serde_json::to_string(&OutputEnvelope {
            schema: Self::SCHEMA.to_owned(),
            output: self.clone(),
        })
        .ok()?;
        (text.len() <= Self::MAX_SNAPSHOT_BYTES).then_some(text)
    }

    /// 이 정확한 profile만 허용하며 인식할 수 없거나 잘못된 입력은 일반 텍스트로 남습니다.
    pub fn from_snapshot(text: &str) -> Option<Self> {
        if text.len() > Self::MAX_SNAPSHOT_BYTES {
            return None;
        }
        let envelope: OutputEnvelope<Self> = serde_json::from_str(text).ok()?;
        (envelope.schema == Self::SCHEMA && !envelope.output.tool.is_empty())
            .then_some(envelope.output)
    }

    /// 알 수 없는 type과 field를 유지하면서 source 순서의 원본 content block을 반환합니다.
    pub fn content_blocks(&self) -> impl Iterator<Item = &Value> {
        self.result
            .as_ref()
            .and_then(|result| result.get("content"))
            .or(self.content_items.as_ref())
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
    }
}
