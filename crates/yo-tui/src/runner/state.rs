use std::{collections::VecDeque, time::Duration};

use yo_core::{
    ActivityKind, ActivityRef, ActivityRequestRef, AgentControlOutcome, AgentEvent,
    ApprovalDecision, InputSubmission, JournalDurability, RequestTraceEntry,
    SkillReferenceSearchRequest, SkillReferenceSearchUpdate, SubmissionId, SubmissionOutcome,
    TranscriptRecord, TurnRef, UserInput, WorkspaceReferenceSearchRequest,
    WorkspaceReferenceSearchUpdate,
};

use crate::{
    appearance::AppearancePin,
    command::{CommandEffect, CommandPalette, CommandRegistry, compact_argument, model_argument},
    input::{
        editor::{EditorEffect, PromptEditor},
        event::InputEvent,
    },
    overlay::{
        AcceptanceReceipt, OverlayInputEffect, OverlayInstanceToken, PanelSnapshot,
        PromptOverlaySlot, SlotError,
    },
    prompt::{
        assist::{PromptAssistController, PromptAssistRequest},
        workspace_reference::WorkspaceEdit,
    },
    runner::{
        AgentAction, PresentationMode,
        chat::{ChatProjection, ChatProjectionChange},
        model::ModelSelectionState,
        session::TuiSessionInfo,
        view::{ObservabilityViews, ViewInputEffect},
    },
    surface::Size,
    transcript::{TranscriptMeasureError, TranscriptStateError},
};

mod presentation;

pub(super) use presentation::{FrameError, MotionDemand, PreparedFrame};

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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StateError {
    Transcript(TranscriptStateError),
    UnknownActivity(ActivityRef),
    ItemIdOverflow,
    SubmissionIdentityUnavailable,
    StalePublication,
}

#[derive(Debug, Default)]
pub(super) struct TuiState {
    chat: ChatProjection,
    editor: PromptEditor,
    views: ObservabilityViews,
    pending_requests: VecDeque<PendingRequest>,
    active_turn: Option<TurnRef>,
    durability: Option<JournalDurability>,
    session_info: TuiSessionInfo,
    presentation_mode: PresentationMode,
    overlay: PromptOverlaySlot,
    command_palette: CommandPalette,
    accepted_overlays: VecDeque<AcceptanceReceipt>,
    prompt_assist: PromptAssistController,
    pending_submissions: VecDeque<InputSubmission>,
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
    #[cfg(test)]
    pub(super) fn new() -> Self {
        Self::with_session_info(TuiSessionInfo::default())
    }

    pub(super) fn with_session_info(session_info: TuiSessionInfo) -> Self {
        Self {
            chat: ChatProjection::new(),
            session_info,
            ..Self::default()
        }
    }

    pub(super) fn set_presentation_mode(&mut self, mode: PresentationMode) {
        self.presentation_mode = mode;
    }

    pub(super) fn handle(
        &mut self,
        input: InputEvent,
        now: Duration,
    ) -> Result<StateEffect, StateError> {
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
                    self.overlay.close_current();
                    self.command_palette.dismiss();
                    self.model_overlay = None;
                    self.prompt_assist.cancel();
                }
                return Ok(StateEffect::Redraw);
            },
        }

        match self.overlay.handle(&input) {
            OverlayInputEffect::Unhandled => {},
            OverlayInputEffect::Consumed => return Ok(StateEffect::Unchanged),
            OverlayInputEffect::Redraw => return Ok(StateEffect::Redraw),
            OverlayInputEffect::Dismissed(token) => {
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

        match self.views.handle_local(&input) {
            ViewInputEffect::Unhandled => {},
            ViewInputEffect::Consumed => return Ok(StateEffect::Unchanged),
            ViewInputEffect::Redraw => return Ok(StateEffect::Redraw),
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
                let assist_eligible = self.views.active()
                    == crate::runner::view::ObservabilityView::Chat
                    && !self.has_pending_request();
                let command_eligible =
                    self.views.active() == crate::runner::view::ObservabilityView::Chat;
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
                if !escaped_palette {
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
                if !escaped_palette && self.prompt_assist.has_accepted_references() {
                    self.editor.replace_range(0..0, &text);
                    self.chat.push_notice(
                        "Structured reference selected. Exact admission is not connected yet; the draft was preserved."
                            .to_owned(),
                    )?;
                    return Ok(StateEffect::Redraw);
                }
                if let Some(request) = self.pending_requests.front().copied() {
                    return self.request_response(request, text);
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
                let submission = InputSubmission::new(id, UserInput::new(text.clone()));
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
        match outcome {
            SubmissionOutcome::Accepted { .. } => {
                if self.editor.text() == submission.input().as_str() {
                    let length = self.editor.text().len();
                    self.editor.replace_range(0..length, "");
                }
                Ok(StateEffect::Redraw)
            },
            SubmissionOutcome::Rejected { rejection, .. } => {
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

    pub(super) fn observe_control_outcome(
        &mut self,
        outcome: AgentControlOutcome,
    ) -> Result<StateEffect, StateError> {
        match outcome {
            AgentControlOutcome::ContextCompactionRejected { detail } => {
                self.chat
                    .push_notice(format!("Context compaction was not started.\n{detail}"))?;
            },
        }
        Ok(StateEffect::Redraw)
    }

    pub(super) fn observe_record(
        &mut self,
        record: TranscriptRecord,
    ) -> Result<StateEffect, StateError> {
        let lifecycle_effect = self.observe_live_lifecycle(&record)?;
        let chat_change = self.chat.observe_record(&record)?;
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
        self.durability = Some(durability);
        Ok(StateEffect::Unchanged)
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
        let TranscriptRecord::EventCommitted(event) = record else {
            return Ok(StateEffect::Unchanged);
        };
        match event {
            AgentEvent::TurnStarted { turn } => self.active_turn = Some(*turn),
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
                    self.overlay.close_current();
                    self.command_palette.dismiss();
                    self.model_overlay = None;
                    self.prompt_assist.cancel();
                    self.pending_requests.push_back(request);
                }
            },
            AgentEvent::ActivityFinished { activity, .. } => {
                self.pending_requests
                    .retain(|request| request.activity() != *activity);
            },
            AgentEvent::TurnFinished { turn, .. } if self.active_turn == Some(*turn) => {
                self.active_turn = None;
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
        if self.views.active() != crate::runner::view::ObservabilityView::Chat {
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

    fn execute_command(
        &mut self,
        effect: CommandEffect,
        invocation: &str,
        draft: &str,
    ) -> Result<StateEffect, StateError> {
        match effect {
            CommandEffect::ShowHelp => {
                self.chat
                    .push_notice(CommandRegistry::built_in().help_notice())?;
                self.clear_editor();
                Ok(StateEffect::Redraw)
            },
            CommandEffect::SelectModel => self.handle_model_command(invocation, draft),
            CommandEffect::CompactContext => self.handle_compact_command(invocation, draft),
            CommandEffect::ExitProcess => {
                self.cancel_model_switches();
                self.clear_editor();
                Ok(StateEffect::Exit)
            },
        }
    }

    fn handle_compact_command(
        &mut self,
        text: &str,
        draft: &str,
    ) -> Result<StateEffect, StateError> {
        let guidance = compact_argument(text).expect("command syntax checked");
        if self.active_turn.is_some() {
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
        let length = self.editor.text().len();
        self.editor.replace_range(0..length, "");
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
        self.views.commit(frame.view_state);
        if let Some(presentation) = frame.overlay_presentation {
            self.overlay
                .commit_presentation(presentation, frame.overlay_presented);
        }
    }

    pub(super) fn acknowledge_publication(&mut self, frame: &PreparedFrame) -> bool {
        frame
            .publication
            .as_ref()
            .is_none_or(|publication| self.chat.acknowledge_publication(&publication.candidate))
    }

    fn request_response(
        &mut self,
        pending: PendingRequest,
        text: String,
    ) -> Result<StateEffect, StateError> {
        match pending {
            PendingRequest::Approval(request) => self.approval_response(request, text),
            PendingRequest::UserInput(request) => {
                self.pending_requests.pop_front();
                Ok(StateEffect::Dispatch(AgentAction::RespondToUserInput {
                    request,
                    input: text,
                }))
            },
        }
    }

    fn approval_response(
        &mut self,
        request: ActivityRequestRef,
        text: String,
    ) -> Result<StateEffect, StateError> {
        let decision = match text.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => ApprovalDecision::Approved,
            "n" | "no" => ApprovalDecision::Declined,
            _ => {
                self.chat.push_notice(
                    "Approval is waiting: enter `y` to approve or `n` to decline.".to_owned(),
                )?;
                return Ok(StateEffect::Redraw);
            },
        };
        self.pending_requests.pop_front();
        Ok(StateEffect::Dispatch(AgentAction::RespondToApproval {
            request,
            decision,
        }))
    }
}

impl PendingRequest {
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
