use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::OutputEnvelope;

/// 로컬에 게시된 파일 내용을 담는 bounded, versioned 관찰 자료입니다.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilePublicationEvidence {
    path: String,
    state: FilePublicationEvidenceState,
}

/// 캡처한 파일 내용 또는 내용을 사용할 수 없는 명시적 이유입니다.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum FilePublicationEvidenceState {
    /// 성공적으로 게시한 파일의 기존 내용과 게시 후 내용입니다.
    Complete {
        /// edit가 게시하기 전에 관찰한 파일 내용입니다.
        before: String,
        /// 성공한 게시 이후 계획된 파일 내용입니다.
        after: String,
    },
    /// 내용을 보존하지 않고 캡처를 사용할 수 없는 상태입니다.
    Unavailable {
        /// 캡처 내용을 생략한 폐쇄형 사유입니다.
        reason: FilePublicationEvidenceUnavailableReason,
    },
}

/// 캡처한 파일 내용을 생략한 이유를 payload 없이 표현하는 폐쇄형 값입니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilePublicationEvidenceUnavailableReason {
    /// 설정에서 내용 캡처를 사용하지 않았습니다.
    Disabled,
    /// 캡처한 내용이 허용된 byte 상한을 넘었습니다.
    OverBound,
    /// 선택적인 캡처 준비 또는 인코딩이 완료되지 않았습니다.
    GenerationFailed,
    /// 개인정보 보호 심사에서 내용을 거절하거나 변경했습니다.
    SemanticAdmission,
    /// 캡처 profile을 인식하거나 경로와 연결하지 못했습니다.
    InvalidEvidence,
    /// activity snapshot에 전체 결과를 담을 공간이 부족했습니다.
    SnapshotCapacity,
}

impl FilePublicationEvidence {
    /// presentation 전용 profile을 식별하는 정확한 discriminator입니다.
    pub const SCHEMA: &str = "yo.file-publication-evidence/v1";
    /// 인코딩된 profile의 최대 크기입니다.
    pub const MAX_SNAPSHOT_BYTES: usize = 1024 * 1024;

    /// 이스케이프 전 payload가 raw 상한에 맞으면 완전한 evidence를 만듭니다.
    pub fn complete(
        path: impl Into<String>,
        before: impl Into<String>,
        after: impl Into<String>,
    ) -> Option<Self> {
        let evidence = Self {
            path: path.into(),
            state: FilePublicationEvidenceState::Complete {
                before: before.into(),
                after: after.into(),
            },
        };
        evidence.is_valid().then_some(evidence)
    }

    /// 승인된 경로에 대해 payload가 없는 unavailable 관찰 자료를 만듭니다.
    pub fn unavailable(
        path: impl Into<String>,
        reason: FilePublicationEvidenceUnavailableReason,
    ) -> Option<Self> {
        let evidence = Self {
            path: path.into(),
            state: FilePublicationEvidenceState::Unavailable { reason },
        };
        evidence.is_valid().then_some(evidence)
    }

    /// 이 관찰 자료와 연결된 workspace 상대 경로입니다.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// 캡처한 파일 내용 또는 폐쇄형 unavailable 이유입니다.
    pub const fn state(&self) -> &FilePublicationEvidenceState {
        &self.state
    }

    /// 이 bounded presentation profile을 정확한 형식으로 인코딩합니다.
    pub fn to_snapshot(&self) -> Option<String> {
        if !self.is_valid() {
            return None;
        }
        let text = serde_json::to_string(&OutputEnvelope {
            schema: Self::SCHEMA.to_owned(),
            output: self.clone(),
        })
        .ok()?;
        (text.len() <= Self::MAX_SNAPSHOT_BYTES).then_some(text)
    }

    /// 이 bounded profile과 폐쇄형 state variant만 디코딩합니다.
    pub fn from_snapshot(text: &str) -> Option<Self> {
        if text.len() > Self::MAX_SNAPSHOT_BYTES {
            return None;
        }
        let envelope: OutputEnvelope<Self> = serde_json::from_str(text).ok()?;
        (envelope.schema == Self::SCHEMA && envelope.output.is_valid()).then_some(envelope.output)
    }

    fn is_valid(&self) -> bool {
        if self.path.is_empty() || self.path.len() > 1024 || self.path.chars().any(char::is_control)
        {
            return false;
        }
        match &self.state {
            FilePublicationEvidenceState::Complete { before, after } => before
                .len()
                .checked_add(after.len())
                .and_then(|bytes| bytes.checked_add(self.path.len()))
                .is_some_and(|bytes| bytes <= Self::MAX_SNAPSHOT_BYTES),
            FilePublicationEvidenceState::Unavailable { .. } => true,
        }
    }
}

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
