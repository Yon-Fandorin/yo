use std::{
    collections::{HashMap, VecDeque},
    time::Duration,
};

use yo_core::{
    ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef, ActivityUpdate, AgentCommand,
    AgentEvent, ApprovalDecision, JournalDurability, TranscriptRecord, TurnOutcome,
};

use crate::{
    appearance::{AppearancePin, AppearanceRevision},
    input::{
        editor::{EditorEffect, PromptEditor},
        event::InputEvent,
    },
    runner::{
        AgentAction,
        view::{
            ObservabilityRenderError, ObservabilityViewState, ObservabilityViews, ViewInputEffect,
        },
    },
    surface::{Point, Rect, Size, Surface, SurfaceError},
    transcript::{TranscriptItemId, TranscriptMeasureError, TranscriptState, TranscriptStateError},
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
    view_state: ObservabilityViewState,
}

#[derive(Debug, Default)]
pub(super) struct TuiState {
    transcript: TranscriptState,
    editor: PromptEditor,
    views: ObservabilityViews,
    next_item_id: u64,
    activities: HashMap<ActivityRef, ActivityPresentation>,
    pending_requests: VecDeque<PendingRequest>,
    turn_active: bool,
    durability: Option<JournalDurability>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActivityPresentation {
    item: TranscriptItemId,
    kind: ActivityKind,
    has_payload: bool,
    visible: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingRequest {
    Approval(ActivityRequestRef),
    UserInput(ActivityRequestRef),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChatProjectionChange {
    Unchanged,
    VisibleItem(TranscriptItemId),
}

impl TuiState {
    pub(super) fn new() -> Self {
        Self {
            next_item_id: 1,
            ..Self::default()
        }
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
        let projection_record = record.clone();
        let (effect, chat_change) = match record {
            TranscriptRecord::CommandCommitted(
                AgentCommand::StartTurn { input, .. } | AgentCommand::SteerTurn { input, .. },
            ) => {
                let id = self.next_transcript_id()?;
                self.transcript
                    .push_user(id, input.into_string())
                    .map_err(StateError::Transcript)?;
                (StateEffect::Redraw, ChatProjectionChange::VisibleItem(id))
            },
            TranscriptRecord::CommandCommitted(
                AgentCommand::CreateSession { .. }
                | AgentCommand::RespondToActivity { .. }
                | AgentCommand::InterruptTurn { .. },
            ) => (StateEffect::Unchanged, ChatProjectionChange::Unchanged),
            TranscriptRecord::EventCommitted(event) => self.observe_event(event)?,
        };
        self.views
            .observe_record(
                &projection_record,
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
        self.observe_event(event).map(|(effect, _)| effect)
    }

    fn observe_event(
        &mut self,
        event: AgentEvent,
    ) -> Result<(StateEffect, ChatProjectionChange), StateError> {
        match event {
            AgentEvent::SessionCreated { .. } => {
                Ok((StateEffect::Unchanged, ChatProjectionChange::Unchanged))
            },
            AgentEvent::TurnStarted { .. } => {
                self.turn_active = true;
                Ok((StateEffect::Redraw, ChatProjectionChange::Unchanged))
            },
            AgentEvent::ActivityStarted { activity, kind } => {
                let (item, visible) = self.start_activity(activity, kind)?;
                let change = if visible {
                    ChatProjectionChange::VisibleItem(item)
                } else {
                    ChatProjectionChange::Unchanged
                };
                Ok((StateEffect::Redraw, change))
            },
            AgentEvent::ActivityUpdated { activity, update } => {
                let Some(presentation) = self.activities.get_mut(&activity) else {
                    return Err(StateError::UnknownActivity(activity));
                };
                match update {
                    ActivityUpdate::TextDelta(text) => {
                        if text.is_empty() {
                            return Ok((StateEffect::Unchanged, ChatProjectionChange::Unchanged));
                        }
                        if !presentation.has_payload && activity_label(presentation.kind).is_some()
                        {
                            self.transcript
                                .append_text(presentation.item, "\n")
                                .map_err(StateError::Transcript)?;
                        }
                        self.transcript
                            .append_text(presentation.item, &text)
                            .map_err(StateError::Transcript)?;
                        presentation.has_payload = true;
                        presentation.visible = true;
                        Ok((
                            StateEffect::Redraw,
                            ChatProjectionChange::VisibleItem(presentation.item),
                        ))
                    },
                    ActivityUpdate::TextSnapshot(text) => {
                        let text = project_snapshot(presentation.kind, text);
                        let visible = !text.is_empty();
                        let changed = self
                            .transcript
                            .replace_text_changed(presentation.item, text)
                            .map_err(StateError::Transcript)?;
                        presentation.has_payload = true;
                        presentation.visible = visible;
                        let change = if changed {
                            ChatProjectionChange::VisibleItem(presentation.item)
                        } else {
                            ChatProjectionChange::Unchanged
                        };
                        Ok((StateEffect::Redraw, change))
                    },
                }
            },
            AgentEvent::ActivityFinished { activity, outcome } => {
                let (item, visible) = self.finish_activity(activity, outcome)?;
                let change = if visible {
                    ChatProjectionChange::VisibleItem(item)
                } else {
                    ChatProjectionChange::Unchanged
                };
                Ok((StateEffect::Redraw, change))
            },
            AgentEvent::TurnFinished { outcome, .. } => {
                self.turn_active = false;
                let change = match outcome {
                    TurnOutcome::Completed => ChatProjectionChange::Unchanged,
                    TurnOutcome::Interrupted => ChatProjectionChange::VisibleItem(
                        self.push_notice("Turn interrupted".to_owned())?,
                    ),
                    TurnOutcome::Failed(failure) => ChatProjectionChange::VisibleItem(
                        self.push_notice(format!("Turn failed: {}", failure.message()))?,
                    ),
                };
                Ok((StateEffect::Redraw, change))
            },
        }
    }

    pub(super) fn prepare_frame(
        &self,
        size: Size,
        appearance: &AppearancePin,
    ) -> Result<PreparedFrame, FrameError> {
        self.prepare_frame_with_measure_hook(size, appearance, || {})
    }

    pub(super) fn prepare_frame_with_measure_hook(
        &self,
        size: Size,
        appearance: &AppearancePin,
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
                    &self.transcript,
                    &self.editor,
                    &mut view,
                    snapshot,
                    after_measure,
                )
                .map_err(FrameError::Render)?
        };

        Ok(PreparedFrame {
            surface,
            cursor: frame.cursor,
            appearance_revision: appearance.revision(),
            view_state: frame.state,
        })
    }

    // This is the currently rendered Chat projection. Future Transcript and Request views select
    // their own projections above this state instead of changing the generic RunOutcome boundary.
    pub(super) fn session_output(
        &self,
        appearance: &AppearancePin,
    ) -> Result<Option<String>, TranscriptMeasureError> {
        self.transcript
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
                self.push_notice(
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

    fn start_activity(
        &mut self,
        activity: ActivityRef,
        kind: ActivityKind,
    ) -> Result<(TranscriptItemId, bool), StateError> {
        let id = self.next_transcript_id()?;
        self.transcript
            .start_assistant(id)
            .map_err(StateError::Transcript)?;
        let label = activity_label(kind);
        if let Some(label) = label {
            self.transcript
                .append_text(id, label)
                .map_err(StateError::Transcript)?;
        }
        self.activities.insert(
            activity,
            ActivityPresentation {
                item: id,
                kind,
                has_payload: false,
                visible: label.is_some(),
            },
        );
        let request = match kind {
            ActivityKind::ApprovalRequest { request_id } => Some(PendingRequest::Approval(
                ActivityRequestRef::new(activity, request_id),
            )),
            ActivityKind::UserInputRequest { request_id } => Some(PendingRequest::UserInput(
                ActivityRequestRef::new(activity, request_id),
            )),
            _ => None,
        };
        if let Some(request) = request {
            self.pending_requests.push_back(request);
        }
        Ok((id, label.is_some()))
    }

    fn finish_activity(
        &mut self,
        activity: ActivityRef,
        outcome: ActivityOutcome,
    ) -> Result<(TranscriptItemId, bool), StateError> {
        let Some(presentation) = self.activities.remove(&activity) else {
            return Err(StateError::UnknownActivity(activity));
        };
        let id = presentation.item;
        let visible = match outcome {
            ActivityOutcome::Completed => !presentation.visible,
            ActivityOutcome::Interrupted => self
                .transcript
                .append_text(id, "\nInterrupted")
                .map(|()| true)
                .map_err(StateError::Transcript)?,
            ActivityOutcome::Failed(failure) => self
                .transcript
                .append_text(id, &format!("\nFailed: {}", failure.message()))
                .map(|()| true)
                .map_err(StateError::Transcript)?,
        };
        self.transcript
            .finalize(id)
            .map_err(StateError::Transcript)?;
        self.pending_requests
            .retain(|request| request.activity() != activity);
        Ok((id, visible))
    }

    fn push_notice(&mut self, text: String) -> Result<TranscriptItemId, StateError> {
        let id = self.next_transcript_id()?;
        self.transcript
            .start_assistant(id)
            .map_err(StateError::Transcript)?;
        self.transcript
            .append_text(id, &text)
            .map_err(StateError::Transcript)?;
        self.transcript
            .finalize(id)
            .map_err(StateError::Transcript)?;
        Ok(id)
    }

    fn next_transcript_id(&mut self) -> Result<TranscriptItemId, StateError> {
        let id = TranscriptItemId::new(self.next_item_id);
        self.next_item_id = self
            .next_item_id
            .checked_add(1)
            .ok_or(StateError::ItemIdOverflow)?;
        Ok(id)
    }
}

impl PendingRequest {
    const fn activity(self) -> ActivityRef {
        match self {
            Self::Approval(request) | Self::UserInput(request) => request.activity(),
        }
    }
}

fn project_snapshot(kind: ActivityKind, text: String) -> String {
    let Some(label) = activity_label(kind) else {
        return text;
    };
    if text.is_empty() {
        label.to_owned()
    } else {
        format!("{label}\n{text}")
    }
}

const fn activity_label(kind: ActivityKind) -> Option<&'static str> {
    match kind {
        ActivityKind::ModelWork => Some("Thinking…"),
        ActivityKind::AgentMessage => None,
        ActivityKind::ToolCall => Some("Running tool…"),
        ActivityKind::ToolResult => Some("Tool result"),
        ActivityKind::FileChange => Some("File change observed"),
        ActivityKind::ApprovalRequest { .. } => {
            Some("Approval required: enter `y` to approve or `n` to decline.")
        },
        ActivityKind::ApprovalResponse { .. } => Some("Approval response sent"),
        ActivityKind::UserInputRequest { .. } => Some("Agent requested input"),
        ActivityKind::UserInputResponse { .. } => Some("Input response sent"),
    }
}

#[cfg(test)]
impl TuiState {
    pub(super) fn transcript(&self) -> &TranscriptState {
        &self.transcript
    }

    pub(super) fn editor(&self) -> &PromptEditor {
        &self.editor
    }

    pub(super) fn set_next_item_id(&mut self, value: u64) {
        self.next_item_id = value;
    }

    pub(super) const fn turn_active(&self) -> bool {
        self.turn_active
    }

    pub(super) fn views(&self) -> &ObservabilityViews {
        &self.views
    }
}
