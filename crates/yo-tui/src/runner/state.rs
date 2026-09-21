use std::collections::{HashSet, VecDeque};

use yo_core::{
    ActivityRef, ActivityRequestRef, ImagePreparationRequest, InputSubmission, JournalDurability,
    SkillReferenceSearchRequest, SubmissionId, TurnRef, UserInput, WorkspaceReferenceSearchRequest,
    secret_store::{SecretDestination, SecretStore},
};

#[cfg(test)]
use crate::transcript::TranscriptState;
use crate::{
    PromptTemplates,
    command::CommandPalette,
    input::{
        editor::PromptEditor,
        secret::{PromptInputView, SecretEditor},
    },
    overlay::{
        AcceptanceReceipt, OverlayInstanceToken, PanelSnapshot, PromptOverlaySlot, SelectionPanel,
        SlotError,
    },
    prompt::{self, assist::PromptAssistController},
    runner::{
        AgentAction, ExternalEditorSnapshot, ForkPickerToken, PresentationMode,
        attention::{AttentionKind, AttentionState},
        chat::ChatProjection,
        model::ModelSelectionState,
        session::{TuiSessionInfo, TuiStatusLine},
        view::ObservabilityViews,
    },
    surface::Size,
    transcript::TranscriptStateError,
};

mod commands;
mod external_editor;
mod find;
mod history;
mod image;
mod input;
mod interview;
#[cfg(test)]
mod interview_tests;
mod observation;
mod presentation;
mod preview;
mod requests;

pub use history::RestoredPromptHistory;
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
    CopyToClipboard(String),
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
    pub(super) interview: Option<super::interview::InterviewController>,
    preview: Option<Box<preview::Preview>>,
    preview_mode: bool,
    chat: ChatProjection,
    editor: PromptEditor,
    secret_editor: Option<SecretEditor>,
    pub(super) secret_store: Option<SecretStore>,
    pub(super) secret_destination: Option<SecretDestination>,
    views: ObservabilityViews,
    pending_requests: VecDeque<PendingRequest>,
    /// User-input requests stay blocked until their typed presentation arrives.
    request_presentations_seen: HashSet<ActivityRef>,
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
    prompt_history: history::PromptHistory,
    recall_picker: Option<history::RecallPicker>,
    find_picker: Option<find::FindPicker>,
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
    external_editor_requested: bool,
    external_editor_snapshot: Option<ExternalEditorSnapshot>,
    external_editor_generation: u64,
    attention: AttentionState,
    presented_secret_request: Option<ActivityRequestRef>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingRequest {
    Approval(ActivityRequestRef),
    PresentationPending(ActivityRequestRef),
    PresentationInvalid(ActivityRequestRef),
    UserInput(ActivityRequestRef),
    SecretInput(ActivityRequestRef),
}

impl TuiState {
    pub(super) fn chat_notice(&mut self, notice: String) -> Result<(), StateError> {
        self.chat.push_notice(notice).map(|_| ())
    }

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

    pub(super) fn set_notifications_enabled(&mut self, enabled: bool) {
        self.attention.set_enabled(enabled);
    }

    pub(super) fn set_notification_history_cutoff(&mut self, turn: Option<TurnRef>) {
        self.attention.set_history_cutoff(turn);
    }

    pub(super) fn arm_notifications(&mut self) {
        if self.preview.is_none() {
            self.attention.arm();
        }
    }

    pub(super) fn take_attention_bell(&mut self, continuation_dispatched: bool) -> bool {
        if continuation_dispatched || self.preview.is_some() {
            return false;
        }
        match self.attention.pending() {
            Some(AttentionKind::Request(request)) => {
                let request_is_front = self
                    .pending_requests
                    .front()
                    .and_then(|pending| pending.attention_request())
                    .is_some_and(|front| front == request);
                let request_is_presented = (self.presented_secret_request == Some(request)
                    && self.views.active() == super::view::ObservabilityView::Chat)
                    || self.request_overlay.is_some_and(|(pending, token)| {
                        pending.attention_request() == Some(request)
                            && self.overlay.is_current(token)
                            && self.overlay.can_submit_current()
                    });
                if request_is_front && request_is_presented {
                    return self.attention.take_request().is_some();
                }
                if !request_is_front {
                    self.attention.observe_request_finished(request.activity());
                }
                false
            },
            Some(AttentionKind::Turn(turn)) if self.pending_requests.is_empty() => {
                if self.active_turn.is_some()
                    || (!self.follow_ups.is_empty() && !self.follow_ups_paused)
                    || !self.pending_submissions.is_empty()
                    || self.starting_submission.is_some()
                {
                    return false;
                }
                self.attention.take_turn(turn)
            },
            _ => false,
        }
    }

    pub(super) fn clear_editor(&mut self) {
        self.command_palette.clear_literal();
        let length = self.editor.text().len();
        self.editor.replace_range(0..length, "");
        self.prompt_assist.prompt_cleared(&mut self.overlay);
    }

    pub(super) fn clear_secret_editor(&mut self) {
        if let Some(editor) = self.secret_editor.as_mut() {
            editor.clear();
        }
        self.secret_editor = None;
    }

    pub(super) fn presentation_blocked(&self) -> bool {
        matches!(
            self.pending_requests.front(),
            Some(PendingRequest::PresentationPending(_) | PendingRequest::PresentationInvalid(_))
        )
    }

    pub(super) fn is_secret_input(&self) -> bool {
        matches!(
            self.pending_requests.front(),
            Some(PendingRequest::SecretInput(_))
        )
    }

    #[cfg(test)]
    pub(super) fn has_secret_editor(&self) -> bool {
        self.secret_editor.is_some()
    }

    pub(super) fn prompt_input(&self) -> PromptInputView<'_> {
        match self.pending_requests.front() {
            Some(PendingRequest::PresentationPending(_)) => {
                PromptInputView::Waiting("Waiting for question")
            },
            Some(PendingRequest::PresentationInvalid(_)) => {
                PromptInputView::Waiting("Question unavailable")
            },
            Some(PendingRequest::SecretInput(_)) => self.secret_editor.as_ref().map_or(
                PromptInputView::Waiting("Waiting for question"),
                PromptInputView::Secret,
            ),
            _ => PromptInputView::Ordinary(&self.editor),
        }
    }

    pub(super) fn restore_draft(&mut self, draft: &str) {
        let length = self.editor.text().len();
        self.editor.replace_range(0..length, draft);
    }

    pub(super) fn cancel_model_switches(&mut self) {
        self.pending_model_selection = None;
        self.reserved_model_selection = None;
    }

    pub(super) fn commit_frame(&mut self, frame: &PreparedFrame) {
        if let Some(preview) = self.preview.as_mut() {
            preview.state.commit_frame(frame);
            return;
        }
        if let Some(width) = prompt::content_width(frame.surface.size().width) {
            self.editor.set_layout_width(width);
        }
        self.views.commit(frame.view_state);
        self.presented_secret_request = frame.secret_request_presented.filter(|request| {
            self.pending_requests
                .front()
                .and_then(|pending| pending.attention_request())
                == Some(*request)
        });
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
}

#[cfg(test)]
impl TuiState {
    pub(super) fn transcript(&self) -> &TranscriptState {
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
