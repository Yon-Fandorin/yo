//! Chat-only finalized-message search and its draft-preserving picker.

use std::{iter::repeat_n, mem, time::Duration};

use unicode_segmentation::UnicodeSegmentation;
use yo_core::MessageContent;

use super::{StateEffect, StateError, TuiState};
use crate::{
    command::find_argument,
    input::{
        editor::{EditorEffect, PromptEditor},
        event::{InputEvent, KeyAction, KeyCode, KeyModifiers},
    },
    overlay::{OverlayInstanceToken, PanelSnapshot, SelectionEntry},
    prompt::assist::PromptAssistController,
    runner::view::ObservabilityView,
    surface::Grapheme,
    transcript::{
        MessageRole, TranscriptBody, TranscriptItem, TranscriptItemId, TranscriptPhase,
        TranscriptState,
    },
};

const CORPUS_ENTRY_LIMIT: usize = 1024;
const CORPUS_BYTE_LIMIT: usize = 2 * 1024 * 1024;
const MESSAGE_BYTE_LIMIT: usize = 256 * 1024;
const CORPUS_SCAN_LIMIT: usize = 4096;
const RESULT_LIMIT: usize = 100;
const QUERY_BYTE_LIMIT: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
struct FindEntry {
    item: TranscriptItemId,
    role: MessageRole,
    text: String,
}

#[derive(Debug)]
struct FindCorpus {
    entries: Vec<FindEntry>,
    truncated: bool,
}

#[derive(Debug)]
pub(super) struct FindPicker {
    pub(super) token: OverlayInstanceToken,
    pub(super) saved_editor: PromptEditor,
    saved_assist: PromptAssistController,
    entries: Vec<FindEntry>,
    corpus_truncated: bool,
}

impl FindPicker {
    fn panel(entries: &[FindEntry], corpus_truncated: bool, query: &str) -> PanelSnapshot {
        let mut choices = Vec::new();
        if query.is_empty() {
            choices.push(SelectionEntry::status(
                "find-instructions",
                if corpus_truncated {
                    "Type a word or phrase to search finalized Chat messages · corpus capped"
                } else {
                    "Type a word or phrase to search finalized Chat messages"
                },
            ));
        } else {
            let needle = query.to_lowercase();
            let matches = entries
                .iter()
                .filter(|entry| entry.text.to_lowercase().contains(&needle))
                .collect::<Vec<_>>();
            choices.extend(matches.iter().take(RESULT_LIMIT).map(|entry| {
                SelectionEntry::enabled_with_context(
                    entry.item.get().to_string(),
                    preview(&entry.text, query),
                    Some(role_label(entry.role).to_owned()),
                    None,
                )
            }));
            if matches.is_empty() {
                let coverage = if corpus_truncated {
                    " · search corpus capped"
                } else {
                    ""
                };
                choices.push(SelectionEntry::status(
                    "find-empty",
                    format!("No matching messages{coverage}"),
                ));
            } else {
                let coverage = if corpus_truncated {
                    " · search corpus capped"
                } else {
                    ""
                };
                choices.push(SelectionEntry::status(
                    "find-status",
                    format!(
                        "Showing {} of {} matches{}",
                        matches.len().min(RESULT_LIMIT),
                        matches.len(),
                        coverage
                    ),
                ));
            }
        }
        PanelSnapshot::new("Find messages · type to filter", choices)
            .expect("bounded find labels form a valid panel")
    }
}

fn role_label(role: MessageRole) -> &'static str {
    match role {
        MessageRole::User => "User",
        MessageRole::Assistant => "Assistant",
    }
}

fn preview(text: &str, query: &str) -> String {
    let start = match_start(text, query).unwrap_or(0);
    let graphemes = text.graphemes(true).collect::<Vec<_>>();
    let start_grapheme = text[..start].graphemes(true).count();
    let from = start_grapheme.saturating_sub(24);
    let to = (from + 96).min(graphemes.len());
    let compact = graphemes[from..to]
        .iter()
        .copied()
        .collect::<String>()
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
        .join(" ");
    let compact = compact.graphemes(true).take(96).collect::<String>();
    if compact.is_empty() {
        "(empty message)".to_owned()
    } else if from > 0 {
        format!("…{compact}")
    } else if to < graphemes.len() {
        format!("{compact}…")
    } else {
        compact
    }
}

fn match_start(text: &str, query: &str) -> Option<usize> {
    let needle = query.to_lowercase();
    let mut folded = String::new();
    let mut source_offsets = Vec::new();
    for (index, character) in text.char_indices() {
        let lower = character.to_lowercase().collect::<String>();
        folded.push_str(&lower);
        source_offsets.extend(repeat_n(index, lower.len()));
    }
    folded
        .find(&needle)
        .and_then(|index| source_offsets.get(index).copied())
}

fn corpus(transcript: &TranscriptState) -> FindCorpus {
    let mut entries = Vec::with_capacity(CORPUS_ENTRY_LIMIT);
    let mut bytes = 0;
    let mut truncated = false;
    for (examined, item) in transcript.items().iter().rev().enumerate() {
        if examined >= CORPUS_SCAN_LIMIT {
            truncated = true;
            break;
        }
        if entries.len() >= CORPUS_ENTRY_LIMIT {
            truncated = true;
            break;
        }
        if item.phase() != TranscriptPhase::Final {
            continue;
        }
        let TranscriptBody::Message(message) = item.body();
        if !is_searchable(item) {
            continue;
        }
        if message.text().len() > MESSAGE_BYTE_LIMIT {
            truncated = true;
            continue;
        }
        if message.text().len() > CORPUS_BYTE_LIMIT.saturating_sub(bytes) {
            truncated = true;
            continue;
        }
        bytes += message.text().len();
        entries.push(FindEntry {
            item: item.id(),
            role: message.role(),
            text: message.text().to_owned(),
        });
    }
    FindCorpus { entries, truncated }
}

fn is_searchable(item: &TranscriptItem) -> bool {
    if item.phase() != TranscriptPhase::Final {
        return false;
    }
    let TranscriptBody::Message(message) = item.body();
    match message.role() {
        MessageRole::User => true,
        // Chat's ordinary assistant answers are the only non-activity assistant items
        // rendered as Markdown. Notices use the plain assistant projection.
        MessageRole::Assistant => {
            message.is_markdown() && MessageContent::from_snapshot(message.text()).is_none()
        },
    }
}

fn bounded_query(text: &str) -> String {
    let mut query = text.to_owned();
    truncate_utf8(&mut query, QUERY_BYTE_LIMIT);
    query
}

fn truncate_utf8(text: &mut String, limit: usize) {
    if text.len() <= limit {
        return;
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
}

fn sanitized_paste(text: &str) -> String {
    let mut paste = String::new();
    for character in text.chars() {
        let character = if character.is_control() {
            ' '
        } else {
            character
        };
        if character.len_utf8() > QUERY_BYTE_LIMIT - paste.len() {
            break;
        }
        paste.push(character);
    }
    paste
}

impl TuiState {
    pub(super) fn handle_find_command(
        &mut self,
        text: &str,
        draft: &str,
    ) -> Result<StateEffect, StateError> {
        let query = find_argument(text).expect("find command syntax checked");
        if query.len() > QUERY_BYTE_LIMIT {
            self.restore_draft(draft);
            self.chat.push_notice(
                "Find queries are limited to 256 UTF-8 bytes; your draft was preserved.".to_owned(),
            )?;
            return Ok(StateEffect::Redraw);
        }
        if !self.can_open_find() {
            self.restore_draft(draft);
            self.chat.push_notice(
                "Finish the pending request or turn before searching Chat messages. Your draft was preserved."
                    .to_owned(),
            )?;
            return Ok(StateEffect::Redraw);
        }
        self.clear_editor();
        self.open_find_picker(query)
    }

    pub(super) fn open_find_picker(
        &mut self,
        initial_query: &str,
    ) -> Result<StateEffect, StateError> {
        let initial_query = bounded_query(initial_query);
        let saved_editor = self.editor.clone();
        self.prompt_assist.cancel();
        let saved_assist = mem::take(&mut self.prompt_assist);
        let corpus = corpus(self.chat.transcript());
        let entries = corpus.entries;
        let corpus_truncated = corpus.truncated;
        self.editor.replace_range(0..self.editor.text().len(), "");
        self.editor.replace_range(0..0, &initial_query);
        let token = match self.open_overlay(FindPicker::panel(
            &entries,
            corpus_truncated,
            &initial_query,
        )) {
            Ok(token) => token,
            Err(error) => {
                self.editor = saved_editor;
                self.prompt_assist = saved_assist;
                return Err(StateError::RequestPanel(error));
            },
        };
        self.find_picker = Some(FindPicker {
            token,
            saved_editor,
            saved_assist,
            entries,
            corpus_truncated,
        });
        Ok(StateEffect::Redraw)
    }

    pub(super) fn cancel_find_picker(&mut self) {
        if let Some(picker) = self.find_picker.take() {
            if self.overlay.is_current(picker.token) {
                self.overlay.close_current();
            }
            self.editor = picker.saved_editor;
            self.prompt_assist = picker.saved_assist;
        }
    }

    pub(super) fn accept_find_result(&mut self, identity: &str) -> StateEffect {
        let Some(picker) = self.find_picker.take() else {
            return StateEffect::Redraw;
        };
        let Some(entry) = picker
            .entries
            .iter()
            .find(|entry| entry.item.get().to_string() == identity)
        else {
            self.editor = picker.saved_editor;
            self.prompt_assist = picker.saved_assist;
            return StateEffect::Redraw;
        };
        let item = entry.item;
        let item_exists = self
            .chat
            .transcript()
            .items()
            .iter()
            .any(|candidate| candidate.id() == item && is_searchable(candidate));
        self.editor = picker.saved_editor;
        self.prompt_assist = picker.saved_assist;
        if !item_exists {
            return StateEffect::Redraw;
        }
        self.views.jump_chat_to(item);
        StateEffect::Redraw
    }

    pub(super) fn handle_find_query(
        &mut self,
        input: &InputEvent,
        now: Duration,
    ) -> Result<Option<StateEffect>, StateError> {
        if self.find_picker.is_none() {
            return Ok(None);
        }
        if matches!(input, InputEvent::Key(key) if key.code == KeyCode::Escape
            && key.modifiers == KeyModifiers::NONE && key.action == KeyAction::Press)
        {
            self.cancel_find_picker();
            return Ok(Some(StateEffect::Redraw));
        }
        if matches!(input, InputEvent::Key(key)
            if matches!(key.code, KeyCode::Character('c' | 'C'))
                && key.modifiers == KeyModifiers::CONTROL
                && key.action == KeyAction::Press)
        {
            self.cancel_find_picker();
            return Ok(Some(StateEffect::Redraw));
        }
        let edit = match input {
            InputEvent::Paste(text) => Some(InputEvent::Paste(sanitized_paste(text))),
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
        if let Some(edit) = edit {
            let before = self.editor.clone();
            let changed = matches!(
                self.editor.handle(edit, false, now),
                EditorEffect::BufferChanged
            );
            if changed && self.editor.text().len() <= QUERY_BYTE_LIMIT {
                let picker = self.find_picker.as_ref().expect("active find picker");
                self.overlay
                    .refresh(
                        picker.token,
                        FindPicker::panel(
                            &picker.entries,
                            picker.corpus_truncated,
                            self.editor.text(),
                        ),
                    )
                    .map_err(StateError::RequestPanel)?;
                return Ok(Some(StateEffect::Redraw));
            }
            if self.editor.text().len() > QUERY_BYTE_LIMIT {
                self.editor = before;
                return Ok(Some(StateEffect::Redraw));
            }
            return Ok(Some(StateEffect::Unchanged));
        }
        // An overlay that has not been committed to a visible frame returns
        // Unhandled for Enter. Never let that key submit the saved Chat draft.
        Ok(Some(StateEffect::Unchanged))
    }

    pub(super) fn can_open_find(&self) -> bool {
        self.views.active() == ObservabilityView::Chat
            && !self.has_pending_request()
            && self.find_picker.is_none()
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
