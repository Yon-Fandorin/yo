use std::{collections::VecDeque, time::Duration};

use yo_core::{
    ActivityKind, ActivityRef, ActivityRequestRef, AgentEvent, ApprovalDecision, JournalDurability,
    TranscriptRecord,
};

use crate::{
    appearance::{AppearancePin, AppearanceRevision},
    input::{
        editor::{EditorEffect, PromptEditor},
        event::InputEvent,
    },
    runner::{
        AgentAction, PresentationMode,
        chat::{ChatProjection, ChatProjectionChange},
        session::TuiSessionInfo,
        view::{
            ObservabilityRenderError, ObservabilityRenderOptions, ObservabilityViewState,
            ObservabilityViews, ViewInputEffect,
        },
    },
    shell::ShellChromeSnapshot,
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StateError {
    Transcript(TranscriptStateError),
    UnknownActivity(ActivityRef),
    ItemIdOverflow,
}

#[derive(Debug)]
pub(super) enum FrameError {
    Allocate(SurfaceError),
    Render(ObservabilityRenderError),
}

impl FrameError {
    pub(super) fn detail(&self) -> String {
        match self {
            Self::Allocate(error) => format!("allocating the frame failed: {error}"),
            Self::Render(error) => format!("composing the agent shell failed: {error:?}"),
        }
    }
}

pub(super) struct PreparedFrame {
    pub(super) surface: Surface,
    pub(super) cursor: Point,
    pub(super) appearance_revision: AppearanceRevision,
    pub(super) motion_demand: Option<MotionDemand>,
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
        match self.views.handle(&input) {
            ViewInputEffect::Unhandled => {},
            ViewInputEffect::Consumed => return Ok(StateEffect::Unchanged),
            ViewInputEffect::Redraw => return Ok(StateEffect::Redraw),
        }

        let effect = self.editor.handle(input, self.turn_active, now);
        match effect {
            EditorEffect::BufferChanged => Ok(StateEffect::Redraw),
            EditorEffect::Submitted(text) => {
                if let Some(request) = self.pending_requests.front().copied() {
                    return self.request_response(request, text);
                }
                Ok(StateEffect::Dispatch(AgentAction::Submit(text)))
            },
            EditorEffect::Exit => Ok(StateEffect::Exit),
            EditorEffect::InterruptTask => Ok(StateEffect::Dispatch(AgentAction::Interrupt)),
            EditorEffect::Unhandled | EditorEffect::NoChange | EditorEffect::ExitArmed => {
                Ok(StateEffect::Unchanged)
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

    pub(super) fn observe_durability(
        &mut self,
        durability: JournalDurability,
    ) -> Result<StateEffect, StateError> {
        self.durability = Some(durability);
        Ok(StateEffect::Unchanged)
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

    pub(super) fn prepare_frame_at(
        &self,
        size: Size,
        appearance: &AppearancePin,
        elapsed: Duration,
    ) -> Result<PreparedFrame, FrameError> {
        self.prepare_frame_at_with_measure_hook(size, appearance, elapsed, || {})
    }

    #[cfg(test)]
    pub(super) fn prepare_frame_with_measure_hook(
        &self,
        size: Size,
        appearance: &AppearancePin,
        after_measure: impl FnOnce(),
    ) -> Result<PreparedFrame, FrameError> {
        self.prepare_frame_at_with_measure_hook(size, appearance, Duration::ZERO, after_measure)
    }

    fn prepare_frame_at_with_measure_hook(
        &self,
        size: Size,
        appearance: &AppearancePin,
        elapsed: Duration,
        after_measure: impl FnOnce(),
    ) -> Result<PreparedFrame, FrameError> {
        let mut surface = Surface::new(size).map_err(FrameError::Allocate)?;
        let snapshot = appearance.snapshot();
        let frame = {
            let area = Rect::new(Point::new(0, 0), size);
            let mut view = surface
                .view(area)
                .expect("the complete surface is always a valid view");
            self.views
                .render(
                    self.chat.transcript(),
                    &self.editor,
                    &mut view,
                    ObservabilityRenderOptions {
                        appearance: snapshot,
                        chrome: self.chrome_snapshot(),
                        elapsed,
                    },
                    after_measure,
                )
                .map_err(FrameError::Render)?
        };

        Ok(PreparedFrame {
            surface,
            cursor: frame.cursor,
            appearance_revision: appearance.revision(),
            motion_demand: frame
                .activity_motion_period
                .map(|period| MotionDemand { period }),
            view_state: frame.state,
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
        self.chat
            .transcript()
            .plain_output(appearance.snapshot().transcript_config())
    }

    pub(super) fn has_pending_request(&self) -> bool {
        !self.pending_requests.is_empty()
    }

    pub(super) fn commit_frame(&mut self, frame: &PreparedFrame) {
        self.views.commit(frame.view_state);
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
