//! 실행 중인 TUI 상태의 인터뷰 명령과 작업 사본을 담당한다.

use yo_core::{ActivityDocument, interview as core_interview};

use super::{StateEffect, StateError, TuiState};
use crate::runner::session::TuiDocument;

impl TuiState {
    pub(in crate::runner) fn tick_interview(&mut self) -> Result<bool, StateError> {
        if let Some(controller) = &mut self.interview
            && let Some(notice) = controller.tick()
        {
            self.chat.push_notice(notice)?;
            return Ok(true);
        }
        Ok(false)
    }

    fn sync_interview_input(&mut self) {
        if !self.is_secret_input() {
            self.clear_secret_editor();
        }
    }

    pub(super) fn handle_interview_command(
        &mut self,
        argument: &str,
        draft: &str,
    ) -> Result<StateEffect, StateError> {
        let result = self
            .interview
            .as_mut()
            .ok_or_else(|| {
                core_interview::InterviewError::Invalid(
                    "interview recovery storage is unavailable".into(),
                )
            })
            .and_then(|controller| controller.command(argument));
        self.apply_interview_command(result, draft)
    }

    pub(super) fn apply_interview_command(
        &mut self,
        result: Result<super::super::interview::InterviewCommand, core_interview::InterviewError>,
        draft: &str,
    ) -> Result<StateEffect, StateError> {
        match result {
            Ok(command) => {
                if let Some(editor) = command.editor {
                    self.restore_draft(&editor);
                } else {
                    self.clear_editor();
                }
                self.sync_interview_input();
                self.sync_request_overlay()?;
                if let Some(document) = TuiDocument::new(ActivityDocument {
                    title: "Interview".into(),
                    markdown: command.document,
                }) {
                    self.observe_document(document.with_expanded(true))?;
                }
            },
            Err(error) => {
                self.restore_draft(draft);
                self.sync_interview_input();
                self.chat.push_notice(error.to_string())?;
            },
        }
        Ok(StateEffect::Redraw)
    }
}
