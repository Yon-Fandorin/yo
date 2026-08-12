use std::{collections::VecDeque, time::Duration};

use yo_core::{
    ActivityKind, ActivityRef, ActivityRequestRef, AgentEvent, ApprovalDecision, InputSubmission,
    JournalDurability, RequestTraceEntry, SkillReferenceSearchRequest, SkillReferenceSearchUpdate,
    SubmissionId, SubmissionOutcome, TranscriptRecord, UserInput, WorkspaceReferenceSearchRequest,
    WorkspaceReferenceSearchUpdate,
};

use crate::{
    appearance::{AppearancePin, AppearanceRevision},
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
        publication::{self, PreparedPublication, PublicationPrepareError},
        session::TuiSessionInfo,
        view::{
            ObservabilityRenderError, ObservabilityRenderOptions, ObservabilityViewState,
            ObservabilityViews, ViewInputEffect,
        },
    },
    shell::{AgentShellMeasureError, AgentShellRenderOptions, ShellChromeSnapshot},
    surface::{Point, Rect, Size, Surface, SurfaceError},
    transcript::{TranscriptMeasureError, TranscriptStateError},
};

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

#[derive(Debug)]
pub(super) enum FrameError {
    Allocate(SurfaceError),
    Measure(AgentShellMeasureError),
    Publication(PublicationPrepareError),
    Render(ObservabilityRenderError),
}

impl FrameError {
    pub(super) fn detail(&self) -> String {
        match self {
            Self::Allocate(error) => format!("allocating the frame failed: {error}"),
            Self::Measure(error) => format!("measuring the compact agent shell failed: {error:?}"),
            Self::Publication(error) => error.detail(),
            Self::Render(error) => format!("composing the agent shell failed: {error:?}"),
        }
    }
}

pub(super) struct PreparedFrame {
    pub(super) surface: Surface,
    pub(super) publication: Option<PreparedPublication>,
    pub(super) cursor: Point,
    pub(super) appearance_revision: AppearanceRevision,
    pub(super) motion_demand: Option<MotionDemand>,
    pub(super) overlay_presented: bool,
    pub(super) reprepare_for_publication: bool,
    view_state: ObservabilityViewState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MotionDemand {
    period: Duration,
}

#[derive(Debug, Default)]
pub(super) struct TuiState {
    chat: ChatProjection,
    editor: PromptEditor,
    views: ObservabilityViews,
    pending_requests: VecDeque<PendingRequest>,
    turn_active: bool,
    durability: Option<JournalDurability>,
    session_info: TuiSessionInfo,
    presentation_mode: PresentationMode,
    overlay: PromptOverlaySlot,
    accepted_overlays: VecDeque<AcceptanceReceipt>,
    prompt_assist: PromptAssistController,
    pending_submissions: VecDeque<InputSubmission>,
    model_selection: Option<ModelSelectionState>,
    model_overlay: Option<OverlayInstanceToken>,
    pending_model_selection: Option<yo_core::ModelSelection>,
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
                    self.prompt_assist.cancel();
                }
                return Ok(StateEffect::Redraw);
            },
        }

        match self.overlay.handle(&input) {
            OverlayInputEffect::Unhandled => {},
            OverlayInputEffect::Consumed => return Ok(StateEffect::Unchanged),
            OverlayInputEffect::Redraw => {
                if !self.overlay.is_open() {
                    self.prompt_assist.cancel();
                }
                return Ok(StateEffect::Redraw);
            },
            OverlayInputEffect::FilterChanged(selected) => {
                self.prompt_assist
                    .filter_changed(selected, &mut self.overlay);
                return Ok(StateEffect::Redraw);
            },
            OverlayInputEffect::Accepted(receipt) => {
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
        let effect = self.editor.handle(input, self.turn_active, now);
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
                Ok(self
                    .prompt_assist
                    .prompt_changed(
                        &self.editor,
                        &mut self.overlay,
                        edit.as_ref(),
                        assist_eligible,
                    )
                    .map_or(StateEffect::Redraw, |request| match request {
                        PromptAssistRequest::Workspace(request) => {
                            StateEffect::WorkspaceSearch(request)
                        },
                        PromptAssistRequest::Skill(request) => StateEffect::SkillSearch(request),
                    }))
            },
            EditorEffect::Submitted(text) => {
                if self.prompt_assist.has_accepted_references() {
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
                if text == "/model" || text.starts_with("/model ") {
                    return self.handle_model_command(&text);
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
                Ok(StateEffect::Dispatch(AgentAction::Submit(submission)))
            },
            EditorEffect::Exit => Ok(StateEffect::Exit),
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
                Ok(StateEffect::Redraw)
            },
        }
    }

    pub(super) fn observe_record(
        &mut self,
        record: TranscriptRecord,
    ) -> Result<StateEffect, StateError> {
        self.observe_live_lifecycle(&record);
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
        Ok(effect)
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

    fn observe_live_lifecycle(&mut self, record: &TranscriptRecord) {
        let TranscriptRecord::EventCommitted(event) = record else {
            return;
        };
        match event {
            AgentEvent::TurnStarted { .. } => self.turn_active = true,
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
                    self.prompt_assist.cancel();
                    self.pending_requests.push_back(request);
                }
            },
            AgentEvent::ActivityFinished { activity, .. } => {
                self.pending_requests
                    .retain(|request| request.activity() != *activity);
            },
            AgentEvent::TurnFinished { .. } => self.turn_active = false,
            AgentEvent::SessionCreated { .. } | AgentEvent::ActivityUpdated { .. } => {},
        }
    }

    #[cfg(test)]
    pub(super) fn prepare_frame(
        &self,
        size: Size,
        appearance: &AppearancePin,
    ) -> Result<PreparedFrame, FrameError> {
        self.prepare_frame_at(size, appearance, Duration::ZERO)
    }

    #[cfg(test)]
    pub(super) fn prepare_frame_at(
        &self,
        size: Size,
        appearance: &AppearancePin,
        elapsed: Duration,
    ) -> Result<PreparedFrame, FrameError> {
        self.prepare_frame_at_with_measure_hook(size, appearance, elapsed, 0, false, || {})
    }

    pub(super) fn prepare_frame_for_geometry(
        &self,
        size: Size,
        appearance: &AppearancePin,
        elapsed: Duration,
        geometry_epoch: u64,
    ) -> Result<PreparedFrame, FrameError> {
        self.prepare_frame_at_with_measure_hook(
            size,
            appearance,
            elapsed,
            geometry_epoch,
            true,
            || {},
        )
    }

    #[cfg(test)]
    pub(super) fn prepare_frame_with_measure_hook(
        &self,
        size: Size,
        appearance: &AppearancePin,
        after_measure: impl FnOnce(),
    ) -> Result<PreparedFrame, FrameError> {
        self.prepare_frame_at_with_measure_hook(
            size,
            appearance,
            Duration::ZERO,
            0,
            false,
            after_measure,
        )
    }

    fn prepare_frame_at_with_measure_hook(
        &self,
        size: Size,
        appearance: &AppearancePin,
        elapsed: Duration,
        geometry_epoch: u64,
        publication_enabled: bool,
        after_measure: impl FnOnce(),
    ) -> Result<PreparedFrame, FrameError> {
        let snapshot = appearance.snapshot();
        let publication_eligible = publication_enabled
            && self.presentation_mode == PresentationMode::Inline
            && self.views.inline_publication_eligible();
        let candidate = publication_eligible
            .then(|| self.chat.publication_candidate())
            .flatten();
        let live_start = if publication_eligible {
            candidate.as_ref().map_or_else(
                || self.chat.published_item_count(),
                |candidate| candidate.range().end,
            )
        } else {
            0
        };
        let live_transcript = self.chat.transcript().suffix(live_start);
        let render_options = ObservabilityRenderOptions {
            appearance: snapshot,
            chrome: self.chrome_snapshot(),
            elapsed,
            overlay: self.overlay.panel(),
            overlay_bindings: self.overlay.bindings(),
        };
        let frame_size = if publication_eligible {
            let shell_options = AgentShellRenderOptions {
                transcript_config: snapshot.transcript_config(),
                styles: snapshot.styles(),
                scroll: None,
                frame_prompt: true,
                chrome: render_options.chrome,
                activity_motion: snapshot.activity_motion_frame(elapsed),
                overlay: render_options.overlay,
                overlay_bindings: render_options.overlay_bindings,
            };
            publication::compact_live_size(live_transcript, &self.editor, size, shell_options)
                .map_err(FrameError::Measure)?
        } else {
            size
        };
        let publication = candidate
            .map(|candidate| {
                publication::prepare(
                    self.chat.transcript().slice(candidate.range()),
                    candidate,
                    size,
                    geometry_epoch,
                    appearance.revision(),
                    snapshot,
                )
            })
            .transpose()
            .map_err(FrameError::Publication)?;
        let mut surface = Surface::new(frame_size).map_err(FrameError::Allocate)?;
        let frame = {
            let area = Rect::new(Point::new(0, 0), frame_size);
            let mut view = surface
                .view(area)
                .expect("the complete surface is always a valid view");
            self.views
                .render(
                    live_transcript,
                    &self.editor,
                    &mut view,
                    render_options,
                    after_measure,
                )
                .map_err(FrameError::Render)?
        };

        Ok(PreparedFrame {
            surface,
            publication,
            cursor: frame.cursor,
            appearance_revision: appearance.revision(),
            motion_demand: frame.motion_period.map(|period| MotionDemand { period }),
            view_state: frame.state,
            overlay_presented: frame.overlay_presented,
            reprepare_for_publication: publication_enabled
                && self.presentation_mode == PresentationMode::Inline
                && !publication_eligible
                && frame.state.inline_publication_eligible(),
        })
    }

    fn chrome_snapshot(&self) -> ShellChromeSnapshot<'_> {
        ShellChromeSnapshot {
            turn_active: self.turn_active,
            backend: self.session_info.backend(),
            workspace: self.session_info.workspace(),
            mode: self.presentation_mode,
        }
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

    pub(super) fn take_model_selection(&mut self) -> Option<yo_core::ModelSelection> {
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

    fn handle_model_command(&mut self, text: &str) -> Result<StateEffect, StateError> {
        if self.turn_active || !self.pending_submissions.is_empty() || self.has_pending_request() {
            self.chat.push_notice(
                "Model switching is available only while the Session is idle.".to_owned(),
            )?;
            return Ok(StateEffect::Redraw);
        }
        let Some(selection) = self.model_selection.as_ref() else {
            self.chat.push_notice(
                "No configured model catalog is available for this Session.".to_owned(),
            )?;
            return Ok(StateEffect::Redraw);
        };
        let argument = text
            .strip_prefix("/model")
            .expect("command prefix checked")
            .trim();
        if argument.is_empty() {
            match selection.panel() {
                Ok(panel) => match self.overlay.open(panel) {
                    Ok(token) => {
                        self.model_overlay = Some(token);
                    },
                    Err(error) => {
                        self.chat.push_notice(format!(
                            "The model picker could not be opened: {error:?}"
                        ))?;
                    },
                },
                Err(error) => {
                    self.chat.push_notice(error)?;
                },
            }
            return Ok(StateEffect::Redraw);
        }
        match selection.resolve_direct(argument) {
            Ok(selected) if selection.is_current(&selected) => {
                self.chat
                    .push_notice(format!("Model {} is already selected.", selected.model()))?;
                Ok(StateEffect::Redraw)
            },
            Ok(selected) => {
                self.pending_model_selection = Some(selected);
                Ok(StateEffect::Exit)
            },
            Err(error) => {
                self.chat
                    .push_notice(format!("Model switch rejected: {error}"))?;
                Ok(StateEffect::Redraw)
            },
        }
    }

    fn accept_model_selection(&mut self, identity: &str) -> Result<StateEffect, StateError> {
        if self.turn_active || !self.pending_submissions.is_empty() || self.has_pending_request() {
            self.chat.push_notice(
                "Model switching is available only while the Session is idle.".to_owned(),
            )?;
            return Ok(StateEffect::Redraw);
        }
        let Some(controller) = self.model_selection.as_ref() else {
            self.chat
                .push_notice("The model selection controller is unavailable.".to_owned())?;
            return Ok(StateEffect::Redraw);
        };
        match controller.accept_identity(identity) {
            Ok(selected) if controller.is_current(&selected) => {
                self.chat
                    .push_notice(format!("Model {} is already selected.", selected.model()))?;
                Ok(StateEffect::Redraw)
            },
            Ok(selected) => {
                self.pending_model_selection = Some(selected);
                Ok(StateEffect::Exit)
            },
            Err(error) => {
                self.chat
                    .push_notice(format!("Model switch rejected: {error}"))?;
                Ok(StateEffect::Redraw)
            },
        }
    }

    pub(super) fn commit_frame(&mut self, frame: &PreparedFrame) {
        self.views.commit(frame.view_state);
        self.overlay.set_presented(frame.overlay_presented);
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

impl MotionDemand {
    pub(super) const fn period(self) -> Duration {
        self.period
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
        self.turn_active
    }

    pub(super) fn views(&self) -> &ObservabilityViews {
        &self.views
    }
}
