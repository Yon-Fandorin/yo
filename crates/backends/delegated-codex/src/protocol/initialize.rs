//! Codex initialize 응답, warning, version compatibility 해석.
//!
//! 초기화 handshake의 version 정책과 terminal-safe warning 표현을 한 곳에서
//! 유지합니다. 이 모듈은 runtime state를 가져오지 않습니다.

use std::fmt;

use serde::Deserialize;
use serde_json::Value;
use yo_core::{ActivityNotice, BackendFailure, BackendFailureKind, NoticeLevel};

use super::bounds::safe_user_agent;

const SUPPORTED_CODEX_MAJOR: u64 = 0;
const SUPPORTED_CODEX_MINORS: &[u64] = &[145, 146, 149];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InitializeResult {
    pub(crate) user_agent: String,
    pub(crate) platform_family: String,
    pub(crate) platform_os: String,
    #[serde(skip)]
    pub(crate) compatibility_warning: Option<CodexCompatibilityWarning>,
}

/// 길이와 제어문자 경계를 지킨 Codex compatibility/server warning입니다.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexWarning {
    display_user_agent: String,
    notice: Option<ActivityNotice>,
    thread_id: Option<String>,
}

/// 기존 observer 인자 이름과의 source compatibility를 위해 유지하는 alias입니다.
pub type CodexCompatibilityWarning = CodexWarning;

impl CodexWarning {
    pub(crate) fn redacted_for_secret() -> Self {
        Self {
            display_user_agent: String::new(),
            notice: Some(ActivityNotice {
                title: "Codex warning (details redacted)".to_owned(),
                message: "Warning details are redacted after secret input.".to_owned(),
                level: NoticeLevel::Warning,
            }),
            thread_id: None,
        }
    }

    /// Turn/tool activity와 독립적으로 표시할 terminal-safe notice를 반환합니다.
    pub fn to_notice(&self) -> ActivityNotice {
        self.notice.clone().unwrap_or_else(|| ActivityNotice {
            title: "Codex compatibility warning".to_owned(),
            message: self.to_string(),
            level: NoticeLevel::Warning,
        })
    }

    /// Codex warning notification을 안전한 notice로 변환합니다.
    pub(crate) fn from_notification(method: &str, params: &Value) -> Option<Self> {
        let (title, mut message) = match method {
            "warning" => ("Codex warning", params.get("message")?.as_str()?.to_owned()),
            "guardianWarning" => {
                params.get("threadId")?.as_str()?;
                (
                    "Codex approval warning",
                    params.get("message")?.as_str()?.to_owned(),
                )
            },
            "deprecationNotice" => (
                "Codex deprecation notice",
                params.get("summary")?.as_str()?.to_owned(),
            ),
            "configWarning" => (
                "Codex configuration warning",
                params.get("summary")?.as_str()?.to_owned(),
            ),
            _ => return None,
        };
        for (field, label) in [("details", "Details"), ("path", "File")] {
            if let Some(value) = params.get(field).and_then(Value::as_str) {
                message.push_str(&format!("\n{label}: {value}"));
            }
        }
        if let Some(range) = params.get("range").filter(|value| !value.is_null()) {
            message.push_str(&format!("\nRange: {range}"));
        }
        let thread_id = match params.get("threadId") {
            None | Some(Value::Null) => None,
            Some(Value::String(id)) => Some(id.clone()),
            _ => return None,
        };
        let mut safe = String::new();
        for character in message.chars() {
            let rendered = if character.is_control() && !matches!(character, '\n' | '\t') {
                character.escape_default().collect::<String>()
            } else {
                character.to_string()
            };
            if safe.len() + rendered.len() + 3 > 8192 {
                safe.push('…');
                break;
            }
            safe.push_str(&rendered);
        }
        Some(Self {
            display_user_agent: String::new(),
            notice: Some(ActivityNotice {
                title: title.to_owned(),
                message: safe,
                level: NoticeLevel::Warning,
            }),
            thread_id,
        })
    }

    /// warning notification이 연결된 Thread id를 반환합니다.
    pub(crate) fn thread_id(&self) -> Option<&str> {
        self.thread_id.as_deref()
    }
}

impl fmt::Display for CodexWarning {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(notice) = &self.notice {
            return write!(formatter, "{}: {}", notice.title, notice.message);
        }
        let supported = SUPPORTED_CODEX_MINORS
            .iter()
            .map(|minor| format!("{SUPPORTED_CODEX_MAJOR}.{minor}"))
            .collect::<Vec<_>>()
            .join(", ");
        formatter.write_str("Codex app-server `")?;
        formatter.write_str(&self.display_user_agent)?;
        write!(
            formatter,
            "` is newer or otherwise unverified; continuing because its 0.x protocol major matches (verified minor lines: {supported})"
        )
    }
}

/// initialize result를 decode하고 platform/version compatibility를 확인합니다.
pub(crate) fn decode_initialize(result: Value) -> Result<InitializeResult, BackendFailure> {
    let initialize: InitializeResult = serde_json::from_value(result).map_err(|error| {
        BackendFailure::new(
            BackendFailureKind::Initialization,
            format!("invalid Codex initialize response: {error}"),
        )
    })?;
    let mut initialize = initialize;
    initialize.compatibility_warning = version_compatibility_warning(&initialize.user_agent)?;
    if initialize.platform_family != "unix"
        || !matches!(initialize.platform_os.as_str(), "linux" | "macos")
    {
        return Err(BackendFailure::new(
            BackendFailureKind::Initialization,
            "unsupported Codex app-server platform; expected unix/linux or unix/macos",
        ));
    }
    Ok(initialize)
}

/// 설치된 Codex version을 compatibility warning 또는 초기화 오류로 분류합니다.
pub(crate) fn version_compatibility_warning(
    user_agent: &str,
) -> Result<Option<CodexCompatibilityWarning>, BackendFailure> {
    let display_user_agent = safe_user_agent(user_agent);
    let version = user_agent
        .split_whitespace()
        .find_map(|part| part.split_once('/').map(|(_, version)| version))
        .unwrap_or(user_agent);
    let mut components = version.split('.');
    let major = components.next().and_then(|part| part.parse::<u64>().ok());
    let minor = components.next().and_then(|part| part.parse::<u64>().ok());
    let Some(major) = major else {
        return Err(BackendFailure::new(
            BackendFailureKind::Initialization,
            format!("Codex app-server returned an unparseable version in `{display_user_agent}`"),
        ));
    };
    let Some(minor) = minor else {
        return Err(BackendFailure::new(
            BackendFailureKind::Initialization,
            format!("Codex app-server returned an unparseable version in `{display_user_agent}`"),
        ));
    };
    if major != SUPPORTED_CODEX_MAJOR {
        return Err(BackendFailure::new(
            BackendFailureKind::Initialization,
            format!(
                "unsupported Codex app-server major version in `{display_user_agent}`; yo requires {SUPPORTED_CODEX_MAJOR}.x"
            ),
        ));
    }
    if SUPPORTED_CODEX_MINORS.contains(&minor) {
        return Ok(None);
    }
    Ok(Some(CodexWarning {
        display_user_agent,
        notice: None,
        thread_id: None,
    }))
}

/// 검토된 정확한 Codex image-capable wire build만 허용합니다.
pub(crate) fn image_wire_version_supported(user_agent: &str) -> bool {
    matches!(
        user_agent
            .split_whitespace()
            .find_map(|part| part.split_once('/').map(|(_, version)| version)),
        Some("0.153.4" | "0.154.0")
    )
}
