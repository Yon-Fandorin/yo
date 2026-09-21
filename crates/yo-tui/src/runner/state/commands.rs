//! 실행 중인 TUI 상태의 슬래시 명령과 세션 전환을 담당한다.

use std::{
    mem,
    time::{SystemTime, UNIX_EPOCH},
};

use yo_core::{ActivityDocument, JournalDurability, secret_store::SecretMetadata};

use super::{PendingRequest, StateEffect, StateError, TuiState};
use crate::{
    command::{
        CommandEffect, CommandRegistry, compact_argument, fork_argument, model_argument,
        prompt_argument, resume_argument, secrets_argument, tree_argument,
    },
    overlay::{PanelSnapshot, SlotError},
    runner::{
        AgentAction, ForkPickerToken, chat::CopyAnswer, model::ModelSelectionState,
        session::TuiDocument,
    },
    terminal::clipboard::MAX_TEXT_BYTES,
};

impl TuiState {
    pub(in crate::runner) fn report_clipboard_sent(&mut self) -> Result<(), StateError> {
        let notice = "Sent answer to the terminal clipboard; paste to confirm.".to_owned();
        if let Some(preview) = self.preview.as_mut() {
            preview.state.chat.push_notice(notice)?;
        } else {
            self.chat.push_notice(notice)?;
        }
        Ok(())
    }

    pub(in crate::runner) fn enable_model_selection(
        &mut self,
        controller: yo_core::ModelSelectionController,
    ) {
        self.model_selection = Some(ModelSelectionState::new(controller));
    }

    pub(in crate::runner) fn take_session_tree_request(&mut self) -> bool {
        mem::take(&mut self.session_tree_requested)
    }

    pub(in crate::runner) fn report_session_tree_failure(&mut self, detail: String) {
        let _ = self
            .chat
            .push_notice(format!("Session tree could not be opened: {detail}"));
    }

    pub(in crate::runner) fn take_fork_session_request(&mut self) -> bool {
        mem::take(&mut self.fork_session_requested)
    }

    pub(in crate::runner) fn take_fork_picker_request(&mut self) -> bool {
        mem::take(&mut self.fork_picker_requested)
    }

    pub(in crate::runner) fn take_fork_boundary_request(
        &mut self,
    ) -> Option<(ForkPickerToken, usize)> {
        self.fork_boundary_requested.take()
    }

    pub(in crate::runner) fn show_fork_picker(
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

    pub(in crate::runner) fn cancel_fork_picker(&mut self) {
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

    pub(in crate::runner) fn report_fork_started(&mut self, parent: yo_core::SessionId) {
        self.fork_draft = None;
        let _ = self.chat.push_notice(format!(
            "Forked from {parent}. Inherited context is preserved."
        ));
    }

    pub(in crate::runner) fn report_fork_failure(&mut self, detail: String) {
        self.cancel_fork_picker();
        let _ = self.chat.push_notice(format!(
            "Fork was not started; your current session remains open: {detail}"
        ));
    }

    pub(in crate::runner) fn report_fork_cleanup_failure(&mut self, detail: String) {
        let _ = self.chat.push_notice(format!(
            "Fork started, but previous session cleanup failed: {detail}"
        ));
    }

    pub(in crate::runner) fn take_new_session_request(&mut self) -> bool {
        mem::take(&mut self.new_session_requested)
    }

    pub(in crate::runner) fn report_new_session_cleanup_failure(&mut self, detail: String) {
        let _ = self.chat.push_notice(format!(
            "New session started, but previous session cleanup failed: {detail}"
        ));
    }

    pub(in crate::runner) fn report_new_session_failure(&mut self, detail: String) {
        let _ = self.chat.push_notice(format!(
            "New session was not started; your current session remains open: {detail}"
        ));
    }

    pub(in crate::runner) fn take_model_selection(&mut self) -> Option<yo_core::ModelPickerTarget> {
        self.pending_model_selection.take()
    }

    pub(in crate::runner) fn report_model_switch_failure(&mut self, detail: String) {
        let _ = self.chat.push_notice(format!(
            "Model switch failed; the previous model remains active: {detail}"
        ));
    }

    pub(in crate::runner) fn commit_model_switch(
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

    pub(super) fn handle_model_command(
        &mut self,
        text: &str,
        draft: &str,
    ) -> Result<StateEffect, StateError> {
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

    pub(in crate::runner) fn take_resume_session_request(
        &mut self,
    ) -> Option<Option<yo_core::SessionId>> {
        self.resume_session_requested.take()
    }

    pub(in crate::runner) fn show_resume_picker(
        &mut self,
        panel: PanelSnapshot,
    ) -> Result<(), SlotError> {
        self.resume_overlay = Some(self.open_overlay(panel)?);
        Ok(())
    }

    pub(in crate::runner) fn report_resume_failure(&mut self, detail: String) {
        let _ = self.chat.push_notice(format!(
            "Session was not resumed; your current conversation remains open: {detail}"
        ));
    }

    pub(in crate::runner) fn report_resume_cleanup_failure(&mut self, detail: String) {
        let _ = self.chat.push_notice(format!(
            "Saved session resumed, but previous session cleanup failed: {detail}"
        ));
    }

    pub(super) fn allow_session_transition(&mut self, draft: &str) -> Result<bool, StateError> {
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

    pub(super) fn handle_tree_command(
        &mut self,
        text: &str,
        draft: &str,
    ) -> Result<StateEffect, StateError> {
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

    pub(super) fn handle_fork_command(
        &mut self,
        text: &str,
        draft: &str,
    ) -> Result<StateEffect, StateError> {
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

    pub(super) fn handle_resume_command(
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

    pub(super) fn execute_command(
        &mut self,
        effect: CommandEffect,
        invocation: &str,
        draft: &str,
    ) -> Result<StateEffect, StateError> {
        match effect {
            CommandEffect::CopyAnswer => {
                self.clear_editor();
                let Some(answer) = self.chat.last_completed_answer() else {
                    self.chat
                        .push_notice("No completed assistant answer to copy.".to_owned())?;
                    return Ok(StateEffect::Redraw);
                };
                let CopyAnswer::Text(answer) = answer else {
                    self.chat.push_notice(
                        "The latest completed answer has no copyable text.".to_owned(),
                    )?;
                    return Ok(StateEffect::Redraw);
                };
                if answer.len() > MAX_TEXT_BYTES {
                    self.chat.push_notice(format!(
                        "Answer is too large for terminal clipboard ({} bytes; limit {MAX_TEXT_BYTES}).",
                        answer.len()
                    ))?;
                    return Ok(StateEffect::Redraw);
                }
                Ok(StateEffect::CopyToClipboard(answer.to_owned()))
            },
            CommandEffect::Interview => self.handle_interview_command("", draft),
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
            CommandEffect::ShowSecrets => self.handle_secrets_command(invocation, draft),
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

    pub(super) fn handle_prompt_command(
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

    pub(super) fn handle_secrets_command(
        &mut self,
        text: &str,
        draft: &str,
    ) -> Result<StateEffect, StateError> {
        let argument = secrets_argument(text).expect("command syntax checked");
        let delete_id = if argument.is_empty() {
            None
        } else if let Some(id) = parse_secret_delete_id(argument) {
            Some(id)
        } else {
            self.restore_draft(draft);
            self.chat.push_notice(
                "Use /secrets to list saved entries, or /secrets delete <64-character public ID>. Your draft was preserved."
                    .to_owned(),
            )?;
            return Ok(StateEffect::Redraw);
        };
        let Some(store) = self.secret_store.as_ref() else {
            self.restore_draft(draft);
            self.chat.push_notice("Local secret storage is unavailable; no entry was changed. Your draft was preserved.".to_owned())?;
            return Ok(StateEffect::Redraw);
        };
        let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => duration.as_secs(),
            Err(_) => {
                self.restore_draft(draft);
                self.chat.push_notice("Current time is unavailable; saved secrets were not changed. Your draft was preserved.".to_owned())?;
                return Ok(StateEffect::Redraw);
            },
        };
        let entries = match store.list(now) {
            Ok(entries) => entries,
            Err(_) => {
                self.restore_draft(draft);
                self.chat.push_notice("Saved secrets could not be authenticated; no entry was changed. Your draft was preserved.".to_owned())?;
                return Ok(StateEffect::Redraw);
            },
        };
        if let Some(id) = delete_id {
            let mut selected = None;
            for entry in entries {
                let Ok(public_id) = entry.public_id() else {
                    self.restore_draft(draft);
                    self.chat.push_notice("Saved secret identity is invalid; no entry was changed. Your draft was preserved.".to_owned())?;
                    return Ok(StateEffect::Redraw);
                };
                if public_id == id {
                    if selected.is_some() {
                        self.restore_draft(draft);
                        self.chat.push_notice("Saved secret ID is ambiguous; no entry was changed. Your draft was preserved.".to_owned())?;
                        return Ok(StateEffect::Redraw);
                    }
                    selected = Some(entry);
                }
            }
            let Some(entry) = selected else {
                self.restore_draft(draft);
                self.chat.push_notice(
                    "No saved secret matches that public ID. Your draft was preserved.".to_owned(),
                )?;
                return Ok(StateEffect::Redraw);
            };
            if store.delete(entry.destination(), entry.scope()).is_err() {
                self.restore_draft(draft);
                self.chat.push_notice("Saved secret deletion failed; the entry remains unavailable or unchanged. Your draft was preserved.".to_owned())?;
                return Ok(StateEffect::Redraw);
            }
            self.clear_editor();
            self.chat
                .push_notice(format!("Deleted saved secret `{id}`."))?;
            return Ok(StateEffect::Redraw);
        }
        let Some(document) = secrets_document(&entries).and_then(TuiDocument::new) else {
            self.restore_draft(draft);
            self.chat.push_notice(
                "The saved secret list exceeds the display limit. Your draft was preserved."
                    .to_owned(),
            )?;
            return Ok(StateEffect::Redraw);
        };
        self.observe_document(document.with_expanded(true))?;
        self.clear_editor();
        Ok(StateEffect::Redraw)
    }

    pub(super) fn handle_compact_command(
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

    pub(super) fn accept_model_selection(
        &mut self,
        identity: &str,
    ) -> Result<StateEffect, StateError> {
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

    pub(super) fn push_unknown_command_notice(&mut self, text: String) -> Result<(), StateError> {
        self.chat.push_notice(format!(
            "Unknown command `{text}`. Press Esc while the command palette is visible to send it to the agent."
        )).map(|_| ())
    }

    pub(in crate::runner) const fn model_switch_ready(&self) -> bool {
        self.pending_model_selection.is_some()
    }
}

#[cfg(test)]
mod tests;

fn parse_secret_delete_id(argument: &str) -> Option<&str> {
    let mut parts = argument.split_whitespace();
    let (Some("delete"), Some(id), None) = (parts.next(), parts.next(), parts.next()) else {
        return None;
    };
    (id.len() == 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then_some(id)
}

fn secrets_document(entries: &[SecretMetadata]) -> Option<ActivityDocument> {
    let mut markdown = String::from("## Saved secrets\n\n");
    if entries.is_empty() {
        markdown.push_str("No saved secrets.\n");
    } else {
        markdown.push_str("Only authenticated public metadata is shown. To delete an entry, run `/secrets delete <ID>`.\n\n");
        for entry in entries {
            let id = entry.public_id().ok()?;
            let destination = entry.destination();
            let expiry = match entry.expires_at() {
                Some(seconds) => i64::try_from(seconds)
                    .ok()
                    .and_then(|second| jiff::Timestamp::from_second(second).ok())
                    .map_or_else(
                        || format!("Unix seconds {seconds}"),
                        |time| time.to_string(),
                    ),
                None => "Until deleted".to_owned(),
            };
            markdown.push_str(&format!(
                "- **{}** · ID `{id}` · scope `{}` · {} / {} · account {} · {}\n",
                escape_public_markdown(entry.title()),
                escape_public_markdown(entry.scope()),
                escape_public_markdown(destination.provider()),
                escape_public_markdown(destination.model()),
                escape_public_markdown(destination.authenticated_account()),
                escape_public_markdown(&expiry),
            ));
        }
    }
    Some(ActivityDocument {
        title: "Saved secrets".to_owned(),
        markdown,
    })
}

fn escape_public_markdown(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(
            character,
            '\\' | '`'
                | '*'
                | '_'
                | '['
                | ']'
                | '('
                | ')'
                | '{'
                | '}'
                | '#'
                | '+'
                | '-'
                | '.'
                | '!'
                | '|'
                | '<'
                | '>'
        ) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

#[cfg(test)]
mod secrets_tests {
    use super::{escape_public_markdown, parse_secret_delete_id};

    // 삭제는 목록에 보인 정확한 공개 64자 ID만 받고 추가 토큰은 거절한다.
    #[test]
    fn secret_delete_requires_one_exact_public_id() {
        let id = "a".repeat(64);
        let command = format!("delete {id}");
        assert_eq!(parse_secret_delete_id(&command), Some(id.as_str()));
        for bad in [
            format!("delete {}", "a".repeat(63)),
            format!("delete {}", "A".repeat(64)),
            format!("delete {} extra", "a".repeat(64)),
            format!("show {}", "a".repeat(64)),
        ] {
            assert_eq!(parse_secret_delete_id(&bad), None);
        }
    }

    // 공개 metadata에도 Markdown 구문이 들어올 수 있어 목록의 행과 링크를 만들지 못하게 한다.
    #[test]
    fn secret_list_escapes_public_markdown() {
        assert_eq!(
            escape_public_markdown("`x` [link](https://example.invalid)"),
            "\\`x\\` \\[link\\]\\(https://example\\.invalid\\)"
        );
    }
}
