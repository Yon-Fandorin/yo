//! 실행 중인 TUI 상태의 요청 오버레이와 응답 상관관계를 담당한다.

use std::time::{SystemTime, UNIX_EPOCH};

use yo_core::{
    ActivityApproval, ActivityQuestion, ActivityRef, ActivityRequestRef, ApprovalDecision,
};

use super::{PendingRequest, StateEffect, StateError, TuiState};
use crate::{
    input::{event::InputEvent, secret::SecretEditor},
    overlay::{AcceptanceReceipt, OverlayInstanceToken, PanelSnapshot, SelectionEntry, SlotError},
    runner::{AgentAction, view::ObservabilityView},
};

impl TuiState {
    pub(in crate::runner) fn has_pending_request(&self) -> bool {
        !self.pending_requests.is_empty()
    }

    pub(in crate::runner) fn wants_overlay_input(&self, input: &InputEvent) -> bool {
        self.overlay.wants_input(input)
    }

    pub(in crate::runner) fn wants_global_input(&self, input: &InputEvent) -> bool {
        self.views.wants_global_input(input)
    }

    pub(in crate::runner) fn open_overlay(
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

    pub(in crate::runner) fn refresh_overlay(
        &mut self,
        token: OverlayInstanceToken,
        snapshot: PanelSnapshot,
    ) -> Result<(), SlotError> {
        self.overlay.refresh(token, snapshot)
    }

    pub(in crate::runner) fn close_overlay(
        &mut self,
        token: OverlayInstanceToken,
    ) -> Result<(), SlotError> {
        self.overlay.close(token)
    }

    pub(in crate::runner) fn take_overlay_acceptance(&mut self) -> Option<AcceptanceReceipt> {
        self.accepted_overlays.pop_front()
    }

    pub(super) fn close_request_overlay(&mut self) {
        self.request_panel = None;
        if let Some((_, token)) = self.request_overlay.take()
            && self.overlay.is_current(token)
        {
            self.overlay.close_current();
        }
    }

    pub(super) fn sync_request_overlay(&mut self) -> Result<(), StateError> {
        self.normalize_secret_request();
        let pending = self.pending_requests.front().copied();
        if pending.is_some() {
            self.cancel_recall_picker();
            self.cancel_find_picker();
        }
        if self.question_notes_refresh != pending {
            self.question_notes_refresh = None;
        }
        if self
            .question_notes
            .is_some_and(|(request, _)| Some(request) != pending)
        {
            self.question_notes = None;
        }
        if matches!(
            pending,
            Some(
                PendingRequest::PresentationPending(_)
                    | PendingRequest::PresentationInvalid(_)
                    | PendingRequest::SecretInput(_)
            )
        ) {
            self.restored_question_draft = None;
            self.question_notes = None;
            self.saved_request_panel = None;
            self.close_request_overlay();
            return Ok(());
        }
        if self.restored_question_draft != pending {
            self.restored_question_draft = None;
            if let Some(request @ PendingRequest::UserInput(_)) = pending
                && let Some(question) = self.chat.question(request.activity())
                && let Some(draft) = &question.draft
            {
                // host draft는 아직 편집되지 않은 editor를 한 번 채운다. 이후 snapshot은
                // 사용자의 입력을 덮어쓰지 않는다.
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

    pub(super) fn reject_referenced_answer(&mut self) -> Result<bool, StateError> {
        if !self.prompt_assist.has_accepted_references() {
            return Ok(false);
        }
        self.chat.push_notice(
            "Selected references cannot answer an agent question. Remove them from the answer draft first."
                .to_owned(),
        )?;
        Ok(true)
    }

    fn normalize_secret_request(&mut self) {
        let Some(pending) = self.pending_requests.front().copied() else {
            self.clear_secret_editor();
            return;
        };
        let request = match pending {
            PendingRequest::PresentationPending(request)
            | PendingRequest::PresentationInvalid(request)
            | PendingRequest::UserInput(request)
            | PendingRequest::SecretInput(request) => request,
            PendingRequest::Approval(_) => {
                self.clear_secret_editor();
                return;
            },
        };
        let question = self.chat.question(request.activity());
        if matches!(
            pending,
            PendingRequest::PresentationPending(_) | PendingRequest::PresentationInvalid(_)
        ) {
            match question {
                Some(question) if question.is_secret => {
                    self.pending_requests[0] = PendingRequest::SecretInput(request);
                    self.clear_secret_editor();
                    self.secret_editor = Some(self.secret_editor_for(request));
                },
                Some(_) => {
                    self.pending_requests[0] = PendingRequest::UserInput(request);
                    self.clear_secret_editor();
                },
                None if matches!(pending, PendingRequest::PresentationPending(_)) => {
                    if self
                        .request_presentations_seen
                        .contains(&request.activity())
                    {
                        self.pending_requests[0] = PendingRequest::PresentationInvalid(request);
                    }
                    self.clear_secret_editor();
                },
                None => self.clear_secret_editor(),
            }
            return;
        }
        let is_secret = question.is_some_and(|question| question.is_secret);
        match (pending, is_secret) {
            (PendingRequest::UserInput(request), true) => {
                self.pending_requests[0] = PendingRequest::SecretInput(request);
                self.clear_secret_editor();
                self.secret_editor = Some(self.secret_editor_for(request));
            },
            (PendingRequest::SecretInput(_), true) => {
                if self
                    .secret_editor
                    .as_ref()
                    .is_none_or(|editor| !editor.ready())
                {
                    self.secret_editor = Some(self.secret_editor_for(request));
                }
            },
            (PendingRequest::SecretInput(request), false) => {
                // A request already classified as secret never falls back to the ordinary
                // editor when its presentation disappears or changes shape.
                self.pending_requests[0] = PendingRequest::PresentationInvalid(request);
                self.clear_secret_editor();
            },
            _ => {
                self.clear_secret_editor();
            },
        }
    }

    fn secret_editor_for(&self, request: ActivityRequestRef) -> SecretEditor {
        let mut editor = SecretEditor::new();
        if self
            .chat
            .question(request.activity())
            .is_some_and(|question| question.storage_offer.is_some())
        {
            editor = editor.with_retention_offer(
                self.secret_store.is_some() && self.secret_destination.is_some(),
            );
        }
        editor.mark_ready();
        if self
            .interview
            .as_ref()
            .is_some_and(|controller| controller.recovery_available(request).unwrap_or(false))
        {
            editor.mark_recovery_available();
        }
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
            if store.list(now).is_ok_and(|entries| {
                entries.iter().any(|entry| {
                    entry.destination() == destination && entry.scope() == offer.scope.as_str()
                })
            }) {
                editor.mark_recovery_available();
            }
        }
        editor
    }

    pub(super) fn request_response(
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
            PendingRequest::SecretInput(_) => Ok(StateEffect::Unchanged),
            PendingRequest::PresentationPending(_) | PendingRequest::PresentationInvalid(_) => {
                Ok(StateEffect::Unchanged)
            },
        }
    }

    pub(super) fn approval_response(
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
            Self::PresentationPending(_) => (
                "Waiting for question",
                vec![SelectionEntry::status(
                    "waiting",
                    "The request is waiting for its typed presentation.",
                )],
            ),
            Self::PresentationInvalid(_) => (
                "Question unavailable",
                vec![SelectionEntry::status(
                    "unavailable",
                    "This request cannot accept input until its presentation is repaired.",
                )],
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
            Self::SecretInput(_) => (
                "Secret input",
                vec![SelectionEntry::status(
                    "secret",
                    "Enter submits the value. Esc cancels.",
                )],
            ),
        };
        match PanelSnapshot::new(title, entries) {
            Ok(panel) => panel.for_request(matches!(self, Self::Approval(_))),
            Err(_) if question.is_some() => self.panel(None, None),
            Err(_) => unreachable!("static request panel is valid"),
        }
    }

    pub(super) const fn activity(self) -> ActivityRef {
        match self {
            Self::Approval(request)
            | Self::PresentationPending(request)
            | Self::PresentationInvalid(request)
            | Self::UserInput(request)
            | Self::SecretInput(request) => request.activity(),
        }
    }

    pub(super) const fn attention_request(self) -> Option<ActivityRequestRef> {
        match self {
            Self::Approval(request) | Self::UserInput(request) | Self::SecretInput(request) => {
                Some(request)
            },
            Self::PresentationPending(_) | Self::PresentationInvalid(_) => None,
        }
    }
}
