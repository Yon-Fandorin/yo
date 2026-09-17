use std::fmt::Display;

use super::model::{StartupFrontend, StartupOutcome};
use crate::{
    application::{live_selection as live, output::write_session_command_output},
    command,
    interaction::diagnostic::AppError,
    state::storage,
};

/// 읽기 전용 저장소에서 재개 결과를 출력하고 현재 generation을 완료합니다.
pub(in crate::application::runtime) fn complete_with_read_only_resume(
    storage: &storage::LocalReadStorage,
    session_id: yo_core::SessionId,
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
pub(in crate::application::runtime) fn handle_launch_failure(
    selection: live::LiveSelection,
    glyph_profile: yo_tui::GlyphProfile,
    storage: Option<&storage::LocalReadStorage>,
    stage: live::ResumeFailureStage,
    detail: impl Display,
) -> Result<StartupOutcome, AppError> {
    match live::classify_launch_failure(selection, stage, detail) {
        live::ResumeFailureDisposition::Abort(reason) => Err(AppError::many([reason])),
        live::ResumeFailureDisposition::ReadOnly { session_id, reason } => {
            let storage = storage.ok_or_else(|| {
                AppError::message("read-only resume fallback lost its captured local storage")
            })?;
            complete_with_read_only_resume(storage, session_id, glyph_profile, &reason)
        },
    }
}

pub(in crate::application::runtime) fn require_exact_print_resume_binding(
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
