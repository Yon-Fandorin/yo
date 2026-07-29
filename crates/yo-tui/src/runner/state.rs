use std::{
    collections::{HashMap, VecDeque},
    time::Duration,
};

use yo_core::{
    ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef, ActivityUpdate, AgentEvent,
    ApprovalDecision, TurnOutcome,
};

use crate::{
    input::{
        editor::{EditorEffect, PromptEditor},
        event::InputEvent,
    },
    runner::AgentAction,
    shell::{self, AgentShellRenderError, AgentShellStyles, AgentShellViewState},
    surface::{Point, Rect, Size, Style, Surface, SurfaceError},
    transcript::{
        TranscriptItemId, TranscriptLayoutConfig, TranscriptMeasureError, TranscriptState,
        TranscriptStateError, TranscriptStyles,
    },
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum StateEffect {
    Unchanged,
    Redraw,
    Dispatch(AgentAction),
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
    Render(AgentShellRenderError),
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
    view_state: AgentShellViewState,
}

#[derive(Debug, Default)]
pub(super) struct TuiState {
    transcript: TranscriptState,
    editor: PromptEditor,
    view: AgentShellViewState,
    next_item_id: u64,
    activities: HashMap<ActivityRef, ActivityPresentation>,
    pending_requests: VecDeque<PendingRequest>,
    turn_active: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActivityPresentation {
    item: TranscriptItemId,
    kind: ActivityKind,
    has_payload: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingRequest {
    Approval(ActivityRequestRef),
    UserInput(ActivityRequestRef),
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

        let editor_before = self.editor.clone();
        let effect = self.editor.handle(input, self.turn_active, now);
        match effect {
            EditorEffect::BufferChanged => Ok(StateEffect::Redraw),
            EditorEffect::Submitted(text) => {
                if let Some(request) = self.pending_requests.front().copied() {
                    return self.request_response(request, text);
                }
                let id = TranscriptItemId::new(self.next_item_id);
                let Some(next) = self.next_item_id.checked_add(1) else {
                    self.editor = editor_before;
                    return Err(StateError::ItemIdOverflow);
                };
                if let Err(error) = self.transcript.push_user(id, text.clone()) {
                    self.editor = editor_before;
                    return Err(StateError::Transcript(error));
                }
                self.next_item_id = next;
                Ok(StateEffect::Dispatch(AgentAction::Submit(text)))
            },
            EditorEffect::Exit => Ok(StateEffect::Exit),
            EditorEffect::InterruptTask => Ok(StateEffect::Dispatch(AgentAction::Interrupt)),
            EditorEffect::Unhandled | EditorEffect::NoChange | EditorEffect::ExitArmed => {
                Ok(StateEffect::Unchanged)
            },
        }
    }

    pub(super) fn observe(&mut self, event: AgentEvent) -> Result<StateEffect, StateError> {
        match event {
            AgentEvent::SessionCreated { .. } => return Ok(StateEffect::Unchanged),
            AgentEvent::TurnStarted { .. } => {
                self.turn_active = true;
                return Ok(StateEffect::Redraw);
            },
            AgentEvent::ActivityStarted { activity, kind } => {
                self.start_activity(activity, kind)?;
            },
            AgentEvent::ActivityUpdated { activity, update } => {
                let Some(presentation) = self.activities.get_mut(&activity) else {
                    return Err(StateError::UnknownActivity(activity));
                };
                match update {
                    ActivityUpdate::TextDelta(text) => {
                        if text.is_empty() {
                            return Ok(StateEffect::Unchanged);
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
                    },
                    ActivityUpdate::TextSnapshot(text) => {
                        let text = project_snapshot(presentation.kind, text);
                        self.transcript
                            .replace_text(presentation.item, text)
                            .map_err(StateError::Transcript)?;
                        presentation.has_payload = true;
                    },
                }
            },
            AgentEvent::ActivityFinished { activity, outcome } => {
                self.finish_activity(activity, outcome)?;
            },
            AgentEvent::TurnFinished { outcome, .. } => {
                self.turn_active = false;
                match outcome {
                    TurnOutcome::Completed => {},
                    TurnOutcome::Interrupted => {
                        self.push_notice("Turn interrupted".to_owned())?;
                    },
                    TurnOutcome::Failed(failure) => {
                        self.push_notice(format!("Turn failed: {}", failure.message()))?;
                    },
                }
            },
        }
        Ok(StateEffect::Redraw)
    }

    pub(super) fn prepare_frame(&self, size: Size) -> Result<PreparedFrame, FrameError> {
        let mut surface = Surface::new(size).map_err(FrameError::Allocate)?;
        let mut view_state = self.view;
        let frame = {
            let area = Rect::new(Point::new(0, 0), size);
            let mut view = surface
                .view(area)
                .expect("the complete surface is always a valid view");
            shell::render(
                &self.transcript,
                &self.editor,
                &mut view,
                &TranscriptLayoutConfig::default(),
                default_styles(),
                &mut view_state,
                None,
            )
            .map_err(FrameError::Render)?
        };

        Ok(PreparedFrame {
            surface,
            cursor: frame.cursor,
            view_state,
        })
    }

    // This is the currently rendered Chat projection. Future Transcript and Request views select
    // their own projections above this state instead of changing the generic RunOutcome boundary.
    pub(super) fn session_output(&self) -> Result<Option<String>, TranscriptMeasureError> {
        self.transcript
            .plain_output(&TranscriptLayoutConfig::default())
    }

    pub(super) fn has_pending_request(&self) -> bool {
        !self.pending_requests.is_empty()
    }

    pub(super) fn commit_frame(&mut self, frame: &PreparedFrame) {
        self.view = frame.view_state;
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
    ) -> Result<(), StateError> {
        let id = self.next_transcript_id()?;
        self.transcript
            .start_assistant(id)
            .map_err(StateError::Transcript)?;
        if let Some(label) = activity_label(kind) {
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
        Ok(())
    }

    fn finish_activity(
        &mut self,
        activity: ActivityRef,
        outcome: ActivityOutcome,
    ) -> Result<(), StateError> {
        let Some(presentation) = self.activities.remove(&activity) else {
            return Err(StateError::UnknownActivity(activity));
        };
        let id = presentation.item;
        match outcome {
            ActivityOutcome::Completed => {},
            ActivityOutcome::Interrupted => self
                .transcript
                .append_text(id, "\nInterrupted")
                .map_err(StateError::Transcript)?,
            ActivityOutcome::Failed(failure) => self
                .transcript
                .append_text(id, &format!("\nFailed: {}", failure.message()))
                .map_err(StateError::Transcript)?,
        }
        self.transcript
            .finalize(id)
            .map_err(StateError::Transcript)?;
        self.pending_requests
            .retain(|request| request.activity() != activity);
        Ok(())
    }

    fn push_notice(&mut self, text: String) -> Result<(), StateError> {
        let id = self.next_transcript_id()?;
        self.transcript
            .start_assistant(id)
            .map_err(StateError::Transcript)?;
        self.transcript
            .append_text(id, &text)
            .map_err(StateError::Transcript)?;
        self.transcript.finalize(id).map_err(StateError::Transcript)
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

const fn default_styles() -> AgentShellStyles {
    let style = Style::new(
        crate::surface::Color::Default,
        crate::surface::Color::Default,
        crate::surface::Attributes::empty(),
    );
    AgentShellStyles {
        transcript: TranscriptStyles {
            background: style,
            user_marker: style,
            user_body: style,
            assistant_marker: style,
            assistant_body: style,
        },
        prompt: style,
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
}
