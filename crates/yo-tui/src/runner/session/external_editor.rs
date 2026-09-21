use std::{error::Error, fmt};

use yo_core::UserInput;

use super::TuiSession;

/// 호스트 외부 편집기에 전달된 하나의 Chat 초안 세대입니다.
///
/// 이 값은 편집기 결과를 다시 적용할 때 원래 초안과 현재 TUI 초안이
/// 같은지 확인하는 데 사용됩니다. 호스트는 `text`만 외부 편집기의
/// 파일 내용으로 사용해야 합니다.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalEditorSnapshot {
    input: UserInput,
    cursor_byte_index: usize,
    generation: u64,
}

impl ExternalEditorSnapshot {
    pub(crate) fn new(input: UserInput, cursor_byte_index: usize, generation: u64) -> Self {
        Self {
            input,
            cursor_byte_index,
            generation,
        }
    }

    /// 외부 편집기에 보여 줄 정확한 초안 텍스트입니다.
    #[must_use]
    pub fn text(&self) -> &str {
        self.input.as_str()
    }

    pub(crate) fn input(&self) -> &UserInput {
        &self.input
    }

    pub(crate) const fn cursor_byte_index(&self) -> usize {
        self.cursor_byte_index
    }

    pub(crate) const fn generation(&self) -> u64 {
        self.generation
    }
}

/// 외부 편집기 결과를 현재 Chat 초안에 적용할 수 없는 이유입니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ExternalEditorImportError {
    /// 적용할 외부 편집기 요청이 없습니다.
    NoRequest,
    /// 결과가 요청 당시의 초안 세대와 일치하지 않습니다.
    StaleDraft,
    /// 결과가 typed reference, skill 또는 image marker의 일부를 바꿉니다.
    AmbiguousAnnotations,
    /// 결과를 검증된 입력으로 재구성할 수 없습니다.
    InvalidDraft,
}

impl fmt::Display for ExternalEditorImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoRequest => "no external editor request is pending",
            Self::StaleDraft => "the external editor result belongs to an older draft",
            Self::AmbiguousAnnotations => {
                "the external editor changed a typed reference or image marker"
            },
            Self::InvalidDraft => "the external editor result is not a valid draft",
        })
    }
}

impl Error for ExternalEditorImportError {}

impl TuiSession {
    /// 마지막으로 반환한 외부 편집기 요청의 immutable snapshot을 복사합니다.
    #[must_use]
    pub fn external_editor_snapshot(&self) -> Option<ExternalEditorSnapshot> {
        self.state.external_editor_snapshot()
    }

    /// 외부 편집기의 결과를 하나의 undo 가능한 Chat 초안 편집으로 적용합니다.
    ///
    /// 결과는 제출하지 않습니다. 현재 초안이 snapshot과 달라졌거나 기존
    /// typed annotation과 겹치는 변경이면 초안을 보존하고 오류를 반환합니다.
    pub fn import_external_editor_result(
        &mut self,
        snapshot: &ExternalEditorSnapshot,
        text: impl Into<String>,
    ) -> Result<(), ExternalEditorImportError> {
        self.state
            .import_external_editor_result(snapshot, text.into())
    }

    /// 외부 편집기 실행 또는 결과 적용 실패를 표시하고 보류한 snapshot을 폐기합니다.
    /// 현재 Chat 초안은 그대로 유지됩니다.
    pub fn report_external_editor_failure(&mut self, detail: impl Into<String>) {
        self.state.report_external_editor_failure(detail.into());
    }

    pub(in crate::runner) fn take_external_editor_request(&mut self) -> bool {
        self.state.take_external_editor_request()
    }
}
