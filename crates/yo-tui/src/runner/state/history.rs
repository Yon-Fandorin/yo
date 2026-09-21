//! 현재 TUI 세션에서 승인된 일반 입력의 제한된 재편집 이력.

use std::{collections::VecDeque, mem, sync::Arc, time::Duration};

use unicode_segmentation::UnicodeSegmentation;
use yo_core::{InputReference, UserInput};

use crate::{
    input::{
        editor::{EditorEffect, PromptEditor},
        event::{InputEvent, KeyAction, KeyCode, KeyModifiers},
    },
    overlay::{OverlayInstanceToken, PanelSnapshot, SelectionEntry},
    prompt::assist::PromptAssistController,
    runner::{
        state::{StateEffect, StateError, TuiState},
        view::ObservabilityView,
    },
    surface::Grapheme,
};

const ENTRY_LIMIT: usize = 32;
const BYTE_LIMIT: usize = 32 * 1024 * 1024;

#[derive(Debug, Default)]
pub(super) struct PromptHistory {
    entries: VecDeque<(Arc<UserInput>, usize)>,
    bytes: usize,
}

#[derive(Debug)]
pub(super) struct RecallPicker {
    pub(super) token: OverlayInstanceToken,
    pub(super) saved_editor: PromptEditor,
    saved_assist: PromptAssistController,
    entries: Vec<Arc<UserInput>>,
}

impl PromptHistory {
    pub(super) fn retain(&mut self, input: UserInput) {
        // PNG 원본은 UserInput이 소유한다. 전체 항목을 함께 제거해야 marker와 payload가 분리되지
        // 않는다.
        let bytes = input.as_str().len()
            + input
                .references()
                .iter()
                .map(reference_bytes)
                .sum::<usize>()
            + input
                .images()
                .iter()
                .map(|image| {
                    image.snapshot().png().len()
                        + image.snapshot().sha256().len()
                        + image.display().map_or(0, |display| {
                            display.filename.as_ref().map_or(0, String::len)
                                + display.source_mime_type.as_ref().map_or(0, String::len)
                                + display.source_sha256.as_ref().map_or(0, String::len)
                        })
                        + 1024
                })
                .sum::<usize>();
        if bytes > BYTE_LIMIT {
            return;
        }
        while self.entries.len() >= ENTRY_LIMIT || bytes > BYTE_LIMIT - self.bytes {
            let (_, removed) = self
                .entries
                .pop_front()
                .expect("a full history has an entry");
            self.bytes -= removed;
        }
        self.bytes += bytes;
        self.entries.push_back((Arc::new(input), bytes));
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn recent(&self) -> Vec<Arc<UserInput>> {
        self.entries
            .iter()
            .rev()
            .map(|(input, _)| Arc::clone(input))
            .collect()
    }
}

fn reference_bytes(reference: &InputReference) -> usize {
    let metadata = if let Some(workspace) = reference.workspace_reference() {
        workspace.identity().len()
            + workspace.execution_environment_identity().len()
            + workspace.workspace_identity().len()
            + workspace.root_identity().len()
            + workspace.relative_path().len()
    } else if let Some(skill) = reference.skill_reference() {
        skill.identity().len()
            + skill.execution_environment_identity().len()
            + skill.locator().len()
            + skill.name().len()
            + skill.entry_revision().len()
    } else {
        0
    };
    reference.span().len() + metadata + 1024
}

impl RecallPicker {
    pub(super) fn panel(inputs: &[Arc<UserInput>], query: &str) -> PanelSnapshot {
        let needle = query.to_lowercase();
        let mut entries = inputs
            .iter()
            .enumerate()
            .filter(|(_, input)| input.as_str().to_lowercase().contains(&needle))
            .map(|(index, input)| {
                let label = input
                    .as_str()
                    .graphemes(true)
                    .map(|cluster| {
                        if Grapheme::try_from(cluster).is_ok() {
                            cluster
                        } else {
                            " "
                        }
                    })
                    .collect::<String>()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .graphemes(true)
                    .take(96)
                    .collect::<String>();
                let label = if label.is_empty() {
                    "(empty prompt)".to_owned()
                } else {
                    label
                };
                let detail =
                    (!input.references().is_empty() || !input.images().is_empty()).then(|| {
                        format!(
                            "{} reference(s), {} image(s)",
                            input.references().len(),
                            input.images().len()
                        )
                    });
                SelectionEntry::enabled_with_context(index.to_string(), label, None, detail)
            })
            .collect::<Vec<_>>();
        if entries.is_empty() {
            entries.push(SelectionEntry::status("empty", "No matching prompts"));
        }
        PanelSnapshot::new("Recall prompt · type to filter", entries)
            .expect("bounded ordinary prompt labels are valid")
    }
}

impl TuiState {
    pub(super) fn open_recall_picker(&mut self) -> Result<StateEffect, StateError> {
        if self.prompt_history.is_empty() {
            self.chat
                .push_notice("No prompts to recall in this session.".to_owned())?;
            return Ok(StateEffect::Redraw);
        }
        let saved_editor = self.editor.clone();
        self.prompt_assist.cancel();
        let saved_assist = mem::take(&mut self.prompt_assist);
        let entries = self.prompt_history.recent();
        self.editor.replace_range(0..self.editor.text().len(), "");
        let token = match self.open_overlay(RecallPicker::panel(&entries, "")) {
            Ok(token) => token,
            Err(error) => {
                self.editor = saved_editor;
                self.prompt_assist = saved_assist;
                return Err(StateError::RequestPanel(error));
            },
        };
        self.recall_picker = Some(RecallPicker {
            token,
            saved_editor,
            saved_assist,
            entries,
        });
        Ok(StateEffect::Redraw)
    }

    pub(super) fn cancel_recall_picker(&mut self) {
        if let Some(picker) = self.recall_picker.take() {
            if self.overlay.is_current(picker.token) {
                self.overlay.close_current();
            }
            self.editor = picker.saved_editor;
            self.prompt_assist = picker.saved_assist;
        }
    }

    pub(super) fn accept_recalled_prompt(&mut self, index: usize) -> StateEffect {
        let Some(input) = self
            .recall_picker
            .as_ref()
            .and_then(|picker| picker.entries.get(index))
            .map(Arc::clone)
        else {
            self.cancel_recall_picker();
            return StateEffect::Redraw;
        };
        let picker = self.recall_picker.take().expect("active picker");
        self.prompt_assist = picker.saved_assist;
        self.editor
            .replace_range(0..self.editor.text().len(), input.as_str());
        self.prompt_assist.restore_input(&input, &mut self.overlay);
        self.command_palette
            .preserve_literal(input.as_str(), &mut self.overlay);
        StateEffect::Redraw
    }

    pub(super) fn handle_recall_query(
        &mut self,
        input: &InputEvent,
        now: Duration,
    ) -> Result<Option<StateEffect>, StateError> {
        if self.recall_picker.is_none() {
            return Ok(None);
        }
        if matches!(input, InputEvent::Key(key) if key.code == KeyCode::Escape
            && key.modifiers == KeyModifiers::NONE && key.action == KeyAction::Press)
        {
            self.cancel_recall_picker();
            return Ok(Some(StateEffect::Redraw));
        }
        if matches!(input, InputEvent::Key(key)
            if matches!(key.code, KeyCode::Character('c' | 'C' | 'd' | 'D'))
                && key.modifiers == KeyModifiers::CONTROL
                && key.action == KeyAction::Press)
        {
            self.cancel_recall_picker();
            return Ok(None);
        }
        let edit = match input {
            InputEvent::Paste(text) => Some(InputEvent::Paste(
                text.chars()
                    .map(|character| {
                        if character.is_control() {
                            ' '
                        } else {
                            character
                        }
                    })
                    .take(128)
                    .collect(),
            )),
            InputEvent::Key(key)
                if matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT)
                    && matches!(
                        key.code,
                        KeyCode::Character(_)
                            | KeyCode::Backspace
                            | KeyCode::Delete
                            | KeyCode::Left
                            | KeyCode::Right
                    ) =>
            {
                Some(input.clone())
            },
            _ => None,
        };
        let inserting = matches!(&edit, Some(InputEvent::Paste(_)))
            || matches!(&edit, Some(InputEvent::Key(key))
                if matches!(key.code, KeyCode::Character(_)));
        if let Some(edit) = edit
            && (!inserting || self.editor.text().len() < 512)
            && matches!(
                self.editor.handle(edit, false, now),
                EditorEffect::BufferChanged
            )
        {
            let token = self.recall_picker.as_ref().expect("active picker").token;
            let panel = RecallPicker::panel(
                &self.recall_picker.as_ref().expect("active picker").entries,
                self.editor.text(),
            );
            self.overlay
                .refresh(token, panel)
                .map_err(StateError::RequestPanel)?;
            return Ok(Some(StateEffect::Redraw));
        }
        Ok(Some(StateEffect::Unchanged))
    }

    pub(super) fn can_open_recall(&self) -> bool {
        self.views.active() == ObservabilityView::Chat
            && !self.has_pending_request()
            && self.recall_picker.is_none()
            && self.fork_overlay.is_none()
            && self.resume_overlay.is_none()
            && self.model_overlay.is_none()
            && self.pending_image.is_none()
            && self.pending_submissions.is_empty()
            && self.starting_submission.is_none()
    }
}

#[cfg(test)]
mod tests;
