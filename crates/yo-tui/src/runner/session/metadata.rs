use std::{collections::BTreeMap, error::Error, fmt, num::NonZeroU16, sync::Arc};

use yo_core::{
    ActivityDocument, ActivityNotice, NoticeLevel, SessionId,
    session_repository::ContinuationEligibility,
};

use crate::text::flow::flow_text;

/// 호스트가 발견한 저장 세션입니다. 재개하기 전에 적합성을 다시 확인합니다.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResumeSessionEntry {
    /// 영구 저장되는 세션 식별자입니다.
    pub session_id: SessionId,
    /// 발견 당시의 근거이며, 호스트가 실행을 재개하기 전에 다시 검증합니다.
    pub eligibility: ContinuationEligibility,
    /// 호스트가 표시 형식으로 만든 영구 업데이트 시각이거나 명시적인 미상 레이블입니다.
    pub updated_label: String,
}

/// 호스트가 알고 있어 TUI 상태 줄에 표시할 수 있는 레이블입니다.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TuiSessionInfo {
    backend: Option<String>,
    workspace: String,
    startup_resumed: Option<bool>,
}

/// 검증된 호스트 Markdown 문서입니다. 모델 Turn이나 저널 레코드 없이 표시합니다.
/// 같은 문서를 다시 관찰해도 별도 문서로 추가하며, 중복 제거와 보존은 호스트가 담당합니다.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TuiDocument {
    snapshot: Arc<str>,
    expanded: Option<bool>,
}

impl TuiDocument {
    /// 기존 ActivityDocument 표시 한도를 검증한 뒤 TUI에 게시할 문서를 만듭니다.
    /// 유효하지 않거나 너무 큰 문서에는 None을 반환합니다. 생성 과정에서는 I/O를 수행하지 않습니다.
    #[must_use]
    pub fn new(document: ActivityDocument) -> Option<Self> {
        document.to_snapshot().map(|snapshot| Self {
            snapshot: Arc::from(snapshot),
            expanded: None,
        })
    }

    /// 이 문서의 초기 펼침 상태를 지정합니다. 생략하면 현재 전역 상태를 따릅니다.
    /// 사용자는 Alt+O로 항목을 전환하거나 Ctrl+O로 모든 항목을 초기화할 수 있습니다.
    #[must_use]
    pub fn with_expanded(mut self, expanded: bool) -> Self {
        self.expanded = Some(expanded);
        self
    }

    pub(in crate::runner) fn expanded(&self) -> Option<bool> {
        self.expanded
    }

    pub(in crate::runner) fn snapshot(&self) -> &str {
        &self.snapshot
    }
}

/// 대화 이력과 분리되어 키 순서로 정렬되는 완전한 임시 호스트 상태 줄입니다.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TuiStatusLine {
    text: String,
}

/// 유효하지 않거나 너무 큰 호스트 상태 스냅샷입니다. 기존 줄은 그대로 둘 수 있습니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TuiStatusError {
    /// 항목이 16개를 초과했습니다.
    TooManyEntries,
    /// 비어 있거나 제어 문자를 포함하거나 64바이트를 초과한 키입니다.
    InvalidKey,
    /// 같은 스냅샷에 키가 중복되었습니다.
    DuplicateKey,
    /// 원문 또는 이스케이프된 항목 텍스트가 1024바이트를 초과했습니다.
    TextTooLong,
    /// 텍스트를 터미널 한 줄로 표현할 수 없습니다.
    InvalidText,
}

impl fmt::Display for TuiStatusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooManyEntries => "host status supports at most 16 entries",
            Self::InvalidKey => {
                "host status keys must be nonempty, control-free and at most 64 bytes"
            },
            Self::DuplicateKey => "host status keys must be unique",
            Self::TextTooLong => "host status entry exceeds 1024 bytes",
            Self::InvalidText => "host status text cannot be displayed",
        })
    }
}

impl Error for TuiStatusError {}

impl TuiStatusLine {
    /// 고유 키를 최대 16개까지 받아 1024바이트 표시 값으로 전체 교체본을 만듭니다.
    /// 표시 크기를 제한하기 전에 제어 문자를 이스케이프합니다. 빈 값은 생략하며,
    /// 빈 스냅샷은 줄을 비웁니다. 키가 정렬 순서를 결정하지만 화면에는 표시하지 않습니다.
    pub fn new<K: AsRef<str>, V: AsRef<str>>(
        entries: impl IntoIterator<Item = (K, V)>,
    ) -> Result<Self, TuiStatusError> {
        let mut sorted = BTreeMap::new();
        for (index, (key, value)) in entries.into_iter().enumerate() {
            if index >= 16 {
                return Err(TuiStatusError::TooManyEntries);
            }
            let key = key.as_ref();
            if key.is_empty() || key.len() > 64 || key.chars().any(char::is_control) {
                return Err(TuiStatusError::InvalidKey);
            }
            let value = value.as_ref();
            if value.len() > 1024 {
                return Err(TuiStatusError::TextTooLong);
            }
            let value = single_line_label(value.to_owned());
            if value.len() > 1024 {
                return Err(TuiStatusError::TextTooLong);
            }
            if sorted.insert(key.to_owned(), value).is_some() {
                return Err(TuiStatusError::DuplicateKey);
            }
        }
        let text = sorted
            .values()
            .filter(|value| !value.is_empty())
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" · ");
        let flow = flow_text(&text, NonZeroU16::MAX).map_err(|_| TuiStatusError::InvalidText)?;
        if flow.height > 1 {
            return Err(TuiStatusError::InvalidText);
        }
        Ok(Self { text })
    }

    /// 터미널 너비에 맞게 줄이기 전의 정제된 표시 텍스트입니다.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }
}
impl TuiSessionInfo {
    /// 호스트가 제공한 표시 값으로 안전한 한 줄 상태 레이블을 만듭니다.
    #[must_use]
    pub fn new(backend: impl Into<String>, workspace: impl Into<String>) -> Self {
        Self {
            backend: non_empty_label(backend.into()),
            workspace: single_line_label(workspace.into()),
            startup_resumed: None,
        }
    }

    /// 호스트가 확인한 세션 시작점을 사용해 임시 시작 안내를 한 번 표시하도록 요청합니다.
    /// 이 옵션을 생략하면 상태 줄 레이블만 유지하며, 기존 안내 스타일을 적용합니다.
    #[must_use]
    pub fn with_startup_notice(mut self, resumed: bool) -> Self {
        self.startup_resumed = Some(resumed);
        self
    }

    pub(in crate::runner) fn startup_notice(&self) -> Option<ActivityNotice> {
        let resumed = self.startup_resumed?;
        let bounded = |value: &str| {
            let mut label = value.chars().take(4096).collect::<String>();
            if value.chars().nth(4096).is_some() {
                label.push('…');
            }
            label
        };
        let mut lines = Vec::new();
        if let Some(backend) = self.backend() {
            lines.push(format!("Backend: {}", bounded(backend)));
        }
        if !self.workspace.is_empty() {
            lines.push(format!("Workspace: {}", bounded(&self.workspace)));
        }
        if lines.is_empty() {
            return None;
        }
        Some(ActivityNotice {
            title: if resumed {
                "Session resumed"
            } else {
                "New session"
            }
            .to_owned(),
            message: lines.join("\n"),
            level: NoticeLevel::Info,
        })
    }

    pub(in crate::runner) fn backend(&self) -> Option<&str> {
        self.backend.as_deref()
    }

    pub(in crate::runner) fn set_backend(&mut self, backend: String) {
        self.backend = non_empty_label(backend);
    }

    pub(in crate::runner) fn workspace(&self) -> &str {
        &self.workspace
    }
}

fn non_empty_label(value: String) -> Option<String> {
    let label = single_line_label(value);
    (!label.is_empty()).then_some(label)
}

fn single_line_label(value: String) -> String {
    let mut label = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control() {
            label.extend(character.escape_default());
        } else {
            label.push(character);
        }
    }
    label
}
