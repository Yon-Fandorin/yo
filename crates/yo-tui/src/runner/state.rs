use std::{collections::VecDeque, time::Duration};

use yo_core::{
    ActivityApproval, ActivityDocument, ActivityKind, ActivityNotice, ActivityQuestion,
    ActivityRef, ActivityRequestRef, AgentControlOutcome, AgentEvent, ApprovalDecision,
    DurabilityGapCause, ImagePreparationRequest, InputSubmission, JournalDurability,
    RequestTraceEntry, SkillReferenceSearchRequest, SkillReferenceSearchUpdate, SubmissionId,
    SubmissionOutcome, TranscriptRecord, TurnOutcome, TurnRef, UserInput,
    WorkspaceReferenceSearchRequest, WorkspaceReferenceSearchUpdate,
    session_repository::{DurableCutoff, InheritedSessionHistory},
};

use crate::{
    PromptTemplates,
    appearance::AppearancePin,
    command::{
        CommandEffect, CommandPalette, CommandRegistry, attachment_argument, compact_argument,
        fork_argument, model_argument, prompt_argument, resume_argument, tree_argument,
    },
    input::{
        editor::{EditorEffect, PromptEditor},
        event::{InputEvent, KeyAction, KeyCode, KeyModifiers},
    },
    overlay::{
        AcceptanceReceipt, OverlayInputEffect, OverlayInstanceToken, PanelSnapshot,
        PromptOverlaySlot, SelectionEntry, SelectionPanel, SlotError,
    },
    prompt::{
        assist::{PromptAssistController, PromptAssistRequest},
        workspace_reference::WorkspaceEdit,
    },
    runner::{
        AgentAction, ForkPickerToken, PresentationMode,
        chat::{ChatProjection, ChatProjectionChange},
        model::ModelSelectionState,
        session::{TuiDocument, TuiSessionInfo, TuiStatusLine},
        view::{ObservabilityView, ObservabilityViews, ViewInputEffect},
    },
    surface::Size,
    transcript::{TranscriptMeasureError, TranscriptStateError},
};

mod image;
mod presentation;
mod preview;

pub(super) use presentation::{FrameError, MotionDemand, PreparedFrame};

const FOLLOW_UP_LIMIT: usize = 16;
const FOLLOW_UP_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum StateEffect {
    Unchanged,
    Redraw,
    Dispatch(AgentAction),
    Suspend,
    Exit,
    Resize(Size),
    WorkspaceSearch(WorkspaceReferenceSearchRequest),
    SkillSearch(SkillReferenceSearchRequest),
    PrepareImage(ImagePreparationRequest),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StateError {
    Transcript(TranscriptStateError),
    UnknownActivity(ActivityRef),
    ItemIdOverflow,
    SubmissionIdentityUnavailable,
    StalePublication,
    PreviewAgent,
    RequestPanel(SlotError),
}

#[derive(Debug, Default)]
pub(super) struct TuiState {
    preview: Option<Box<preview::Preview>>,
    preview_mode: bool,
    chat: ChatProjection,
    editor: PromptEditor,
    views: ObservabilityViews,
    pending_requests: VecDeque<PendingRequest>,
    request_overlay: Option<(PendingRequest, OverlayInstanceToken)>,
    request_panel: Option<PanelSnapshot>,
    saved_request_panel: Option<(PendingRequest, PanelSnapshot, SelectionPanel)>,
    question_notes: Option<(PendingRequest, u32)>,
    question_notes_refresh: Option<PendingRequest>,
    restored_question_draft: Option<PendingRequest>,
    active_turn: Option<TurnRef>,
    durability: Option<JournalDurability>,
    session_info: TuiSessionInfo,
    host_status: TuiStatusLine,
    presentation_mode: PresentationMode,
    overlay: PromptOverlaySlot,
    command_palette: CommandPalette,
    accepted_overlays: VecDeque<AcceptanceReceipt>,
    prompt_assist: PromptAssistController,
    prompt_templates: PromptTemplates,
    pending_submissions: VecDeque<InputSubmission>,
    follow_ups: VecDeque<UserInput>,
    follow_ups_paused: bool,
    follow_up_submission: Option<SubmissionId>,
    starting_submission: Option<SubmissionId>,
    image_preparation_enabled: bool,
    pending_image: Option<image::PendingImage>,
    next_image_id: u64,
    new_session_requested: bool,
    fork_session_requested: bool,
    fork_picker_requested: bool,
    fork_boundary_requested: Option<(ForkPickerToken, usize)>,
    fork_overlay: Option<(OverlayInstanceToken, usize)>,
    fork_draft: Option<String>,
    session_tree_requested: bool,
    resume_session_requested: Option<Option<yo_core::SessionId>>,
    resume_overlay: Option<OverlayInstanceToken>,
    context_compaction_pending: bool,
    model_selection: Option<ModelSelectionState>,
    model_overlay: Option<OverlayInstanceToken>,
    pending_model_selection: Option<yo_core::ModelPickerTarget>,
    reserved_model_selection: Option<yo_core::ModelPickerTarget>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingRequest {
    Approval(ActivityRequestRef),
    UserInput(ActivityRequestRef),
}

impl TuiState {
    pub(super) fn set_prompt_templates(&mut self, templates: PromptTemplates) {
        self.prompt_templates = templates;
    }

    pub(super) fn set_status_line(&mut self, status: TuiStatusLine) -> bool {
        if self.host_status == status {
            return false;
        }
        self.host_status = status;
        true
    }

    #[cfg(test)]
    pub(super) fn new() -> Self {
        Self::with_session_info(TuiSessionInfo::default())
    }

    pub(super) fn with_session_info(session_info: TuiSessionInfo) -> Self {
        let notice = session_info.startup_notice();
        let mut state = Self {
            chat: ChatProjection::new(),
            session_info,
            ..Self::default()
        };
        if let Some(notice) = notice {
            state
                .observe_notice(notice)
                .expect("bounded notice in a new projection");
        }
        state
    }

    pub(super) fn set_presentation_mode(&mut self, mode: PresentationMode) {
        self.presentation_mode = mode;
    }

    pub(super) fn handle(
        &mut self,
        input: InputEvent,
        now: Duration,
    ) -> Result<StateEffect, StateError> {
        let effect = self.handle_input(input, now)?;
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
        // Reviewing retained changes does not submit an approval. Its command palette
        // replaces the request panel, so it must not require that panel's commit token.
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
                .map(|(_, choice)| choice);
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

        // Typed replies and local commands retain their existing editor path.
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

        match self.views.handle_local(&input) {
            ViewInputEffect::Unhandled => {},
            ViewInputEffect::Consumed => return Ok(StateEffect::Unchanged),
            ViewInputEffect::Redraw => return Ok(StateEffect::Redraw),
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

    pub(super) fn observe_submission_outcome(
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

    fn queue_follow_up(&mut self) -> Result<StateEffect, StateError> {
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
        // Queuing is explicit intent to continue, including after a paused queue.
        self.follow_ups_paused = false;
        Ok(StateEffect::Redraw)
    }

    fn recall_follow_up(&mut self) -> Result<StateEffect, StateError> {
        self.follow_ups_paused = true;
        if !self.pending_submissions.is_empty() || !self.editor.text().is_empty() {
            self.chat.push_notice("Queue paused. Empty the draft and wait for admission, then Alt+R recalls the next message.".to_owned())?;
        } else if let Some(input) = self.follow_ups.pop_front() {
            self.editor.replace_range(0..0, input.as_str());
            self.prompt_assist.restore_input(&input, &mut self.overlay);
        }
        Ok(StateEffect::Redraw)
    }

    /// Transfers only one idle, unpaused queued snapshot to the existing admission lane.
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

    pub(super) fn observe_control_outcome(
        &mut self,
        outcome: AgentControlOutcome,
    ) -> Result<StateEffect, StateError> {
        match outcome {
            AgentControlOutcome::ContextCompactionRejected { detail } => {
                self.context_compaction_pending = false;
                self.chat
                    .push_notice(format!("Context compaction was not started.\n{detail}"))?;
            },
        }
        Ok(StateEffect::Redraw)
    }

    pub(super) fn observe_inherited_history(
        &mut self,
        history: &InheritedSessionHistory,
    ) -> Result<(), StateError> {
        self.chat.observe_inherited_history(history)?;
        self.views
            .observe_inherited_history(history)
            .map_err(StateError::Transcript)
    }

    pub(super) fn observe_document(
        &mut self,
        document: TuiDocument,
    ) -> Result<StateEffect, StateError> {
        let expanded = document.expanded();
        let item = self.chat.push_document(document)?;
        if let Some(expanded) = expanded {
            self.views.set_item_expansion(item, expanded);
        }
        Ok(StateEffect::Redraw)
    }

    pub(super) fn observe_notice(
        &mut self,
        notice: ActivityNotice,
    ) -> Result<StateEffect, StateError> {
        self.chat.push_session_notice(notice)?;
        Ok(StateEffect::Redraw)
    }

    pub(super) fn observe_record(
        &mut self,
        record: TranscriptRecord,
    ) -> Result<StateEffect, StateError> {
        let lifecycle_effect = self.observe_live_lifecycle(&record)?;
        let chat_change = self.chat.observe_record(&record)?;
        if let TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated { activity, .. }) =
            &record
            && self
                .saved_request_panel
                .as_ref()
                .is_some_and(|(pending, _, _)| pending.activity() == *activity)
        {
            self.saved_request_panel = None;
        }
        if let TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated { activity, .. }) =
            &record
            && self
                .question_notes
                .is_some_and(|(pending, _)| pending.activity() == *activity)
        {
            self.question_notes_refresh = self.question_notes.map(|(pending, _)| pending);
            self.question_notes = None;
        }
        self.sync_request_overlay()?;
        if let TranscriptRecord::EventCommitted(
            AgentEvent::ActivityUpdated { activity, .. }
            | AgentEvent::ActivityFinished { activity, .. },
        ) = &record
            && let Some((pending, token)) = self.request_overlay
            && (pending.activity() == *activity
                || (pending.activity().turn() == activity.turn()
                    && self
                        .chat
                        .approval(pending.activity())
                        .is_some_and(|profile| {
                            profile.related_change == Some(activity.activity_id().get().get())
                        })))
        {
            // A linked diff is approval context even when the request profile is unchanged.
            // Refreshing the revision rejects commits from a frame prepared before the change.
            self.overlay
                .refresh(
                    token,
                    self.request_panel
                        .clone()
                        .expect("current request has a panel"),
                )
                .map_err(StateError::RequestPanel)?;
        }
        let effect = record_effect(&record);
        self.views
            .observe_record(
                &record,
                match chat_change {
                    ChatProjectionChange::Unchanged => None,
                    ChatProjectionChange::VisibleItem(item) => Some(item),
                },
            )
            .map_err(StateError::Transcript)?;
        Ok(match lifecycle_effect {
            StateEffect::Exit => StateEffect::Exit,
            StateEffect::Redraw => StateEffect::Redraw,
            _ => effect,
        })
    }

    pub(super) fn observe_request_trace(&mut self, entry: RequestTraceEntry) {
        self.views.observe_request_trace(entry);
    }

    pub(super) fn observe_durability(
        &mut self,
        durability: JournalDurability,
    ) -> Result<StateEffect, StateError> {
        if self.durability == Some(durability) {
            return Ok(StateEffect::Unchanged);
        }
        let was_gap = matches!(self.durability, Some(JournalDurability::Gap { .. }));
        self.durability = Some(durability);
        match durability {
            JournalDurability::Gap {
                durable_cutoff,
                cause,
            } => {
                let reason = match cause {
                    DurabilityGapCause::Capacity => "History storage is full.",
                    DurabilityGapCause::Storage => "History storage is unavailable.",
                    DurabilityGapCause::Integrity => "A history record failed validation.",
                };
                let saved = match durable_cutoff {
                    DurableCutoff::Known {
                        journal_sequence: Some(sequence),
                        repository_sequence,
                    } => {
                        format!(
                            "Saved through event {} (storage record {}).",
                            sequence.get(),
                            repository_sequence.get()
                        )
                    },
                    DurableCutoff::Known {
                        journal_sequence: None,
                        repository_sequence,
                    } => {
                        format!(
                            "Only session metadata is saved (storage record {}).",
                            repository_sequence.get()
                        )
                    },
                    DurableCutoff::KnownEmpty => "No session history has been saved.".to_owned(),
                    DurableCutoff::Unknown => {
                        "The last saved point could not be verified.".to_owned()
                    },
                };
                self.chat.push_notice(format!(
                    "History not saved\n{reason}\n{saved}\nNew activity stays in memory. Copy important output before closing yo."
                ))?;
                Ok(StateEffect::Redraw)
            },
            JournalDurability::Durable { .. } if was_gap => {
                self.chat.push_notice(
                    "History saving recovered. The complete session has been saved.".to_owned(),
                )?;
                Ok(StateEffect::Redraw)
            },
            _ if was_gap => Ok(StateEffect::Redraw),
            _ => Ok(StateEffect::Unchanged),
        }
    }

    pub(super) fn enable_workspace_references(&mut self) {
        self.prompt_assist.enable_workspace();
    }

    pub(super) fn enable_skill_references(&mut self) {
        self.prompt_assist.enable_skill();
    }

    pub(super) fn observe_workspace_reference_update(
        &mut self,
        update: WorkspaceReferenceSearchUpdate,
    ) -> StateEffect {
        if self
            .prompt_assist
            .observe_workspace(update, &mut self.overlay)
        {
            StateEffect::Redraw
        } else {
            StateEffect::Unchanged
        }
    }

    pub(super) fn observe_workspace_reference_failure(&mut self, reason: String) -> StateEffect {
        if self
            .prompt_assist
            .workspace_failed(reason, &mut self.overlay)
        {
            StateEffect::Redraw
        } else {
            StateEffect::Unchanged
        }
    }

    pub(super) fn observe_skill_reference_update(
        &mut self,
        update: SkillReferenceSearchUpdate,
    ) -> StateEffect {
        if self.prompt_assist.observe_skill(update, &mut self.overlay) {
            StateEffect::Redraw
        } else {
            StateEffect::Unchanged
        }
    }

    pub(super) fn observe_skill_reference_failure(&mut self, reason: String) -> StateEffect {
        if self.prompt_assist.skill_failed(reason, &mut self.overlay) {
            StateEffect::Redraw
        } else {
            StateEffect::Unchanged
        }
    }

    #[cfg(test)]
    pub(super) const fn durability(&self) -> Option<JournalDurability> {
        self.durability
    }

    #[cfg(test)]
    pub(super) fn observe(&mut self, event: AgentEvent) -> Result<StateEffect, StateError> {
        self.observe_record(TranscriptRecord::EventCommitted(event))
    }

    fn observe_live_lifecycle(
        &mut self,
        record: &TranscriptRecord,
    ) -> Result<StateEffect, StateError> {
        if matches!(record, TranscriptRecord::ContextCheckpointCommitted(_)) {
            self.context_compaction_pending = false;
        }
        let TranscriptRecord::EventCommitted(event) = record else {
            return Ok(StateEffect::Unchanged);
        };
        match event {
            AgentEvent::TurnStarted { turn } => {
                self.active_turn = Some(*turn);
                self.starting_submission = None;
            },
            AgentEvent::ActivityStarted { activity, kind } => {
                let request = match kind {
                    ActivityKind::ApprovalRequest { request_id } => Some(PendingRequest::Approval(
                        ActivityRequestRef::new(*activity, *request_id),
                    )),
                    ActivityKind::UserInputRequest { request_id } => Some(
                        PendingRequest::UserInput(ActivityRequestRef::new(*activity, *request_id)),
                    ),
                    _ => None,
                };
                if let Some(request) = request {
                    if self.pending_requests.is_empty() {
                        self.overlay.close_current();
                        self.command_palette.dismiss();
                        self.model_overlay = None;
                        self.prompt_assist.cancel();
                    }
                    self.pending_requests.push_back(request);
                }
            },
            AgentEvent::ActivityFinished { activity, .. } => {
                self.pending_requests
                    .retain(|request| request.activity() != *activity);
            },
            AgentEvent::TurnFinished { turn, outcome } if self.active_turn == Some(*turn) => {
                self.active_turn = None;
                if *outcome != TurnOutcome::Completed {
                    self.follow_ups_paused = true;
                }
                if let Some(selection) = self.reserved_model_selection.take() {
                    if matches!(self.durability, Some(JournalDurability::Durable { .. })) {
                        self.pending_model_selection = Some(selection);
                        return Ok(StateEffect::Exit);
                    }
                    self.chat.push_notice(
                        "The reserved model was not applied because durable Turn completion could not be established; the previous model remains active."
                            .to_owned(),
                    )?;
                    return Ok(StateEffect::Redraw);
                }
            },
            AgentEvent::TurnFinished { .. } => {},
            AgentEvent::SessionCreated { .. } | AgentEvent::ActivityUpdated { .. } => {},
        }
        Ok(StateEffect::Unchanged)
    }

    // This is the currently rendered Chat projection. Future Transcript and Request views select
    // their own projections above this state instead of changing the generic RunOutcome boundary.
    pub(super) fn session_output(
        &self,
        appearance: &AppearancePin,
    ) -> Result<Option<String>, TranscriptMeasureError> {
        let transcript = self.chat.transcript();
        transcript.plain_output_slice(
            transcript.suffix(self.chat.published_item_count()),
            appearance.snapshot().transcript_config(),
        )
    }

    pub(super) fn has_pending_request(&self) -> bool {
        !self.pending_requests.is_empty()
    }

    pub(super) fn wants_overlay_input(&self, input: &InputEvent) -> bool {
        self.overlay.wants_input(input)
    }

    pub(super) fn wants_global_input(&self, input: &InputEvent) -> bool {
        self.views.wants_global_input(input)
    }

    pub(super) fn open_overlay(
        &mut self,
        snapshot: PanelSnapshot,
    ) -> Result<OverlayInstanceToken, SlotError> {
        if self.views.active() != ObservabilityView::Chat {
            return Err(SlotError::ChatNotVisible);
        }
        if self.has_pending_request() {
            return Err(SlotError::AgentInteractionPending);
        }
        self.command_palette.close(&mut self.overlay);
        self.overlay.open(snapshot)
    }

    pub(super) fn refresh_overlay(
        &mut self,
        token: OverlayInstanceToken,
        snapshot: PanelSnapshot,
    ) -> Result<(), SlotError> {
        self.overlay.refresh(token, snapshot)
    }

    pub(super) fn close_overlay(&mut self, token: OverlayInstanceToken) -> Result<(), SlotError> {
        self.overlay.close(token)
    }

    pub(super) fn take_overlay_acceptance(&mut self) -> Option<AcceptanceReceipt> {
        self.accepted_overlays.pop_front()
    }

    pub(super) fn enable_model_selection(&mut self, controller: yo_core::ModelSelectionController) {
        self.model_selection = Some(ModelSelectionState::new(controller));
    }

    pub(super) fn take_session_tree_request(&mut self) -> bool {
        std::mem::take(&mut self.session_tree_requested)
    }

    pub(super) fn report_session_tree_failure(&mut self, detail: String) {
        let _ = self
            .chat
            .push_notice(format!("Session tree could not be opened: {detail}"));
    }

    pub(super) fn take_fork_session_request(&mut self) -> bool {
        std::mem::take(&mut self.fork_session_requested)
    }

    pub(super) fn take_fork_picker_request(&mut self) -> bool {
        std::mem::take(&mut self.fork_picker_requested)
    }

    pub(super) fn take_fork_boundary_request(&mut self) -> Option<(ForkPickerToken, usize)> {
        self.fork_boundary_requested.take()
    }

    pub(super) fn show_fork_picker(
        &mut self,
        panel: PanelSnapshot,
        row_count: usize,
    ) -> Result<ForkPickerToken, String> {
        if self.fork_draft.is_none() || !self.editor.text().is_empty() {
            self.cancel_fork_picker();
            return Err("fork picker request is stale or a newer draft is present".to_owned());
        }
        if !self
            .allow_session_transition("")
            .map_err(|error| format!("fork picker: {error:?}"))?
        {
            self.cancel_fork_picker();
            return Err("fork picker requires an idle durable Session".to_owned());
        }
        let token = self
            .open_overlay(panel)
            .map_err(|error| format!("fork picker: {error:?}"))?;
        self.fork_overlay = Some((token, row_count));
        Ok(ForkPickerToken(token))
    }

    pub(super) fn cancel_fork_picker(&mut self) {
        if let Some((token, _)) = self.fork_overlay.take() {
            let _ = self.overlay.close(token);
        }
        self.fork_picker_requested = false;
        self.fork_boundary_requested = None;
        if let Some(draft) = self.fork_draft.take()
            && self.editor.text().is_empty()
        {
            self.restore_draft(&draft);
        }
    }

    pub(super) fn report_fork_started(&mut self, parent: yo_core::SessionId) {
        self.fork_draft = None;
        let _ = self.chat.push_notice(format!(
            "Forked from {parent}. Inherited context is preserved."
        ));
    }

    pub(super) fn report_fork_failure(&mut self, detail: String) {
        self.cancel_fork_picker();
        let _ = self.chat.push_notice(format!(
            "Fork was not started; your current session remains open: {detail}"
        ));
    }

    pub(super) fn report_fork_cleanup_failure(&mut self, detail: String) {
        let _ = self.chat.push_notice(format!(
            "Fork started, but previous session cleanup failed: {detail}"
        ));
    }

    pub(super) fn take_new_session_request(&mut self) -> bool {
        std::mem::take(&mut self.new_session_requested)
    }

    pub(super) fn report_new_session_cleanup_failure(&mut self, detail: String) {
        let _ = self.chat.push_notice(format!(
            "New session started, but previous session cleanup failed: {detail}"
        ));
    }

    pub(super) fn report_new_session_failure(&mut self, detail: String) {
        let _ = self.chat.push_notice(format!(
            "New session was not started; your current session remains open: {detail}"
        ));
    }

    pub(super) fn take_model_selection(&mut self) -> Option<yo_core::ModelPickerTarget> {
        self.pending_model_selection.take()
    }

    pub(super) fn report_model_switch_failure(&mut self, detail: String) {
        let _ = self.chat.push_notice(format!(
            "Model switch failed; the previous model remains active: {detail}"
        ));
    }

    pub(super) fn commit_model_switch(
        &mut self,
        controller: yo_core::ModelSelectionController,
        backend_label: String,
        cleanup_warning: Option<String>,
    ) {
        self.model_selection = Some(ModelSelectionState::new(controller));
        self.session_info.set_backend(backend_label.clone());
        let mut notice = format!("Model switched to {backend_label}.");
        if let Some(warning) = cleanup_warning {
            notice.push_str(&format!(" Previous backend cleanup warning: {warning}"));
        }
        let _ = self.chat.push_notice(notice);
    }

    fn handle_model_command(&mut self, text: &str, draft: &str) -> Result<StateEffect, StateError> {
        let Some(selection) = self.model_selection.as_ref() else {
            self.chat.push_notice(
                "No configured model catalog is available for this Session.".to_owned(),
            )?;
            self.restore_draft(draft);
            return Ok(StateEffect::Redraw);
        };
        let argument = model_argument(text).expect("command syntax checked");
        if argument.is_empty() {
            let panel = selection.panel();
            match panel {
                Ok(panel) => match self.overlay.open(panel) {
                    Ok(token) => {
                        self.model_overlay = Some(token);
                        self.clear_editor();
                    },
                    Err(error) => {
                        self.chat.push_notice(format!(
                            "The model picker could not be opened: {error:?}"
                        ))?;
                        self.restore_draft(draft);
                    },
                },
                Err(error) => {
                    self.chat.push_notice(error)?;
                    self.restore_draft(draft);
                },
            }
            return Ok(StateEffect::Redraw);
        }
        let resolved = selection.resolve_direct(argument).map(|selected| {
            let is_current = selection.is_current(&selected);
            (selected, is_current)
        });
        match resolved {
            Ok((selected, true)) => self.accept_current_model(selected),
            Ok((selected, false)) => {
                self.clear_editor();
                self.admit_model_selection(selected)
            },
            Err(error) => {
                self.chat
                    .push_notice(format!("Model switch rejected: {error}"))?;
                self.restore_draft(draft);
                Ok(StateEffect::Redraw)
            },
        }
    }

    pub(super) fn take_resume_session_request(&mut self) -> Option<Option<yo_core::SessionId>> {
        self.resume_session_requested.take()
    }

    pub(super) fn show_resume_picker(&mut self, panel: PanelSnapshot) -> Result<(), SlotError> {
        self.resume_overlay = Some(self.open_overlay(panel)?);
        Ok(())
    }

    pub(super) fn report_resume_failure(&mut self, detail: String) {
        let _ = self.chat.push_notice(format!(
            "Session was not resumed; your current conversation remains open: {detail}"
        ));
    }

    pub(super) fn report_resume_cleanup_failure(&mut self, detail: String) {
        let _ = self.chat.push_notice(format!(
            "Saved session resumed, but previous session cleanup failed: {detail}"
        ));
    }

    fn allow_session_transition(&mut self, draft: &str) -> Result<bool, StateError> {
        if !matches!(self.durability, Some(JournalDurability::Durable { .. })) {
            self.restore_draft(draft);
            self.chat.push_notice("Cannot switch sessions until this conversation has durable history. Your current session and draft were preserved.".to_owned())?;
            return Ok(false);
        }
        if self.preview_mode
            || self.context_compaction_pending
            || self.active_turn.is_some()
            || self.starting_submission.is_some()
            || !self.pending_submissions.is_empty()
            || self.has_pending_request()
            || !self.follow_ups.is_empty()
            || self.pending_model_selection.is_some()
            || self.reserved_model_selection.is_some()
        {
            self.restore_draft(draft);
            self.chat.push_notice("Cannot switch sessions during a turn, compaction, pending input, queued messages, model switch, or preview. Finish or recall pending work first; your input was preserved.".to_owned())?;
            return Ok(false);
        }
        Ok(true)
    }

    fn handle_tree_command(&mut self, text: &str, draft: &str) -> Result<StateEffect, StateError> {
        if tree_argument(text).is_none_or(|argument| !argument.is_empty()) {
            self.restore_draft(draft);
            self.chat.push_notice(
                "Use /tree without arguments to inspect session branches.".to_owned(),
            )?;
            return Ok(StateEffect::Redraw);
        }
        if !self.allow_session_transition(draft)? {
            return Ok(StateEffect::Redraw);
        }
        self.clear_editor();
        self.session_tree_requested = true;
        Ok(StateEffect::Exit)
    }

    fn handle_fork_command(&mut self, text: &str, draft: &str) -> Result<StateEffect, StateError> {
        let argument = fork_argument(text);
        if !matches!(argument, Some("" | "at")) {
            self.restore_draft(draft);
            self.chat.push_notice(
                "Use /fork for the latest boundary, or /fork at to choose an earlier point."
                    .to_owned(),
            )?;
            return Ok(StateEffect::Redraw);
        }
        if !self.allow_session_transition(draft)? {
            return Ok(StateEffect::Redraw);
        }
        self.clear_editor();
        if argument == Some("at") {
            self.fork_draft = Some(draft.to_owned());
            self.fork_picker_requested = true;
        } else {
            self.fork_session_requested = true;
        }
        Ok(StateEffect::Exit)
    }

    fn handle_resume_command(
        &mut self,
        text: &str,
        draft: &str,
    ) -> Result<StateEffect, StateError> {
        if !self.allow_session_transition(draft)? {
            return Ok(StateEffect::Redraw);
        }
        let argument = resume_argument(text).expect("known resume command");
        let target = if argument.is_empty() {
            None
        } else {
            match argument.parse::<yo_core::SessionId>() {
                Ok(id) if id.to_string() == argument => Some(id),
                _ => {
                    self.restore_draft(draft);
                    self.chat.push_notice(
                        "Use /resume to choose a session, or /resume followed by its full UUIDv7."
                            .to_owned(),
                    )?;
                    return Ok(StateEffect::Redraw);
                },
            }
        };
        self.clear_editor();
        self.resume_session_requested = Some(target);
        Ok(StateEffect::Exit)
    }

    fn execute_command(
        &mut self,
        effect: CommandEffect,
        invocation: &str,
        draft: &str,
    ) -> Result<StateEffect, StateError> {
        match effect {
            CommandEffect::NewSession => {
                if !self.allow_session_transition(draft)? {
                    return Ok(StateEffect::Redraw);
                }
                self.clear_editor();
                self.new_session_requested = true;
                Ok(StateEffect::Exit)
            },
            CommandEffect::ShowSessionTree => self.handle_tree_command(invocation, draft),
            CommandEffect::ForkSession => self.handle_fork_command(invocation, draft),
            CommandEffect::ResumeSession => self.handle_resume_command(invocation, draft),
            CommandEffect::InsertPrompt => self.handle_prompt_command(invocation, draft),
            CommandEffect::ShowHelp => {
                let document = TuiDocument::new(CommandRegistry::built_in().help_document())
                    .expect("built-in help is a bounded document")
                    .with_expanded(true);
                self.observe_document(document)?;
                self.clear_editor();
                Ok(StateEffect::Redraw)
            },
            CommandEffect::ReviewChanges => {
                self.clear_editor();
                if let Some(PendingRequest::Approval(request)) = self.pending_requests.front()
                    && self
                        .chat
                        .approval(request.activity())
                        .is_some_and(|profile| profile.related_change.is_some())
                {
                    if let Some(item) = self.chat.approval_change(request.activity()) {
                        self.views.open_changes_for(item);
                    } else {
                        self.chat.push_notice(
                            "The file changes linked to this approval are not available."
                                .to_owned(),
                        )?;
                    }
                } else {
                    self.views.open_changes();
                }
                self.sync_request_overlay()?;
                Ok(StateEffect::Redraw)
            },
            CommandEffect::ReviewOutput => {
                self.clear_editor();
                self.views.open_output();
                Ok(StateEffect::Redraw)
            },
            CommandEffect::OpenPreview => self.open_preview(),
            CommandEffect::AttachImage => self.prepare_image_command(draft),
            CommandEffect::SelectModel => self.handle_model_command(invocation, draft),
            CommandEffect::CompactContext => self.handle_compact_command(invocation, draft),
            CommandEffect::ExitProcess => {
                self.cancel_model_switches();
                self.clear_editor();
                Ok(StateEffect::Exit)
            },
        }
    }

    fn handle_prompt_command(
        &mut self,
        text: &str,
        draft: &str,
    ) -> Result<StateEffect, StateError> {
        let name = prompt_argument(text).expect("command syntax checked");
        if self.has_pending_request() {
            self.restore_draft(draft);
            self.chat.push_notice("Finish the pending approval or question before inserting a saved prompt. Your draft was preserved.".to_owned())?;
            return Ok(StateEffect::Redraw);
        }
        if name.is_empty() {
            let mut markdown = self
                .prompt_templates
                .names()
                .map(|name| format!("- `/prompt {name}`\n"))
                .collect::<String>();
            if markdown.is_empty() {
                markdown = "No saved prompts. Add a `prompts` mapping to your yo config.yaml, then restart yo.".to_owned();
            } else {
                markdown.push_str("\nChoose a name to insert its text into the editor. Review and press Enter to send.");
            }
            let document = TuiDocument::new(ActivityDocument {
                title: "Saved prompts".to_owned(),
                markdown,
            })
            .expect("validated template names form a bounded document")
            .with_expanded(true);
            self.observe_document(document)?;
            self.clear_editor();
            return Ok(StateEffect::Redraw);
        }
        let Some(body) = self.prompt_templates.get(name).map(str::to_owned) else {
            self.restore_draft(draft);
            self.chat.push_notice("Saved prompt not found. Use /prompt to list configured names; your draft was preserved.".to_owned())?;
            return Ok(StateEffect::Redraw);
        };
        self.clear_editor();
        self.editor.replace_range(0..0, &body);
        self.command_palette
            .preserve_literal(&body, &mut self.overlay);
        Ok(StateEffect::Redraw)
    }

    fn handle_compact_command(
        &mut self,
        text: &str,
        draft: &str,
    ) -> Result<StateEffect, StateError> {
        let guidance = compact_argument(text).expect("command syntax checked");
        if self.active_turn.is_some()
            || self.starting_submission.is_some()
            || self.context_compaction_pending
        {
            self.chat
                .push_notice("Context compaction requires an idle Session.".to_owned())?;
            self.restore_draft(draft);
            return Ok(StateEffect::Redraw);
        }
        self.clear_editor();
        Ok(StateEffect::Dispatch(AgentAction::CompactContext {
            guidance: (!guidance.is_empty()).then(|| guidance.to_owned()),
        }))
    }

    fn accept_model_selection(&mut self, identity: &str) -> Result<StateEffect, StateError> {
        let Some(controller) = self.model_selection.as_ref() else {
            self.chat
                .push_notice("The model selection controller is unavailable.".to_owned())?;
            return Ok(StateEffect::Redraw);
        };
        let accepted = controller.accept_identity(identity).map(|selected| {
            let is_current = controller.is_current(&selected);
            (selected, is_current)
        });
        match accepted {
            Ok((selected, true)) => self.accept_current_model(selected),
            Ok((selected, false)) => self.admit_model_selection(selected),
            Err(error) => {
                self.chat
                    .push_notice(format!("Model switch rejected: {error}"))?;
                Ok(StateEffect::Redraw)
            },
        }
    }

    fn accept_current_model(
        &mut self,
        selected: yo_core::ModelPickerTarget,
    ) -> Result<StateEffect, StateError> {
        self.clear_editor();
        if self.reserved_model_selection.take().is_some() {
            self.chat.push_notice(format!(
                "Reserved model switch canceled; model {} remains selected.",
                selected.model()
            ))?;
        } else {
            self.chat
                .push_notice(format!("Model {} is already selected.", selected.model()))?;
        }
        Ok(StateEffect::Redraw)
    }

    fn admit_model_selection(
        &mut self,
        selected: yo_core::ModelPickerTarget,
    ) -> Result<StateEffect, StateError> {
        if self.active_turn.is_some()
            || self.starting_submission.is_some()
            || !self.pending_submissions.is_empty()
            || self.has_pending_request()
        {
            let label = selected.coordinate_label();
            self.reserved_model_selection = Some(selected);
            self.chat
                .push_notice(format!("Model {label} will be applied to the next Turn."))?;
            Ok(StateEffect::Redraw)
        } else {
            self.pending_model_selection = Some(selected);
            Ok(StateEffect::Exit)
        }
    }

    fn push_unknown_command_notice(&mut self, text: String) -> Result<(), StateError> {
        self.chat.push_notice(format!(
            "Unknown command `{text}`. Press Esc while the command palette is visible to send it to the agent."
        )).map(|_| ())
    }

    fn clear_editor(&mut self) {
        self.command_palette.clear_literal();
        let length = self.editor.text().len();
        self.editor.replace_range(0..length, "");
        self.prompt_assist.prompt_cleared(&mut self.overlay);
    }

    fn restore_draft(&mut self, draft: &str) {
        let length = self.editor.text().len();
        self.editor.replace_range(0..length, draft);
    }

    fn cancel_model_switches(&mut self) {
        self.pending_model_selection = None;
        self.reserved_model_selection = None;
    }

    pub(super) const fn model_switch_ready(&self) -> bool {
        self.pending_model_selection.is_some()
    }

    pub(super) fn commit_frame(&mut self, frame: &PreparedFrame) {
        if let Some(preview) = self.preview.as_mut() {
            preview.state.commit_frame(frame);
            return;
        }
        self.views.commit(frame.view_state);
        if let Some(presentation) = frame.overlay_presentation
            && self
                .overlay
                .commit_presentation(presentation, frame.overlay_presented)
            && frame.overlay_presented
            && self.request_overlay.is_some_and(|(pending, token)| {
                Some(pending) == self.question_notes_refresh && self.overlay.is_current(token)
            })
        {
            self.question_notes_refresh = None;
        }
    }

    pub(super) fn acknowledge_publication(&mut self, frame: &PreparedFrame) -> bool {
        frame
            .publication
            .as_ref()
            .is_none_or(|publication| self.chat.acknowledge_publication(&publication.candidate))
    }

    fn close_request_overlay(&mut self) {
        self.request_panel = None;
        if let Some((_, token)) = self.request_overlay.take()
            && self.overlay.is_current(token)
        {
            self.overlay.close_current();
        }
    }

    fn sync_request_overlay(&mut self) -> Result<(), StateError> {
        let pending = self.pending_requests.front().copied();
        if self.question_notes_refresh != pending {
            self.question_notes_refresh = None;
        }
        if self
            .question_notes
            .is_some_and(|(request, _)| Some(request) != pending)
        {
            self.question_notes = None;
        }
        if self.restored_question_draft != pending {
            self.restored_question_draft = None;
            if let Some(request @ PendingRequest::UserInput(_)) = pending
                && let Some(question) = self.chat.question(request.activity())
                && let Some(draft) = &question.draft
            {
                // A host draft fills an untouched editor once; later snapshots cannot overwrite
                // typing.
                self.restored_question_draft = Some(request);
                if self.editor.text().is_empty() {
                    self.editor.replace_range(0..0, draft);
                    self.question_notes = question.draft_choice.map(|choice| (request, choice));
                }
            }
        }
        let panel = pending.map(|request| {
            let question = self.chat.question(request.activity());
            if let Some((_, choice)) = self.question_notes
                && let Some(selected) =
                    question.and_then(|question| question.choices.get(choice as usize - 1))
            {
                return PanelSnapshot::new(
                    "Notes",
                    vec![
                        SelectionEntry::status(
                            "notes",
                            if question.is_some_and(|question| question.previous_question) {
                                "Tab: choices. Shift+Tab: previous question. Enter sends both."
                            } else {
                                "Tab returns to choices. Enter sends both."
                            },
                        ),
                        SelectionEntry::enabled_with_context(
                            "send-notes",
                            selected.label.clone(),
                            None,
                            Some(selected.description.clone()),
                        ),
                    ],
                )
                .expect("selected question entry already validated")
                .for_request(false);
            }
            request.panel(question, self.chat.approval(request.activity()))
        });
        if self
            .saved_request_panel
            .as_ref()
            .is_some_and(|(request, snapshot, _)| {
                Some(*request) != pending || Some(snapshot) != panel.as_ref()
            })
        {
            self.saved_request_panel = None;
        }
        if self.request_overlay.is_some_and(|(request, token)| {
            Some(request) == pending
                && self.overlay.is_current(token)
                && self.request_panel == panel
        }) {
            return Ok(());
        }
        self.close_request_overlay();
        if let Some(pending) = pending
            && self.views.active() == ObservabilityView::Chat
        {
            let token = match self.saved_request_panel.take() {
                Some((_, _, saved)) => self.overlay.reopen(saved),
                None => self
                    .overlay
                    .open(panel.clone().expect("pending request has a panel")),
            }
            .map_err(StateError::RequestPanel)?;
            self.request_overlay = Some((pending, token));
            self.request_panel = panel;
        }
        Ok(())
    }

    fn reject_referenced_answer(&mut self) -> Result<bool, StateError> {
        if !self.prompt_assist.has_accepted_references() {
            return Ok(false);
        }
        self.chat.push_notice(
            "Selected references cannot answer an agent question. Remove them from the answer draft first."
                .to_owned(),
        )?;
        Ok(true)
    }

    fn request_response(
        &mut self,
        pending: PendingRequest,
        text: String,
    ) -> Result<StateEffect, StateError> {
        match pending {
            PendingRequest::Approval(request) => self.approval_response(request, text),
            PendingRequest::UserInput(request) => {
                let choice = self
                    .question_notes
                    .take()
                    .filter(|(owner, _)| *owner == pending)
                    .map(|(_, choice)| choice);
                self.pending_requests.pop_front();
                self.close_request_overlay();
                self.sync_request_overlay()?;
                Ok(StateEffect::Dispatch(match choice {
                    Some(choice) => AgentAction::RespondToQuestion {
                        request,
                        choice,
                        notes: text,
                    },
                    None => AgentAction::RespondToUserInput {
                        request,
                        input: text,
                    },
                }))
            },
        }
    }

    fn approval_response(
        &mut self,
        request: ActivityRequestRef,
        text: String,
    ) -> Result<StateEffect, StateError> {
        let decision = if let Some(approval) = self.chat.approval(request.activity()) {
            let input = text.trim().to_ascii_lowercase();
            let choice = if matches!(input.as_str(), "n" | "no") {
                if !approval.decline_choice.is_some_and(|choice| {
                    self.request_panel
                        .as_ref()
                        .is_some_and(|panel| panel.offers(&choice.to_string()))
                }) {
                    return Ok(StateEffect::Dispatch(AgentAction::Interrupt));
                }
                approval.decline_choice
            } else {
                input.parse::<u32>().ok()
            };
            let Some(choice) = choice.filter(|choice| {
                if !self
                    .request_panel
                    .as_ref()
                    .is_some_and(|panel| panel.offers(&choice.to_string()))
                {
                    return false;
                }

                choice
                    .checked_sub(1)
                    .and_then(|index| approval.choices.get(index as usize))
                    .is_some_and(|choice| choice.enabled)
            }) else {
                self.restore_draft(&text);
                self.chat.push_notice(
                    "Select an offered approval option, or press Esc to cancel this request."
                        .to_owned(),
                )?;
                return Ok(StateEffect::Redraw);
            };
            ApprovalDecision::Offered(choice)
        } else {
            match text.trim().to_ascii_lowercase().as_str() {
                "y" | "yes" => ApprovalDecision::Approved,
                "n" | "no" => ApprovalDecision::Declined,
                _ => {
                    self.restore_draft(&text);
                    self.chat.push_notice(
                        "Approval is waiting: enter `y` to approve or `n` to decline.".to_owned(),
                    )?;
                    return Ok(StateEffect::Redraw);
                },
            }
        };
        self.pending_requests.pop_front();
        self.close_request_overlay();
        self.sync_request_overlay()?;
        Ok(StateEffect::Dispatch(AgentAction::RespondToApproval {
            request,
            decision,
        }))
    }
}

impl PendingRequest {
    fn panel(
        self,
        question: Option<&ActivityQuestion>,
        approval: Option<&ActivityApproval>,
    ) -> PanelSnapshot {
        if matches!(self, Self::Approval(_))
            && let Some(approval) = approval
        {
            let mut entries = vec![SelectionEntry::status("context", "F2: request history")];
            if approval.related_change.is_some() {
                entries.push(SelectionEntry::status(
                    "changes",
                    "/changes: proposed files",
                ));
            }
            if approval.decline_choice.is_none() {
                entries.push(SelectionEntry::enabled_with_context(
                    "stop-turn",
                    "Stop turn",
                    None,
                    Some("Interrupt without granting permission".to_owned()),
                ));
            }
            let mut choices: Vec<_> = approval.choices.iter().enumerate().collect();
            choices.sort_by_key(|(index, _)| Some((*index + 1) as u32) != approval.decline_choice);
            for (index, choice) in choices {
                if choice.enabled {
                    entries.push(SelectionEntry::enabled_with_context(
                        (index + 1).to_string(),
                        choice.label.clone(),
                        None,
                        Some(choice.description.clone()),
                    ));
                } else {
                    entries.push(SelectionEntry::status(
                        (index + 1).to_string(),
                        choice.label.clone(),
                    ));
                }
            }
            return PanelSnapshot::new("Approval", entries)
                .unwrap_or_else(|_| {
                    PanelSnapshot::new(
                        "Approval",
                        vec![
                            SelectionEntry::status("context", "F2: request history"),
                            SelectionEntry::enabled_with_context(
                                "stop-turn",
                                "Stop turn",
                                None,
                                Some("Choice text cannot be displayed safely".to_owned()),
                            ),
                        ],
                    )
                    .expect("static approval fallback")
                })
                .for_request(approval.decline_choice.is_some());
        }
        let (title, entries) = match self {
            Self::Approval(_) => (
                "Approval required",
                vec![
                    SelectionEntry::status("context", "F2: request history"),
                    SelectionEntry::enabled_with_context(
                        "decline",
                        "Decline",
                        None,
                        Some("Do not run this action".to_owned()),
                    ),
                    SelectionEntry::enabled_with_context(
                        "approve-request",
                        "Approve request",
                        None,
                        Some("Use the scope described above".to_owned()),
                    ),
                ],
            ),
            Self::UserInput(_) => (
                "Your answer",
                if let Some(question) = question.filter(|question| !question.choices.is_empty()) {
                    let mut entries = vec![SelectionEntry::status(
                        "answer",
                        if question.allow_notes {
                            "Enter selects. Tab adds notes. Or type an answer."
                        } else {
                            "Choose or type your own answer."
                        },
                    )];
                    if question.previous_question {
                        entries.push(SelectionEntry::status(
                            "previous-question",
                            "Shift+Tab: previous question",
                        ));
                    }
                    entries.extend(question.choices.iter().enumerate().map(|(index, choice)| {
                        SelectionEntry::enabled_with_context(
                            (index + 1).to_string(),
                            choice.label.clone(),
                            None,
                            Some(choice.description.clone()),
                        )
                    }));
                    entries
                } else {
                    vec![SelectionEntry::status(
                        "answer",
                        if question.is_some_and(|question| question.previous_question) {
                            "Type your answer. Shift+Tab: previous question."
                        } else {
                            "Type a number or your own answer."
                        },
                    )]
                },
            ),
        };
        match PanelSnapshot::new(title, entries) {
            Ok(panel) => panel.for_request(matches!(self, Self::Approval(_))),
            Err(_) if question.is_some() => self.panel(None, None),
            Err(_) => unreachable!("static request panel is valid"),
        }
    }

    const fn activity(self) -> ActivityRef {
        match self {
            Self::Approval(request) | Self::UserInput(request) => request.activity(),
        }
    }
}

fn record_effect(record: &TranscriptRecord) -> StateEffect {
    match record {
        TranscriptRecord::CommandCommitted(
            yo_core::AgentCommand::CreateSession { .. }
            | yo_core::AgentCommand::RespondToActivity { .. }
            | yo_core::AgentCommand::InterruptTurn { .. },
        )
        | TranscriptRecord::EventCommitted(AgentEvent::SessionCreated { .. }) => {
            StateEffect::Unchanged
        },
        TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated {
            update: yo_core::ActivityUpdate::TextDelta(text),
            ..
        }) if text.is_empty() => StateEffect::Unchanged,
        _ => StateEffect::Redraw,
    }
}

#[cfg(test)]
impl TuiState {
    pub(super) fn transcript(&self) -> &crate::transcript::TranscriptState {
        self.chat.transcript()
    }

    pub(super) fn editor(&self) -> &PromptEditor {
        &self.editor
    }

    pub(super) fn set_next_item_id(&mut self, value: u64) {
        self.chat.set_next_item_id(value);
    }

    pub(super) const fn turn_active(&self) -> bool {
        self.active_turn.is_some()
    }

    pub(super) fn views(&self) -> &ObservabilityViews {
        &self.views
    }
}

impl TuiState {
    pub(in crate::runner) fn visible_motion_turn(&self) -> Option<(bool, TurnRef)> {
        if let Some(preview) = &self.preview {
            return preview.state.active_turn.map(|turn| (true, turn));
        }
        self.active_turn.map(|turn| (false, turn))
    }
}
