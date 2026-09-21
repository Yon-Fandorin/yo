//! 실행 중인 TUI 상태의 저널과 Chat 관찰을 담당한다.

use yo_core::{
    ActivityKind, ActivityNotice, ActivityRequestRef, AgentControlOutcome, AgentEvent,
    DurabilityGapCause, JournalDurability, RequestTraceEntry, SkillReferenceSearchUpdate,
    TranscriptRecord, TurnOutcome, WorkspaceReferenceSearchUpdate,
    session_repository::{DurableCutoff, InheritedSessionHistory},
};

use super::{PendingRequest, StateEffect, StateError, TuiState};
use crate::{
    appearance::AppearancePin,
    input::secret::SecretEditor,
    runner::{chat::ChatProjectionChange, session::TuiDocument},
    transcript::TranscriptMeasureError,
};

impl TuiState {
    pub(in crate::runner) fn observe_control_outcome(
        &mut self,
        outcome: AgentControlOutcome,
    ) -> Result<StateEffect, StateError> {
        match outcome {
            AgentControlOutcome::ContextCompactionRejected { detail } => {
                self.context_compaction_pending = false;
                self.chat
                    .push_notice(format!("Context compaction was not started.\n{detail}"))?;
            },
            AgentControlOutcome::ActivityResponseRejected { request, .. } => {
                // The worker retains this exact outstanding request. Re-open only its
                // request-bound secret editor and discard the submitted value before the
                // next frame can accept another event.
                self.pending_requests
                    .retain(|pending| pending.activity() != request.activity());
                self.pending_requests
                    .push_front(PendingRequest::SecretInput(request));
                self.clear_secret_editor();
                let mut editor = SecretEditor::new();
                editor.mark_ready();
                self.secret_editor = Some(editor);
                self.close_request_overlay();
                self.chat.push_notice(
                    "Secret input was rejected before transmission; enter it again.".to_owned(),
                )?;
                self.sync_request_overlay()?;
            },
        }
        Ok(StateEffect::Redraw)
    }

    pub(in crate::runner) fn observe_inherited_history(
        &mut self,
        history: &InheritedSessionHistory,
    ) -> Result<(), StateError> {
        self.chat.observe_inherited_history(history)?;
        self.views
            .observe_inherited_history(history)
            .map_err(StateError::Transcript)
    }

    pub(in crate::runner) fn observe_document(
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

    pub(in crate::runner) fn observe_notice(
        &mut self,
        notice: ActivityNotice,
    ) -> Result<StateEffect, StateError> {
        self.chat.push_session_notice(notice)?;
        Ok(StateEffect::Redraw)
    }

    pub(in crate::runner) fn observe_record(
        &mut self,
        record: TranscriptRecord,
    ) -> Result<StateEffect, StateError> {
        if let Some(controller) = &mut self.interview
            && let Some(notice) = controller.observe(&record)
        {
            self.chat.push_notice(notice)?;
        }
        let lifecycle_effect = self.observe_live_lifecycle(&record)?;
        let chat_change = self.chat.observe_record(&record)?;
        if let TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated { activity, .. }) =
            &record
            && let Some(request) = self
                .pending_requests
                .front()
                .filter(|request| request.activity() == *activity)
                .copied()
        {
            if matches!(
                request,
                PendingRequest::PresentationPending(_) | PendingRequest::PresentationInvalid(_)
            ) {
                // A matching update closes the pending window even when decoding the typed
                // presentation failed; normalize_secret_request then keeps input fail-closed.
                self.request_presentations_seen.insert(*activity);
            } else if matches!(
                request,
                PendingRequest::UserInput(_) | PendingRequest::SecretInput(_)
            ) && self.chat.question(*activity).is_none()
            {
                // Once a request has been presented, a later malformed update must not
                // silently demote a secret editor into the ordinary draft editor.
                let request = match request {
                    PendingRequest::UserInput(request) | PendingRequest::SecretInput(request) => {
                        request
                    },
                    _ => unreachable!("the request was checked above"),
                };
                self.pending_requests[0] = PendingRequest::PresentationInvalid(request);
                self.clear_secret_editor();
            }
        }
        if let TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated { activity, .. }) =
            &record
            && let Some(controller) = &self.interview
            && let Some((choice, draft)) = controller.live_draft(*activity)
        {
            self.chat.set_interview_draft(*activity, choice, draft);
        }
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
        if let Some(request) = self
            .pending_requests
            .front()
            .and_then(|request| request.attention_request())
        {
            self.attention.observe_request(request);
        }
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
            // 연결된 diff는 request profile이 변경되지 않았더라도 approval context로 취급한다.
            // revision을 갱신하면 변경 전에 준비된 frame의 commit을 거부한다.
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

    pub(in crate::runner) fn observe_request_trace(&mut self, entry: RequestTraceEntry) {
        self.views.observe_request_trace(entry);
    }

    pub(in crate::runner) fn observe_durability(
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

    pub(in crate::runner) fn enable_workspace_references(&mut self) {
        self.prompt_assist.enable_workspace();
    }

    pub(in crate::runner) fn enable_skill_references(&mut self) {
        self.prompt_assist.enable_skill();
    }

    pub(in crate::runner) fn observe_workspace_reference_update(
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

    pub(in crate::runner) fn observe_workspace_reference_failure(
        &mut self,
        reason: String,
    ) -> StateEffect {
        if self
            .prompt_assist
            .workspace_failed(reason, &mut self.overlay)
        {
            StateEffect::Redraw
        } else {
            StateEffect::Unchanged
        }
    }

    pub(in crate::runner) fn observe_skill_reference_update(
        &mut self,
        update: SkillReferenceSearchUpdate,
    ) -> StateEffect {
        if self.prompt_assist.observe_skill(update, &mut self.overlay) {
            StateEffect::Redraw
        } else {
            StateEffect::Unchanged
        }
    }

    pub(in crate::runner) fn observe_skill_reference_failure(
        &mut self,
        reason: String,
    ) -> StateEffect {
        if self.prompt_assist.skill_failed(reason, &mut self.overlay) {
            StateEffect::Redraw
        } else {
            StateEffect::Unchanged
        }
    }

    #[cfg(test)]
    pub(in crate::runner) const fn durability(&self) -> Option<JournalDurability> {
        self.durability
    }

    #[cfg(test)]
    pub(in crate::runner) fn observe(
        &mut self,
        event: AgentEvent,
    ) -> Result<StateEffect, StateError> {
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
                self.attention.observe_turn_started();
            },
            AgentEvent::ActivityStarted { activity, kind } => {
                let request = match kind {
                    ActivityKind::ApprovalRequest { request_id } => Some(PendingRequest::Approval(
                        ActivityRequestRef::new(*activity, *request_id),
                    )),
                    ActivityKind::UserInputRequest { request_id } => {
                        Some(PendingRequest::PresentationPending(
                            ActivityRequestRef::new(*activity, *request_id),
                        ))
                    },
                    _ => None,
                };
                if let Some(request) = request {
                    if self.pending_requests.is_empty() {
                        self.overlay.close_current();
                        self.command_palette.dismiss();
                        self.cancel_find_picker();
                        self.model_overlay = None;
                        self.prompt_assist.cancel();
                    }
                    self.pending_requests.push_back(request);
                }
            },
            AgentEvent::ActivityFinished { activity, .. } => {
                if self
                    .presented_secret_request
                    .is_some_and(|request| request.activity() == *activity)
                {
                    self.presented_secret_request = None;
                }
                self.request_presentations_seen.remove(activity);
                self.attention.observe_request_finished(*activity);
                if self
                    .pending_requests
                    .front()
                    .is_some_and(|request| request.activity() == *activity)
                {
                    self.clear_secret_editor();
                }
                self.pending_requests
                    .retain(|request| request.activity() != *activity);
            },
            AgentEvent::TurnFinished { turn, outcome } if self.active_turn == Some(*turn) => {
                self.active_turn = None;
                self.attention.observe_turn_finished(*turn);
                self.request_presentations_seen.clear();
                self.clear_secret_editor();
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
            AgentEvent::ActivityUpdated { activity, .. } => {
                if self
                    .presented_secret_request
                    .is_some_and(|request| request.activity() == *activity)
                {
                    self.presented_secret_request = None;
                }
            },
            AgentEvent::SessionCreated { .. } => {},
        }
        Ok(StateEffect::Unchanged)
    }

    // 현재 렌더링되는 Chat projection이다. 향후 Transcript와 Request view는 일반 RunOutcome
    // 경계를 변경하지 않고 이 state 위에서 각자의 projection을 선택한다.

    pub(in crate::runner) fn session_output(
        &self,
        appearance: &AppearancePin,
    ) -> Result<Option<String>, TranscriptMeasureError> {
        let transcript = self.chat.transcript();
        transcript.plain_output_slice(
            transcript.suffix(self.chat.published_item_count()),
            appearance.snapshot().transcript_config(),
        )
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
