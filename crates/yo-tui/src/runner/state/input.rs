//! 실행 중인 TUI 상태의 입력 승인과 후속 입력을 담당한다.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use yo_core::{
    InputSubmission, SubmissionId, SubmissionOutcome, UserInput, interview::InterviewError,
    secret_store::RetentionPolicy,
};

use super::{FOLLOW_UP_BYTES, FOLLOW_UP_LIMIT, PendingRequest, StateEffect, StateError, TuiState};
use crate::{
    command::{
        CommandEffect, attachment_argument, compact_argument, fork_argument, model_argument,
        prompt_argument, resume_argument, secrets_argument, tree_argument,
    },
    input::{
        editor::EditorEffect,
        event::{InputEvent, KeyAction, KeyCode, KeyModifiers},
        secret::{SecretEditorEffect, SecretRetention},
    },
    overlay::OverlayInputEffect,
    prompt::{assist::PromptAssistRequest, workspace_reference::WorkspaceEdit},
    runner::{
        AgentAction, ForkPickerToken,
        view::{ObservabilityView, ViewInputEffect},
    },
};

impl TuiState {
    pub(in crate::runner) fn handle(
        &mut self,
        input: InputEvent,
        now: Duration,
    ) -> Result<StateEffect, StateError> {
        let previous_text = self.editor.text().to_owned();
        let previous_notes = self.question_notes;
        let interview_command = previous_text.starts_with("/interview")
            && matches!(&input,
            InputEvent::Key(key) if key.code == KeyCode::Enter && key.modifiers == KeyModifiers::NONE);
        let selection_edit = matches!(&input, InputEvent::Key(key)
            if matches!(key.code, KeyCode::Up | KeyCode::Down)
                && key.modifiers == KeyModifiers::NONE);
        let effect = self.handle_input(input, now)?;
        let request = self
            .pending_requests
            .front()
            .and_then(|pending| match pending {
                PendingRequest::UserInput(request) => Some(*request),
                _ => None,
            });
        let choice = self.question_notes.map(|(_, choice)| choice).or_else(|| {
            (selection_edit && self.editor.text().is_empty())
                .then(|| {
                    self.request_overlay.filter(|(owner, token)| {
                        Some(owner.activity()) == request.map(|r| r.activity())
                            && self.overlay.is_current(*token)
                    })?;
                    self.overlay
                        .panel()?
                        .selected_identity()?
                        .as_str()
                        .parse()
                        .ok()
                })
                .flatten()
        });
        if let Some(controller) = &mut self.interview {
            let response = match &effect {
                StateEffect::Dispatch(AgentAction::RespondToUserInput { request, input }) => {
                    Some((
                        *request,
                        yo_core::ActivityResponse::UserInput(UserInput::new(input)),
                    ))
                },
                StateEffect::Dispatch(AgentAction::RespondToQuestion {
                    request,
                    choice,
                    notes,
                }) => Some((
                    *request,
                    yo_core::ActivityResponse::QuestionAnswer {
                        choice: *choice,
                        notes: UserInput::new(notes),
                    },
                )),
                StateEffect::Dispatch(AgentAction::PreviousQuestion {
                    request,
                    choice,
                    draft,
                }) => Some((
                    *request,
                    yo_core::ActivityResponse::PreviousQuestion {
                        choice: *choice,
                        draft: UserInput::new(draft),
                    },
                )),
                StateEffect::Dispatch(AgentAction::RespondToSecretInput { .. }) => None,
                _ => None,
            };
            let notice = if let Some((request, response)) = response {
                controller.retain_live_response(request, response)
            } else if matches!(effect, StateEffect::Redraw)
                && !interview_command
                && (self.editor.text() != previous_text
                    || previous_notes != self.question_notes
                    || (selection_edit && self.editor.text().is_empty()))
            {
                controller.edit_text(self.editor.text(), request, choice);
                None
            } else if matches!(effect, StateEffect::Exit | StateEffect::Suspend) {
                controller.flush()
            } else {
                None
            };
            if let Some(notice) = notice {
                self.chat.push_notice(notice)?;
                if matches!(effect, StateEffect::Exit | StateEffect::Suspend) {
                    return Ok(StateEffect::Redraw);
                }
            }
        }
        if let StateEffect::Dispatch(AgentAction::Submit(submission)) = &effect {
            self.starting_submission = Some(submission.id());
        }
        if matches!(
            effect,
            StateEffect::Dispatch(AgentAction::CompactContext { .. })
        ) {
            self.context_compaction_pending = true;
        }
        if matches!(effect, StateEffect::Dispatch(AgentAction::Interrupt)) {
            self.follow_ups_paused = true;
        }
        Ok(effect)
    }

    fn handle_input(
        &mut self,
        input: InputEvent,
        now: Duration,
    ) -> Result<StateEffect, StateError> {
        if self.preview.is_some() {
            return self.handle_preview(input, now);
        }
        if let InputEvent::Resize(size) = input {
            return Ok(StateEffect::Resize(size));
        }
        if input.is_ctrl_z_press() {
            return Ok(StateEffect::Suspend);
        }
        if self.presentation_blocked() {
            return self.handle_presentation_blocked(input);
        }
        if self.is_secret_input() {
            if self.is_secret_previous_question_input(&input) {
                return self.handle_secret_previous_question();
            }
            return self.handle_secret_input(input);
        }
        if let InputEvent::Key(key) = &input
            && key.modifiers == KeyModifiers::CONTROL
            && matches!(key.code, KeyCode::Character('v' | 'V'))
        {
            if key.action != KeyAction::Press {
                return Ok(StateEffect::Unchanged);
            }
            if self.views.active() == ObservabilityView::Chat {
                return self.prepare_clipboard_image();
            }
            return Ok(StateEffect::Unchanged);
        }
        let active_before = self.views.active();
        match self.views.handle_global(&input) {
            ViewInputEffect::Unhandled => {},
            ViewInputEffect::Consumed => return Ok(StateEffect::Unchanged),
            ViewInputEffect::Redraw => {
                if self.views.active() != active_before {
                    self.cancel_fork_picker();
                    if let Some((request, token)) = self.request_overlay
                        && self.overlay.is_current(token)
                        && let Some(snapshot) = self.request_panel.clone()
                        && let Some(panel) = self.overlay.panel().cloned()
                    {
                        self.saved_request_panel = Some((request, snapshot, panel));
                    }
                    self.overlay.close_current();
                    self.command_palette.dismiss();
                    self.model_overlay = None;
                    self.prompt_assist.cancel();
                    self.request_overlay = None;
                    self.sync_request_overlay()?;
                }
                return Ok(StateEffect::Redraw);
            },
        }

        if self.question_notes_refresh.is_some()
            && matches!(&input, InputEvent::Key(key) if key.code == KeyCode::Enter && key.modifiers == KeyModifiers::NONE)
        {
            return Ok(StateEffect::Unchanged);
        }
        // 보존된 변경 사항 검토는 승인을 제출하지 않는다. command palette가 request panel을
        // 대신하므로 해당 panel의 commit token을 요구하지 않는다.
        let reviewing_changes = self
            .command_palette
            .exact_submission(self.editor.text(), self.editor.cursor_byte_index())
            .is_some_and(|command| command.effect() == CommandEffect::ReviewChanges);
        if !reviewing_changes
            && self.pending_requests.front().is_some_and(|request| {
                matches!(request, PendingRequest::Approval(_))
                    && self.chat.approval(request.activity()).is_some()
            })
            && matches!(&input, InputEvent::Key(key) if key.code == KeyCode::Enter && key.modifiers == KeyModifiers::NONE)
            && !self.request_overlay.is_some_and(|(pending, token)| {
                self.pending_requests.front() == Some(&pending)
                    && self.overlay.is_current(token)
                    && self.overlay.can_submit_current()
            })
        {
            return Ok(StateEffect::Unchanged);
        }
        if self.views.active() == ObservabilityView::Chat
            && matches!(&input, InputEvent::Key(key) if key.code == KeyCode::BackTab
                && key.action == KeyAction::Press
                && (key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT))
            && let Some(pending @ PendingRequest::UserInput(request)) =
                self.pending_requests.front().copied()
            && self
                .chat
                .question(request.activity())
                .is_some_and(|question| question.previous_question)
        {
            if !self.request_overlay.is_some_and(|(owner, token)| {
                owner == pending
                    && self.overlay.is_current(token)
                    && self.overlay.can_submit_current()
            }) {
                return Ok(StateEffect::Unchanged);
            }
            if self.reject_referenced_answer()? {
                return Ok(StateEffect::Redraw);
            }
            let draft = self.editor.text().to_owned();
            let choice = self
                .question_notes
                .take()
                .filter(|(owner, _)| *owner == pending)
                .map(|(_, choice)| choice)
                .or_else(|| {
                    draft
                        .is_empty()
                        .then(|| self.interview.as_ref()?.live_draft(request.activity())?.0)
                        .flatten()
                });
            self.editor.replace_range(0..draft.len(), "");
            self.pending_requests.pop_front();
            self.close_request_overlay();
            self.sync_request_overlay()?;
            return Ok(StateEffect::Dispatch(AgentAction::PreviousQuestion {
                request,
                choice,
                draft,
            }));
        }

        let notes_key = matches!(&input, InputEvent::Key(key)
            if key.code == KeyCode::Tab && key.modifiers == KeyModifiers::NONE
                && key.action == KeyAction::Press);
        if notes_key
            && self.question_notes.is_some()
            && self.views.active() == ObservabilityView::Chat
        {
            self.question_notes = None;
            self.close_request_overlay();
            self.sync_request_overlay()?;
            return Ok(StateEffect::Redraw);
        }

        if self.restored_question_draft.is_some()
            && self.restored_question_draft == self.pending_requests.front().copied()
            && matches!(&input, InputEvent::Key(key) if key.code == KeyCode::Enter && key.modifiers == KeyModifiers::NONE)
            && !self.request_overlay.is_some_and(|(_, token)| {
                self.overlay.is_current(token) && self.overlay.can_submit_current()
            })
        {
            return Ok(StateEffect::Unchanged);
        }

        // 직접 입력한 답변과 로컬 명령은 기존 editor 경로를 유지한다.
        let typed_reply = self.question_notes.is_none()
            && self.request_overlay.is_some()
            && !self.editor.text().is_empty()
            && matches!(&input, InputEvent::Key(key) if key.code == KeyCode::Enter);
        let overlay_effect = if typed_reply {
            OverlayInputEffect::Unhandled
        } else {
            self.overlay.handle(&input)
        };
        match overlay_effect {
            OverlayInputEffect::Unhandled => {},
            OverlayInputEffect::Consumed => return Ok(StateEffect::Unchanged),
            OverlayInputEffect::Redraw => return Ok(StateEffect::Redraw),
            OverlayInputEffect::Dismissed(token) => {
                if self
                    .fork_overlay
                    .is_some_and(|(current, _)| current == token)
                {
                    self.cancel_fork_picker();
                    return Ok(StateEffect::Redraw);
                }
                if self.resume_overlay == Some(token) {
                    self.resume_overlay = None;
                    return Ok(StateEffect::Redraw);
                }

                if let Some((pending, current)) = self.request_overlay
                    && current == token
                {
                    self.request_overlay = None;
                    return match pending {
                        PendingRequest::Approval(request) => {
                            self.approval_response(request, "n".to_owned())
                        },
                        PendingRequest::UserInput(_) => {
                            Ok(StateEffect::Dispatch(AgentAction::Interrupt))
                        },
                        PendingRequest::PresentationPending(_)
                        | PendingRequest::PresentationInvalid(_) => {
                            Ok(StateEffect::Dispatch(AgentAction::Interrupt))
                        },
                        PendingRequest::SecretInput(_) => {
                            Ok(StateEffect::Dispatch(AgentAction::Interrupt))
                        },
                    };
                }
                if self
                    .command_palette
                    .dismiss_visible(token, self.editor.text())
                {
                    return Ok(StateEffect::Redraw);
                }
                if self.model_overlay == Some(token) {
                    self.model_overlay = None;
                }
                self.prompt_assist.cancel();
                return Ok(StateEffect::Redraw);
            },
            OverlayInputEffect::AcceptedEmpty(token) => {
                if self
                    .command_palette
                    .reject_visible(token, &mut self.overlay)
                {
                    self.push_unknown_command_notice(self.editor.text().to_owned())?;
                    return Ok(StateEffect::Redraw);
                }
                return Ok(StateEffect::Unchanged);
            },
            OverlayInputEffect::FilterChanged(selected) => {
                self.prompt_assist
                    .filter_changed(selected, &mut self.overlay);
                return Ok(StateEffect::Redraw);
            },
            OverlayInputEffect::Accepted(receipt) => {
                if let Some((token, count)) = self.fork_overlay
                    && token == receipt.token()
                {
                    self.fork_overlay = None;
                    if !self.editor.text().is_empty() {
                        self.cancel_fork_picker();
                        self.chat.push_notice("Your newer draft was preserved. Clear it before choosing a fork boundary.".to_owned())?;
                        return Ok(StateEffect::Redraw);
                    }
                    if !self.allow_session_transition("")? {
                        self.cancel_fork_picker();
                        return Ok(StateEffect::Redraw);
                    }
                    let Some(index) = receipt
                        .identity()
                        .parse::<usize>()
                        .ok()
                        .filter(|index| *index < count)
                    else {
                        self.cancel_fork_picker();
                        self.chat.push_notice(
                            "Fork selection is stale. Open /fork at again.".to_owned(),
                        )?;
                        return Ok(StateEffect::Redraw);
                    };
                    self.fork_boundary_requested = Some((ForkPickerToken(token), index));
                    return Ok(StateEffect::Exit);
                }
                if self.resume_overlay == Some(receipt.token()) {
                    self.resume_overlay = None;
                    if !self.editor.text().is_empty() {
                        self.chat.push_notice("Your newer draft was preserved. Clear it before choosing another session.".to_owned())?;
                        return Ok(StateEffect::Redraw);
                    }
                    return self
                        .handle_resume_command(&format!("/resume {}", receipt.identity()), "");
                }

                if let Some((PendingRequest::Approval(request), token)) = self.request_overlay
                    && token == receipt.token()
                    && self.pending_requests.front() == Some(&PendingRequest::Approval(request))
                {
                    if receipt.identity() == "stop-turn" {
                        return Ok(StateEffect::Dispatch(AgentAction::Interrupt));
                    }
                    if self.chat.approval(request.activity()).is_some() {
                        return self.approval_response(request, receipt.identity().to_owned());
                    }
                    let answer = if receipt.identity() == "approve-request" {
                        "y"
                    } else {
                        "n"
                    };
                    return self.approval_response(request, answer.to_owned());
                }
                if let Some((pending, _)) = self.question_notes
                    && self.request_overlay == Some((pending, receipt.token()))
                    && self.pending_requests.front() == Some(&pending)
                    && receipt.identity() == "send-notes"
                {
                    if self.reject_referenced_answer()? {
                        self.sync_request_overlay()?;
                        return Ok(StateEffect::Redraw);
                    }
                    let notes = self.editor.text().to_owned();
                    self.editor.replace_range(0..notes.len(), "");
                    return self.request_response(pending, notes);
                }
                if let Some((pending @ PendingRequest::UserInput(request), token)) =
                    self.request_overlay
                    && token == receipt.token()
                    && self.pending_requests.front() == Some(&pending)
                    && self
                        .chat
                        .question(request.activity())
                        .is_some_and(|question| {
                            receipt
                                .identity()
                                .parse::<usize>()
                                .ok()
                                .is_some_and(|index| index > 0 && index <= question.choices.len())
                        })
                {
                    if notes_key
                        && self
                            .chat
                            .question(request.activity())
                            .is_some_and(|question| question.allow_notes)
                    {
                        self.question_notes = Some((
                            pending,
                            receipt
                                .identity()
                                .parse()
                                .expect("validated choice ordinal"),
                        ));
                        self.request_overlay = None;
                        self.sync_request_overlay()?;
                        return Ok(StateEffect::Redraw);
                    }
                    return self.request_response(pending, receipt.identity().to_owned());
                }
                if let Some(command) = self.command_palette.accept(&receipt) {
                    let draft = self.editor.text().to_owned();
                    return self.execute_command(command.effect(), command.invocation(), &draft);
                }
                if self.prompt_assist.accept(&receipt, &mut self.editor) {
                    return Ok(StateEffect::Redraw);
                }
                if self.model_overlay == Some(receipt.token()) {
                    self.model_overlay = None;
                    return self.accept_model_selection(receipt.identity());
                }
                self.accepted_overlays.push_back(receipt);
                return Ok(StateEffect::Redraw);
            },
        }

        if self.question_notes.is_some()
            && matches!(&input, InputEvent::Key(key) if key.code == KeyCode::Enter
                && key.modifiers == KeyModifiers::NONE)
        {
            return Ok(StateEffect::Unchanged);
        }

        let editor_vertical = self.views.active() == ObservabilityView::Chat
            && !self.editor.text().is_empty()
            && self.editor.has_layout_width()
            && matches!(&input, InputEvent::Key(key)
                if matches!(key.code, KeyCode::Up | KeyCode::Down)
                    && key.modifiers == KeyModifiers::NONE
                    && matches!(key.action, KeyAction::Press | KeyAction::Repeat));
        if !editor_vertical {
            match self.views.handle_local(&input) {
                ViewInputEffect::Unhandled => {},
                ViewInputEffect::Consumed => return Ok(StateEffect::Unchanged),
                ViewInputEffect::Redraw => return Ok(StateEffect::Redraw),
            }
        }

        if !self.has_pending_request()
            && let InputEvent::Key(key) = &input
            && key.action == KeyAction::Press
            && key.modifiers == KeyModifiers::ALT
        {
            match key.code {
                KeyCode::Enter if !self.editor.newline_binding().matches(key.modifiers) => {
                    return self.queue_follow_up();
                },
                KeyCode::Character('q' | 'Q') => return self.queue_follow_up(),
                KeyCode::Character('r' | 'R') => return self.recall_follow_up(),
                _ => {},
            }
        }
        let previous_text = self.editor.text().to_owned();
        let previous_cursor = self.editor.cursor_byte_index();
        let effect = self.editor.handle(input, self.active_turn.is_some(), now);
        match effect {
            EditorEffect::BufferChanged => {
                let edit = WorkspaceEdit::between(
                    &previous_text,
                    previous_cursor,
                    self.editor.text(),
                    self.editor.cursor_byte_index(),
                );
                let assist_eligible =
                    self.views.active() == ObservabilityView::Chat && !self.has_pending_request();
                let command_eligible = self.views.active() == ObservabilityView::Chat
                    && self.question_notes.is_none()
                    && !(self.restored_question_draft.is_some()
                        && self.restored_question_draft == self.pending_requests.front().copied());
                let request = self.prompt_assist.prompt_changed(
                    &self.editor,
                    &mut self.overlay,
                    edit.as_ref(),
                    assist_eligible,
                );
                self.command_palette.sync(
                    self.editor.text(),
                    self.editor.cursor_byte_index(),
                    &mut self.overlay,
                    command_eligible,
                );
                Ok(
                    request.map_or(StateEffect::Redraw, |request| match request {
                        PromptAssistRequest::Workspace(request) => {
                            StateEffect::WorkspaceSearch(request)
                        },
                        PromptAssistRequest::Skill(request) => StateEffect::SkillSearch(request),
                    }),
                )
            },
            EditorEffect::Submitted(text) => {
                let escaped_palette = self.command_palette.take_escape(&text);
                let restored_answer = self.restored_question_draft.is_some()
                    && self.restored_question_draft == self.pending_requests.front().copied();
                if !escaped_palette
                    && !restored_answer
                    && self.question_notes.is_none()
                    && let Some(argument) = text
                        .strip_prefix("/interview")
                        .filter(|rest| rest.is_empty() || rest.starts_with(' '))
                {
                    self.command_palette.close(&mut self.overlay);
                    return self.handle_interview_command(argument, &text);
                }
                if !escaped_palette && attachment_argument(&text).is_some() {
                    self.command_palette.close(&mut self.overlay);
                    return self.prepare_image_command(&text);
                }
                if !escaped_palette && !restored_answer {
                    if let Some(command) = self
                        .command_palette
                        .exact_submission(&text, previous_cursor)
                    {
                        self.command_palette.close(&mut self.overlay);
                        return self.execute_command(command.effect(), command.invocation(), &text);
                    }
                    if model_argument(&text).is_some() {
                        self.command_palette.close(&mut self.overlay);
                        return self.handle_model_command(&text, &text);
                    }
                    if secrets_argument(&text).is_some() {
                        self.command_palette.close(&mut self.overlay);
                        return self.handle_secrets_command(&text, &text);
                    }
                    if tree_argument(&text).is_some() {
                        self.command_palette.close(&mut self.overlay);
                        return self.handle_tree_command(&text, &text);
                    }
                    if fork_argument(&text).is_some() {
                        self.command_palette.close(&mut self.overlay);
                        return self.handle_fork_command(&text, &text);
                    }
                    if resume_argument(&text).is_some() {
                        self.command_palette.close(&mut self.overlay);
                        return self.handle_resume_command(&text, &text);
                    }
                    if prompt_argument(&text).is_some() {
                        self.command_palette.close(&mut self.overlay);
                        return self.handle_prompt_command(&text, &text);
                    }
                    if compact_argument(&text).is_some() {
                        self.command_palette.close(&mut self.overlay);
                        return self.handle_compact_command(&text, &text);
                    }
                    if self.command_palette.owns_submission(&text, previous_cursor) {
                        self.command_palette.close(&mut self.overlay);
                        self.editor.replace_range(0..0, &text);
                        self.push_unknown_command_notice(text)?;
                        return Ok(StateEffect::Redraw);
                    }
                }
                self.command_palette.close(&mut self.overlay);
                if self.pending_image.is_some() {
                    self.editor.replace_range(0..0, &text);
                    self.chat.push_notice(
                        "Wait for image preparation before submitting; your draft was preserved."
                            .to_owned(),
                    )?;
                    return Ok(StateEffect::Redraw);
                }
                if self.has_pending_request() && self.reject_referenced_answer()? {
                    self.editor.replace_range(0..0, &text);
                    return Ok(StateEffect::Redraw);
                }
                if let Some(request) = self.pending_requests.front().copied() {
                    return self.request_response(request, text);
                }
                if self.starting_submission.is_some() {
                    self.editor.replace_range(0..0, &text);
                    self.chat.push_notice(
                        "Wait for the pending turn to start; your draft was preserved.".to_owned(),
                    )?;
                    return Ok(StateEffect::Redraw);
                }
                if self
                    .pending_submissions
                    .iter()
                    .any(|submission| submission.input().as_str() == text)
                {
                    self.editor.replace_range(0..0, &text);
                    self.chat.push_notice(
                        "This exact draft is already waiting for admission.".to_owned(),
                    )?;
                    return Ok(StateEffect::Redraw);
                }
                let id =
                    SubmissionId::new().map_err(|_| StateError::SubmissionIdentityUnavailable)?;
                let input = match self.prompt_assist.input(&text) {
                    Ok(input) => input,
                    Err(error) => {
                        self.editor.replace_range(0..0, &text);
                        self.chat.push_notice(format!("Reference snapshot is invalid: {error}. Reselect the reference; your draft was preserved."))?;
                        return Ok(StateEffect::Redraw);
                    },
                };
                let submission = InputSubmission::new(id, input);
                self.pending_submissions.push_back(submission.clone());
                self.editor.replace_range(0..0, &text);
                Ok(StateEffect::Dispatch(match self.active_turn {
                    Some(turn) => AgentAction::Steer { turn, submission },
                    None => AgentAction::Submit(submission),
                }))
            },
            EditorEffect::Exit => {
                self.cancel_model_switches();
                Ok(StateEffect::Exit)
            },
            EditorEffect::InterruptTask => Ok(StateEffect::Dispatch(AgentAction::Interrupt)),
            EditorEffect::Unhandled | EditorEffect::NoChange | EditorEffect::ExitArmed => {
                Ok(StateEffect::Unchanged)
            },
        }
    }

    fn handle_secret_input(&mut self, input: InputEvent) -> Result<StateEffect, StateError> {
        let retention = self
            .secret_editor
            .as_ref()
            .map_or(SecretRetention::UseOnce, |editor| editor.retention());
        let effect = self
            .secret_editor
            .as_mut()
            .map_or(SecretEditorEffect::NoChange, |editor| editor.handle(input));
        match effect {
            SecretEditorEffect::Unhandled | SecretEditorEffect::NoChange => {
                Ok(StateEffect::Unchanged)
            },
            SecretEditorEffect::Changed => Ok(StateEffect::Redraw),
            SecretEditorEffect::Rejected => {
                self.chat.push_notice(
                    "Secret input exceeds the 64 KiB UTF-8 limit; the value was not changed."
                        .to_owned(),
                )?;
                Ok(StateEffect::Redraw)
            },
            SecretEditorEffect::RetentionUnavailable => {
                self.chat.push_notice(
                    "Storage needs a verified live account and a secure local vault. Use once remains selected."
                        .to_owned(),
                )?;
                Ok(StateEffect::Redraw)
            },
            SecretEditorEffect::RetentionDaysRequested => {
                self.chat.push_notice(
                    "Enter 1–365 days, then Enter to choose the period. A later Enter submits the secret. Ctrl-S chooses Until deleted."
                        .to_owned(),
                )?;
                Ok(StateEffect::Redraw)
            },
            SecretEditorEffect::RetentionDaysRejected => {
                self.chat
                    .push_notice("Choose a whole number from 1 to 365 days.".to_owned())?;
                Ok(StateEffect::Redraw)
            },
            SecretEditorEffect::RetentionChanged(choice) => {
                let notice = match choice {
                    SecretRetention::UseOnce => "Use once selected. Enter submits the secret.".to_owned(),
                    SecretRetention::UntilDeleted => {
                        "Store until deleted selected. Enter separately saves and submits the secret. Storage may still fail closed."
                            .to_owned()
                    },
                    SecretRetention::ForDays { days, expires_at } => {
                        let expiry = i64::try_from(expires_at).ok()
                            .and_then(|seconds| jiff::Timestamp::from_second(seconds).ok())
                            .map_or_else(|| "unavailable".to_owned(), |timestamp| timestamp.to_string());
                        format!("Store for {days} days selected; expires {expiry}. Enter separately saves and submits the secret.")
                    },
                };
                self.chat.push_notice(notice)?;
                Ok(StateEffect::Redraw)
            },
            SecretEditorEffect::RecoveryDisclosureRequested => {
                let result = self
                    .interview
                    .as_ref()
                    .ok_or_else(|| {
                        InterviewError::Invalid("secret recovery storage is unavailable".into())
                    })
                    .and_then(|controller| controller.recovery_boundary());
                match result {
                    Ok(boundary) => {
                        self.chat.push_notice(format!(
                            "Secret recovery is off by default. Enabling it stores this answer in {boundary}. Press Ctrl-R again to opt in; Enter still submits separately."
                        ))?;
                        if let Some(editor) = &mut self.secret_editor {
                            editor.mark_recovery_disclosed();
                        }
                    },
                    Err(error) => {
                        self.chat.push_notice(error.to_string())?;
                    },
                }
                Ok(StateEffect::Redraw)
            },
            SecretEditorEffect::StoreRecovery(input) => {
                let Some(PendingRequest::SecretInput(request)) =
                    self.pending_requests.front().copied()
                else {
                    return Ok(StateEffect::Unchanged);
                };
                let result = self
                    .interview
                    .as_mut()
                    .ok_or_else(|| {
                        InterviewError::Invalid("secret recovery storage is unavailable".into())
                    })
                    .and_then(|controller| controller.store_secret_recovery(request, &input));
                match result {
                    Ok(()) => {
                        if let Some(editor) = &mut self.secret_editor {
                            editor.mark_recovery_available();
                        }
                        self.chat.push_notice(
                            "Encrypted recovery saved. Enter still requires a fresh explicit submission."
                                .into(),
                        )?;
                    },
                    Err(error) => {
                        self.chat.push_notice(error.to_string())?;
                    },
                }
                Ok(StateEffect::Redraw)
            },
            SecretEditorEffect::RecoverRequested => {
                let Some(PendingRequest::SecretInput(request)) =
                    self.pending_requests.front().copied()
                else {
                    return Ok(StateEffect::Unchanged);
                };
                if let (Some(offer), Some(store), Some(destination)) = (
                    self.chat
                        .question(request.activity())
                        .and_then(|question| question.storage_offer.as_ref()),
                    self.secret_store.as_ref(),
                    self.secret_destination.as_ref(),
                ) {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_or(0, |duration| duration.as_secs());
                    match store.recover(destination, &offer.scope, now) {
                        Ok(Some(input)) => {
                            if let Some(editor) = &mut self.secret_editor {
                                editor.restore(input);
                            }
                            self.chat.push_notice(
                                "Saved value recovered into the hidden editor. Enter still submits it separately."
                                    .to_owned(),
                            )?;
                            return Ok(StateEffect::Redraw);
                        },
                        Err(_) => {
                            self.chat.push_notice(
                                "Saved value unavailable; no secret was inserted.".to_owned(),
                            )?;
                            return Ok(StateEffect::Redraw);
                        },
                        Ok(None) => {},
                    }
                }
                let result = self
                    .interview
                    .as_ref()
                    .ok_or_else(|| {
                        InterviewError::Invalid("secret recovery storage is unavailable".into())
                    })
                    .and_then(|controller| controller.recover_secret(request));
                match result {
                    Ok(input) => {
                        if let Some(editor) = &mut self.secret_editor {
                            editor.restore(input);
                        }
                        self.chat.push_notice(
                            "Secret recovered into the hidden editor. Press Enter to submit it."
                                .into(),
                        )?;
                    },
                    Err(error) => {
                        self.chat.push_notice(error.to_string())?;
                    },
                }
                Ok(StateEffect::Redraw)
            },
            SecretEditorEffect::ForgetRecovery => {
                let request = self
                    .pending_requests
                    .front()
                    .and_then(|pending| match pending {
                        PendingRequest::SecretInput(request) => Some(*request),
                        _ => None,
                    });
                let result = self
                    .interview
                    .as_mut()
                    .ok_or_else(|| {
                        InterviewError::Invalid("secret recovery storage is unavailable".into())
                    })
                    .and_then(|controller| controller.forget_secret_recovery(request));
                match result {
                    Ok(warning) => {
                        if let Some(editor) = &mut self.secret_editor {
                            editor.mark_recovery_forgotten();
                        }
                        self.chat.push_notice(match warning {
                            Some(warning) => {
                                format!("Secret recovery reference removed; {warning}")
                            },
                            None => "Secret recovery forgotten.".into(),
                        })?;
                    },
                    Err(error) => {
                        self.chat.push_notice(error.to_string())?;
                    },
                }
                Ok(StateEffect::Redraw)
            },
            SecretEditorEffect::Cancel => {
                self.clear_secret_editor();
                Ok(StateEffect::Dispatch(AgentAction::Interrupt))
            },
            SecretEditorEffect::Exit => {
                self.clear_secret_editor();
                Ok(StateEffect::Exit)
            },
            SecretEditorEffect::Submitted(input) => {
                let Some(PendingRequest::SecretInput(request)) =
                    self.pending_requests.front().copied()
                else {
                    self.clear_secret_editor();
                    return Ok(StateEffect::Unchanged);
                };
                if retention != SecretRetention::UseOnce {
                    let result = (|| {
                        let question = self
                            .chat
                            .question(request.activity())
                            .ok_or("secret request presentation is unavailable")?;
                        let offer = question
                            .storage_offer
                            .as_ref()
                            .ok_or("secret storage offer is unavailable")?;
                        let title = question
                            .plain_text
                            .lines()
                            .next()
                            .ok_or("secret storage title is unavailable")?;
                        let store = self
                            .secret_store
                            .as_ref()
                            .ok_or("secret storage is unavailable")?;
                        let destination = self
                            .secret_destination
                            .as_ref()
                            .ok_or("verified secret destination is unavailable")?;
                        let policy = match retention {
                            SecretRetention::UseOnce => unreachable!("guarded retention"),
                            SecretRetention::ForDays { days, expires_at } => {
                                RetentionPolicy::ForDaysAt { days, expires_at }
                            },
                            SecretRetention::UntilDeleted => RetentionPolicy::UntilDeleted,
                        };
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map_err(|_| "current time is unavailable")?
                            .as_secs();
                        store
                            .save(destination, &offer.scope, title, policy, &input, now)
                            .map_err(|_| "encrypted secret storage failed")?;
                        Ok::<(), &str>(())
                    })();
                    if let Err(detail) = result {
                        if let Some(editor) = &mut self.secret_editor {
                            editor.restore_unsubmitted(input);
                        }
                        self.chat.push_notice(format!(
                            "{detail}. The secret was not submitted; choose Use once or try again."
                        ))?;
                        return Ok(StateEffect::Redraw);
                    }
                }
                self.pending_requests.pop_front();
                self.clear_secret_editor();
                self.close_request_overlay();
                self.sync_request_overlay()?;
                Ok(StateEffect::Dispatch(AgentAction::RespondToSecretInput {
                    request,
                    input,
                }))
            },
        }
    }

    fn is_secret_previous_question_input(&self, input: &InputEvent) -> bool {
        matches!(
            input,
            InputEvent::Key(key)
                if key.code == KeyCode::BackTab
                    && key.action == KeyAction::Press
                    && (key.modifiers == KeyModifiers::NONE
                        || key.modifiers == KeyModifiers::SHIFT)
        ) && self
            .pending_requests
            .front()
            .and_then(|pending| match pending {
                PendingRequest::SecretInput(request) => self.chat.question(request.activity()),
                _ => None,
            })
            .is_some_and(|question| question.previous_question)
    }

    fn handle_secret_previous_question(&mut self) -> Result<StateEffect, StateError> {
        let Some(PendingRequest::SecretInput(request)) = self.pending_requests.pop_front() else {
            return Ok(StateEffect::Unchanged);
        };
        // Previous-question navigation carries no draft for a secret request. Clear before
        // dispatching so the transient value cannot be retained by the interview controller.
        self.clear_secret_editor();
        self.close_request_overlay();
        self.sync_request_overlay()?;
        Ok(StateEffect::Dispatch(AgentAction::PreviousQuestion {
            request,
            choice: None,
            draft: String::new(),
        }))
    }

    fn handle_presentation_blocked(
        &mut self,
        input: InputEvent,
    ) -> Result<StateEffect, StateError> {
        if matches!(
            input,
            InputEvent::Key(key)
                if key.action == KeyAction::Press
                    && ((key.code == KeyCode::Escape && key.modifiers == KeyModifiers::NONE)
                        || (matches!(key.code, KeyCode::Character('c' | 'C'))
                            && key.modifiers == KeyModifiers::CONTROL))
        ) {
            return Ok(StateEffect::Dispatch(AgentAction::Interrupt));
        }
        // Until the typed question arrives, ordinary drafts, palettes, and submit keys are
        // intentionally inert. A malformed presentation remains blocked for the same reason.
        Ok(StateEffect::Unchanged)
    }

    pub(in crate::runner) fn observe_submission_outcome(
        &mut self,
        outcome: SubmissionOutcome,
    ) -> Result<StateEffect, StateError> {
        let Some(index) = self
            .pending_submissions
            .iter()
            .position(|submission| submission.id() == outcome.id())
        else {
            return Ok(StateEffect::Unchanged);
        };
        let submission = self
            .pending_submissions
            .remove(index)
            .expect("the located submission must still exist");
        let queued = self.follow_up_submission == Some(submission.id());
        if queued {
            self.follow_up_submission = None;
        }
        match outcome {
            SubmissionOutcome::Accepted { .. } => {
                if queued {
                    self.follow_ups.pop_front();
                } else if self.prompt_assist.input(self.editor.text()).as_ref().ok()
                    == Some(submission.input())
                {
                    self.clear_editor();
                }
                Ok(StateEffect::Redraw)
            },
            SubmissionOutcome::Rejected { rejection, .. } => {
                if self.starting_submission == Some(submission.id()) {
                    self.starting_submission = None;
                }
                self.follow_ups_paused = true;
                self.chat
                    .push_notice(format!("Submission rejected: {}", rejection.message()))?;
                if self.active_turn.is_none()
                    && self.pending_submissions.is_empty()
                    && !self.has_pending_request()
                    && let Some(selection) = self.reserved_model_selection.take()
                {
                    self.pending_model_selection = Some(selection);
                    return Ok(StateEffect::Exit);
                }
                Ok(StateEffect::Redraw)
            },
        }
    }

    pub(super) fn queue_follow_up(&mut self) -> Result<StateEffect, StateError> {
        if self.pending_image.is_some() {
            self.chat.push_notice(
                "Wait for image preparation before queueing; your draft was preserved.".to_owned(),
            )?;
            return Ok(StateEffect::Redraw);
        }
        let text = self.editor.text();
        if text.is_empty() {
            self.follow_ups_paused = false;
            return Ok(StateEffect::Redraw);
        }
        if text.trim_start().starts_with('/') {
            self.chat
                .push_notice("Commands cannot be queued; the draft was preserved.".to_owned())?;
            return Ok(StateEffect::Redraw);
        }
        if !self.pending_submissions.is_empty() {
            self.chat
                .push_notice("Wait for input admission before queuing this draft.".to_owned())?;
            return Ok(StateEffect::Redraw);
        }
        let bytes = self
            .follow_ups
            .iter()
            .map(|input| input.as_str().len())
            .sum::<usize>();
        if self.follow_ups.len() >= FOLLOW_UP_LIMIT
            || text.len() > FOLLOW_UP_BYTES.saturating_sub(bytes)
        {
            self.chat.push_notice(
                "Follow-up queue is full (16 messages / 64 KiB). The draft was preserved."
                    .to_owned(),
            )?;
            return Ok(StateEffect::Redraw);
        }
        let input = match self.prompt_assist.input(text) {
            Ok(input) => input,
            Err(error) => {
                self.chat.push_notice(format!("Reference snapshot is invalid: {error}. Reselect the reference; your draft was preserved."))?;
                return Ok(StateEffect::Redraw);
            },
        };
        self.follow_ups.push_back(input);
        let length = text.len();
        self.editor.replace_range(0..length, "");
        self.prompt_assist.prompt_cleared(&mut self.overlay);
        // queue에 추가하는 동작은 일시 중지된 queue 뒤에도 계속하겠다는 의도를 명시한다.
        self.follow_ups_paused = false;
        Ok(StateEffect::Redraw)
    }

    pub(super) fn recall_follow_up(&mut self) -> Result<StateEffect, StateError> {
        self.follow_ups_paused = true;
        if !self.pending_submissions.is_empty() || !self.editor.text().is_empty() {
            self.chat.push_notice("Queue paused. Empty the draft and wait for admission, then Alt+R recalls the next message.".to_owned())?;
        } else if let Some(input) = self.follow_ups.pop_front() {
            self.editor.replace_range(0..0, input.as_str());
            self.prompt_assist.restore_input(&input, &mut self.overlay);
        }
        Ok(StateEffect::Redraw)
    }

    /// 유휴 상태이고 일시 중지되지 않은 queue snapshot 하나만 기존 admission lane으로 옮긴다.
    pub(in crate::runner) fn next_follow_up(&mut self) -> Result<Option<AgentAction>, StateError> {
        if self.preview.is_some()
            || self.follow_ups_paused
            || self.active_turn.is_some()
            || self.has_pending_request()
            || !self.pending_submissions.is_empty()
            || self.starting_submission.is_some()
            || self.pending_model_selection.is_some()
            || self.reserved_model_selection.is_some()
        {
            return Ok(None);
        }
        let Some(input) = self.follow_ups.front() else {
            return Ok(None);
        };
        let id = SubmissionId::new().map_err(|_| StateError::SubmissionIdentityUnavailable)?;
        let submission = InputSubmission::new(id, input.clone());
        self.follow_up_submission = Some(id);
        self.starting_submission = Some(id);
        self.pending_submissions.push_back(submission.clone());
        Ok(Some(AgentAction::Submit(submission)))
    }
}
