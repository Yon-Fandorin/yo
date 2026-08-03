//! Prompt editing assembled from semantic input, text storage, and control policy.

use std::{num::NonZeroU16, time::Duration};

pub(crate) mod binding;
pub(crate) mod layout;

use binding::NewlineBinding;
use layout::{LayoutError, TextLayout};

use super::{
    buffer::TextBuffer,
    control::{ControlEffect, ControlKeyPolicy},
    event::{InputEvent, KeyAction, KeyCode, KeyEvent, KeyModifiers},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EditorEffect {
    Unhandled,
    NoChange,
    BufferChanged,
    ExitArmed,
    InterruptTask,
    Submitted(String),
    Exit,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct PromptEditor {
    buffer: TextBuffer,
    control: ControlKeyPolicy,
    newline_binding: NewlineBinding,
}

impl PromptEditor {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn with_newline_binding(newline_binding: NewlineBinding) -> Self {
        Self {
            newline_binding,
            ..Self::default()
        }
    }

    pub(crate) fn text(&self) -> &str {
        self.buffer.as_str()
    }

    pub(crate) const fn cursor_byte_index(&self) -> usize {
        self.buffer.cursor_byte_index()
    }

    pub(crate) fn replace_range(
        &mut self,
        range: std::ops::Range<usize>,
        replacement: &str,
    ) -> bool {
        self.control.cancel_exit_sequence();
        self.buffer.replace_range(range, replacement)
    }

    pub(crate) fn layout(&self, width: NonZeroU16) -> Result<TextLayout, LayoutError> {
        layout::layout_text(self.text(), self.cursor_byte_index(), width)
    }

    pub(crate) fn handle(
        &mut self,
        event: InputEvent,
        task_active: bool,
        now: Duration,
    ) -> EditorEffect {
        match event {
            InputEvent::Key(key) => self.handle_key(key, task_active, now),
            InputEvent::Paste(text) => {
                self.control.cancel_exit_sequence();
                if self.buffer.insert(&text) {
                    EditorEffect::BufferChanged
                } else {
                    EditorEffect::NoChange
                }
            },
            InputEvent::Resize(_) => EditorEffect::Unhandled,
        }
    }

    fn handle_key(&mut self, key: KeyEvent, task_active: bool, now: Duration) -> EditorEffect {
        let control_effect = self.control.handle(key, task_active, &mut self.buffer, now);
        if control_effect != ControlEffect::Unhandled {
            return control_effect.into();
        }

        if key.action == KeyAction::Release {
            return EditorEffect::Unhandled;
        }

        if key.code == KeyCode::Enter {
            return if key.modifiers == KeyModifiers::NONE {
                self.buffer
                    .take()
                    .map_or(EditorEffect::NoChange, EditorEffect::Submitted)
            } else if self.newline_binding.matches(key.modifiers) {
                self.buffer.insert("\n");
                EditorEffect::BufferChanged
            } else {
                EditorEffect::Unhandled
            };
        }

        let changed = match key.code {
            KeyCode::Character(character) if is_plain_text(key.modifiers) => {
                self.buffer.insert(character.encode_utf8(&mut [0; 4]))
            },
            KeyCode::Left if key.modifiers == KeyModifiers::NONE => self.buffer.move_left(),
            KeyCode::Right if key.modifiers == KeyModifiers::NONE => self.buffer.move_right(),
            KeyCode::Backspace if key.modifiers == KeyModifiers::NONE => {
                self.buffer.delete_backward()
            },
            KeyCode::Delete if key.modifiers == KeyModifiers::NONE => self.buffer.delete_forward(),
            _ => return EditorEffect::Unhandled,
        };

        if changed {
            EditorEffect::BufferChanged
        } else {
            EditorEffect::NoChange
        }
    }
}

impl From<ControlEffect> for EditorEffect {
    fn from(effect: ControlEffect) -> Self {
        match effect {
            ControlEffect::Unhandled => Self::Unhandled,
            ControlEffect::NoChange => Self::NoChange,
            ControlEffect::BufferChanged => Self::BufferChanged,
            ControlEffect::ExitArmed => Self::ExitArmed,
            ControlEffect::InterruptTask => Self::InterruptTask,
            ControlEffect::Exit => Self::Exit,
        }
    }
}

fn is_plain_text(modifiers: KeyModifiers) -> bool {
    modifiers == KeyModifiers::NONE || modifiers == KeyModifiers::SHIFT
}

#[cfg(test)]
mod tests;
