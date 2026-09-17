use std::fmt;

use yo_core::SessionId;

use super::model::{StartupFrontend, StartupOutcome};
use crate::{
    application::output::write_session_command_output, command, interaction::diagnostic::AppError,
    state::storage,
};

/// Startup 실패를 재개, continue, 새 실행으로 구분하는 입력입니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LaunchFailureSelection {
    New,
    Resume(SessionId),
    Continue,
}

/// 재개 실패가 발생한 startup 단계를 분류합니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResumeFailureStage {
    WritableStorage,
    Revalidation,
    RecordedWorkspace,
    WorkspaceReferences,
    SkillReferences,
    BackendSpawn,
    NativeResume,
}

impl ResumeFailureStage {
    pub(crate) const fn context(self) -> &'static str {
        match self {
            Self::WritableStorage => "opening writable local Yo storage failed",
            Self::Revalidation => "revalidation failed",
            Self::RecordedWorkspace => "the recorded workspace is unavailable",
            Self::WorkspaceReferences => "starting workspace reference discovery failed",
            Self::SkillReferences => "starting skill discovery failed",
            Self::BackendSpawn => "starting the selected agent backend failed",
            Self::NativeResume => "resuming the selected agent backend failed",
        }
    }
}

/// startup 실패를 중단 또는 읽기 전용 재개로 변환한 결과입니다.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ResumeFailureDisposition {
    Abort(String),
    ReadOnly {
        session_id: SessionId,
        reason: String,
    },
}

/// 실행 종류와 실패 단계를 기존 사용자 메시지와 같은 fallback 결과로 변환합니다.
pub(crate) fn classify_launch_failure(
    selection: LaunchFailureSelection,
    stage: ResumeFailureStage,
    detail: impl fmt::Display,
) -> ResumeFailureDisposition {
    let reason = format!("{}: {detail}", stage.context());
    match selection {
        LaunchFailureSelection::Resume(session_id) => {
            ResumeFailureDisposition::ReadOnly { session_id, reason }
        },
        LaunchFailureSelection::New | LaunchFailureSelection::Continue => {
            ResumeFailureDisposition::Abort(reason)
        },
    }
}

/// 읽기 전용 저장소에서 재개 결과를 출력하고 현재 generation을 완료합니다.
pub(crate) fn complete_with_read_only_resume(
    storage: &storage::LocalReadStorage,
    session_id: SessionId,
    glyph_profile: yo_tui::GlyphProfile,
    reason: &str,
) -> Result<StartupOutcome, AppError> {
    let reader = storage
        .reader()
        .ok_or_else(|| AppError::message("captured read-only storage has no Session reader"))?;
    let output = command::read_only_resume_from(reader, session_id, glyph_profile, reason)?;
    write_session_command_output(output)?;
    Ok(StartupOutcome::Complete)
}

/// 재개 startup 실패를 abort 또는 read-only resume outcome으로 마무리합니다.
pub(crate) fn handle_launch_failure(
    selection: LaunchFailureSelection,
    glyph_profile: yo_tui::GlyphProfile,
    storage: Option<&storage::LocalReadStorage>,
    stage: ResumeFailureStage,
    detail: impl fmt::Display,
) -> Result<StartupOutcome, AppError> {
    match classify_launch_failure(selection, stage, detail) {
        ResumeFailureDisposition::Abort(reason) => Err(AppError::many([reason])),
        ResumeFailureDisposition::ReadOnly { session_id, reason } => {
            let storage = storage.ok_or_else(|| {
                AppError::message("read-only resume fallback lost its captured local storage")
            })?;
            complete_with_read_only_resume(storage, session_id, glyph_profile, &reason)
        },
    }
}

pub(crate) fn require_exact_print_resume_binding(
    frontend: StartupFrontend,
    is_resume: bool,
    replaces_binding: bool,
) -> Result<(), AppError> {
    if matches!(frontend, StartupFrontend::Print) && is_resume && replaces_binding {
        return Err(AppError::message(
            "print resume requires the saved backend binding to remain executable without replacement",
        ));
    }
    Ok(())
}
