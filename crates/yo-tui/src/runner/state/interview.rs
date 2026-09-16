//! 실행 중인 TUI 상태의 인터뷰 명령과 작업 사본을 담당한다.

use yo_core::{ActivityDocument, interview as core_interview};

use super::{StateEffect, StateError, TuiState};
use crate::runner::session::TuiDocument;

impl TuiState {
    pub(in crate::runner) fn tick_interview(&mut self) -> Result<bool, StateError> {
        let was_editing = self.is_editing_interview();
        if let Some(controller) = &mut self.interview
            && let Some(notice) = controller.tick()
        {
            let resumed = (!was_editing && controller.is_editing()).then(|| {
                Ok(super::super::interview::InterviewCommand {
                    document: notice.clone(),
                    editor: Some(controller.editing_text()),
                    conversation: None,
                })
            });
            self.chat.push_notice(notice)?;
            if let Some(command) = resumed {
                self.apply_interview_command(command, "")?;
            }
            return Ok(true);
        }
        Ok(false)
    }

    pub(super) fn is_editing_interview(&self) -> bool {
        self.interview
            .as_ref()
            .is_some_and(|controller| controller.is_editing())
    }

    pub(super) fn handle_interview_command(
        &mut self,
        argument: &str,
        draft: &str,
    ) -> Result<StateEffect, StateError> {
        let busy = self.active_turn.is_some()
            || self.starting_submission.is_some()
            || self.context_compaction_pending
            || !self.pending_requests.is_empty()
            || !self.pending_submissions.is_empty()
            || self.pending_image.is_some()
            || self.pending_model_selection.is_some()
            || self.reserved_model_selection.is_some();
        let result = self
            .interview
            .as_mut()
            .ok_or_else(|| {
                core_interview::InterviewError::Invalid(
                    "interview recovery storage is unavailable".into(),
                )
            })
            .and_then(|controller| controller.command(argument, busy));
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
                if self.is_editing_interview() {
                    self.prompt_assist.cancel();
                    self.command_palette.close(&mut self.overlay);
                    self.close_request_overlay();
                    self.question_notes = None;
                    self.question_notes_refresh = None;
                    self.restored_question_draft = None;
                }
                self.sync_request_overlay()?;
                if let Some(document) = TuiDocument::new(ActivityDocument {
                    title: "Interview".into(),
                    markdown: command.document,
                }) {
                    self.observe_document(document.with_expanded(true))?;
                }
                if let Some(conversation) = command.conversation {
                    self.interview_conversation = Some(conversation);
                    return Ok(StateEffect::Exit);
                }
            },
            Err(error) => {
                self.restore_draft(draft);
                self.chat.push_notice(error.to_string())?;
            },
        }
        Ok(StateEffect::Redraw)
    }
}
