//! Prompt editing assembled from semantic input, text storage, and control policy.

use std::{cmp::Reverse, collections::VecDeque, num::NonZeroU16, ops::Range, time::Duration};

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

const MAX_UNDO_STATES: usize = 64;
const MAX_UNDO_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UndoAction {
    Type(char),
    Atomic,
    Break,
    Ignore,
    Undo,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct UndoHistory {
    states: VecDeque<TextBuffer>,
    retained_bytes: usize,
    typing: bool,
}

impl UndoHistory {
    fn clear(&mut self) {
        self.states.clear();
        self.retained_bytes = 0;
        self.typing = false;
    }

    fn push(&mut self, before: TextBuffer) {
        let bytes = before.as_str().len();
        while self.states.len() >= MAX_UNDO_STATES || self.retained_bytes + bytes > MAX_UNDO_BYTES {
            let old = self.states.pop_front().expect("undo state to evict");
            self.retained_bytes -= old.as_str().len();
        }
        self.retained_bytes += bytes;
        self.states.push_back(before);
    }

    fn pop(&mut self) -> Option<TextBuffer> {
        self.typing = false;
        let state = self.states.pop_back()?;
        self.retained_bytes -= state.as_str().len();
        Some(state)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct PromptEditor {
    buffer: TextBuffer,
    control: ControlKeyPolicy,
    newline_binding: NewlineBinding,
    undo: UndoHistory,
    killed_text: String,
    last_kill: bool,
    layout_width: Option<NonZeroU16>,
    preferred_column: Option<u16>,
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

    pub(crate) const fn newline_binding(&self) -> NewlineBinding {
        self.newline_binding
    }

    pub(crate) const fn has_layout_width(&self) -> bool {
        self.layout_width.is_some()
    }

    pub(crate) fn set_layout_width(&mut self, width: NonZeroU16) {
        if self.layout_width != Some(width) {
            self.layout_width = Some(width);
            self.preferred_column = None;
        }
    }

    pub(crate) fn replace_range(&mut self, range: Range<usize>, replacement: &str) -> bool {
        self.control.cancel_exit_sequence();
        self.last_kill = false;
        self.preferred_column = None;
        self.undo.clear();
        self.buffer.replace_range(range, replacement)
    }

    pub(crate) fn replace_range_undoable(
        &mut self,
        range: Range<usize>,
        replacement: &str,
    ) -> bool {
        self.control.cancel_exit_sequence();
        self.last_kill = false;
        self.preferred_column = None;
        self.undo.typing = false;
        let oversized_before = self.buffer.as_str().len() > MAX_UNDO_BYTES;
        let before = self.undo_snapshot();
        let changed = self.buffer.replace_range(range, replacement);
        if changed {
            if oversized_before {
                self.undo.clear();
            } else if let Some(before) = before {
                self.undo.push(before);
            }
        }
        changed
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
        let action = self.undo_action(&event, task_active);
        if action == UndoAction::Undo {
            self.control.cancel_exit_sequence();
            self.last_kill = false;
            self.preferred_column = None;
            return self.undo.pop().map_or(EditorEffect::NoChange, |state| {
                self.buffer = state;
                EditorEffect::BufferChanged
            });
        }
        let oversized_before = self.buffer.as_str().len() > MAX_UNDO_BYTES;
        let before = match action {
            UndoAction::Type(character) if !character.is_whitespace() && self.undo.typing => None,
            UndoAction::Type(_) | UndoAction::Atomic => self.undo_snapshot(),
            UndoAction::Break | UndoAction::Ignore | UndoAction::Undo => None,
        };
        if action == UndoAction::Break {
            self.undo.typing = false;
        }
        if !matches!(&event, InputEvent::Key(key)
            if key.action == KeyAction::Release
                || (matches!(key.code, KeyCode::Up | KeyCode::Down)
                    && key.modifiers == KeyModifiers::NONE))
        {
            self.preferred_column = None;
        }
        if !matches!(&event, InputEvent::Key(key) if key.action == KeyAction::Release
            || (key.modifiers == KeyModifiers::CONTROL && matches!(key.code, KeyCode::Character('u' | 'U' | 'k' | 'K' | 'w' | 'W'))))
        {
            self.last_kill = false;
        }
        let effect = match event {
            InputEvent::Key(key) => self.handle_key(key, task_active, now),
            InputEvent::Paste(text) => {
                self.control.cancel_exit_sequence();
                if self.buffer.insert(&text) {
                    EditorEffect::BufferChanged
                } else {
                    EditorEffect::NoChange
                }
            },
            InputEvent::Resize(_) | InputEvent::MouseScroll(_) => EditorEffect::Unhandled,
        };
        match effect {
            EditorEffect::Submitted(_) | EditorEffect::Exit => self.undo.clear(),
            EditorEffect::BufferChanged
                if matches!(action, UndoAction::Type(_) | UndoAction::Atomic) =>
            {
                if oversized_before {
                    self.undo.clear();
                } else if let Some(before) = before {
                    self.undo.push(before);
                }
                self.undo.typing =
                    matches!(action, UndoAction::Type(_)) && !self.undo.states.is_empty();
            },
            _ if matches!(action, UndoAction::Type(_) | UndoAction::Atomic) => {
                self.undo.typing = false;
            },
            _ => {},
        }
        effect
    }

    fn undo_snapshot(&self) -> Option<TextBuffer> {
        if self.buffer.as_str().len() > MAX_UNDO_BYTES {
            None
        } else {
            Some(self.buffer.clone())
        }
    }

    fn undo_action(&self, event: &InputEvent, task_active: bool) -> UndoAction {
        match event {
            InputEvent::Paste(text) if !text.is_empty() => UndoAction::Atomic,
            InputEvent::Paste(_) => UndoAction::Ignore,
            InputEvent::Resize(_) | InputEvent::MouseScroll(_) => UndoAction::Break,
            InputEvent::Key(key) if key.action == KeyAction::Release => UndoAction::Ignore,
            InputEvent::Key(key) if is_undo_key(*key) => {
                if key.action == KeyAction::Press {
                    UndoAction::Undo
                } else {
                    UndoAction::Ignore
                }
            },
            InputEvent::Key(key) => match key.code {
                KeyCode::Character(character) if is_plain_text(key.modifiers) => {
                    UndoAction::Type(character)
                },
                KeyCode::Character('c' | 'C')
                    if key.modifiers == KeyModifiers::CONTROL && !task_active =>
                {
                    UndoAction::Atomic
                },
                KeyCode::Character('d' | 'D' | 'u' | 'U' | 'k' | 'K' | 'w' | 'W' | 'y' | 'Y')
                    if key.modifiers == KeyModifiers::CONTROL =>
                {
                    UndoAction::Atomic
                },
                KeyCode::Backspace | KeyCode::Delete if key.modifiers == KeyModifiers::NONE => {
                    UndoAction::Atomic
                },
                KeyCode::Enter if self.newline_binding.matches(key.modifiers) => UndoAction::Atomic,
                _ => UndoAction::Break,
            },
        }
    }

    fn kill_line(&mut self, backward: bool) -> EditorEffect {
        let removed = if backward {
            self.buffer.kill_line_start()
        } else {
            self.buffer.kill_line_end()
        };
        self.retain_kill(removed, backward)
    }

    fn retain_kill(&mut self, removed: Option<String>, backward: bool) -> EditorEffect {
        let Some(removed) = removed else {
            return EditorEffect::NoChange;
        };
        if !self.last_kill {
            self.killed_text = removed;
        } else if backward {
            self.killed_text.insert_str(0, &removed);
        } else {
            self.killed_text.push_str(&removed);
        }
        self.last_kill = true;
        EditorEffect::BufferChanged
    }

    fn handle_key(&mut self, key: KeyEvent, task_active: bool, now: Duration) -> EditorEffect {
        let control_effect = self.control.handle(key, task_active, &mut self.buffer, now);
        if control_effect != ControlEffect::Unhandled {
            return control_effect.into();
        }

        if key.action == KeyAction::Release {
            return EditorEffect::Unhandled;
        }

        if key.modifiers == KeyModifiers::CONTROL {
            match key.code {
                KeyCode::Character('u' | 'U') => return self.kill_line(true),
                KeyCode::Character('k' | 'K') => return self.kill_line(false),
                KeyCode::Character('w' | 'W') => {
                    let removed = self.buffer.kill_word_start();
                    return self.retain_kill(removed, true);
                },
                _ => {},
            }
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
            KeyCode::Character('a' | 'A') if key.modifiers == KeyModifiers::CONTROL => {
                self.buffer.move_line_start()
            },
            KeyCode::Character('e' | 'E') if key.modifiers == KeyModifiers::CONTROL => {
                self.buffer.move_line_end()
            },
            KeyCode::Character('y' | 'Y') if key.modifiers == KeyModifiers::CONTROL => {
                self.buffer.insert(&self.killed_text)
            },
            KeyCode::Character(character) if is_plain_text(key.modifiers) => {
                self.buffer.insert(character.encode_utf8(&mut [0; 4]))
            },
            KeyCode::Left if key.modifiers == KeyModifiers::NONE => self.buffer.move_left(),
            KeyCode::Right if key.modifiers == KeyModifiers::NONE => self.buffer.move_right(),
            KeyCode::Up | KeyCode::Down if key.modifiers == KeyModifiers::NONE => {
                let Some(width) = self.layout_width else {
                    return EditorEffect::Unhandled;
                };
                self.move_vertical(key.code == KeyCode::Up, width)
            },
            KeyCode::Left if key.modifiers == KeyModifiers::CONTROL => {
                self.buffer.move_word_start()
            },
            KeyCode::Right if key.modifiers == KeyModifiers::CONTROL => self.buffer.move_word_end(),
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

    fn move_vertical(&mut self, up: bool, width: NonZeroU16) -> bool {
        let Ok(stops) = layout::cursor_stops(self.buffer.as_str(), width) else {
            return false;
        };
        let Some((_, current)) = stops
            .iter()
            .find(|(byte, _)| *byte == self.buffer.cursor_byte_index())
        else {
            return false;
        };
        let target_row = if up {
            stops
                .iter()
                .filter_map(|(_, point)| (point.y < current.y).then_some(point.y))
                .max()
        } else {
            stops
                .iter()
                .filter_map(|(_, point)| (point.y > current.y).then_some(point.y))
                .min()
        };
        let Some(target_row) = target_row else {
            return false;
        };
        let preferred = self.preferred_column.unwrap_or(current.x);
        let target = stops
            .iter()
            .filter(|(_, point)| point.y == target_row)
            .min_by_key(|(byte, point)| {
                (
                    point.x.abs_diff(preferred),
                    point.x > preferred,
                    Reverse(*byte),
                )
            })
            .map(|(byte, _)| *byte);
        let Some(target) = target else {
            return false;
        };
        self.preferred_column = Some(preferred);
        self.buffer.move_to_grapheme_boundary(target)
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

fn is_undo_key(key: KeyEvent) -> bool {
    (key.modifiers == KeyModifiers::CONTROL
        && matches!(key.code, KeyCode::Character('7' | '-' | '_')))
        || (key.modifiers == KeyModifiers::CONTROL.union(KeyModifiers::SHIFT)
            && key.code == KeyCode::Character('_'))
}

#[cfg(test)]
mod tests;
