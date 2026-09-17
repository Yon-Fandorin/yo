//! Codex wire 값의 공통 경계와 안전한 진단 도구.
//!
//! 이 모듈은 런타임 상태를 알지 못하며, 길이·필드·오류 표현의 공통 규칙만
//! 제공합니다.

use serde_json::Value;
use yo_core::{BackendFailure, BackendFailureKind};

pub(crate) const MAX_USER_AGENT_DISPLAY_BYTES: usize = 256;

/// Catalog 텍스트가 표시와 stable identity에 사용할 수 있는지 확인합니다.
pub(crate) fn valid_catalog_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

/// User-Agent를 제어문자 escape와 byte bound를 적용한 진단 문자열로 만듭니다.
pub(crate) fn safe_user_agent(user_agent: &str) -> String {
    const ELLIPSIS: &str = "…";
    let mut output = String::new();
    for character in user_agent.chars() {
        let rendered = if character.is_control() {
            character.escape_default().collect::<String>()
        } else {
            character.to_string()
        };
        if output.len() + rendered.len() + ELLIPSIS.len() > MAX_USER_AGENT_DISPLAY_BYTES {
            output.push_str(ELLIPSIS);
            break;
        }
        output.push_str(&rendered);
    }
    output
}

/// JSON object에서 지정한 경로의 문자열 필드를 bounded protocol 오류로 읽습니다.
pub(crate) fn string_at<'a>(value: &'a Value, path: &[&str]) -> Result<&'a str, BackendFailure> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment).ok_or_else(|| {
            protocol_failure(format!("Codex message is missing `{}`", path.join(".")))
        })?;
    }
    current.as_str().ok_or_else(|| {
        protocol_failure(format!("Codex field `{}` is not a string", path.join(".")))
    })
}

/// wire 해석 실패를 provider protocol 오류로 통일합니다.
pub(crate) fn protocol_failure(message: impl Into<String>) -> BackendFailure {
    BackendFailure::new(BackendFailureKind::Protocol, message)
}
