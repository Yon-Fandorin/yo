//! Request-bound secret input with no ordinary prompt editing state.

use std::{fmt, num::NonZeroU16};

use yo_core::SecretInput;

use super::{
    buffer::TextBuffer,
    editor::{
        PromptEditor,
        binding::NewlineBinding,
        layout::{TextLayout, layout_text},
    },
    event::{InputEvent, KeyAction, KeyCode, KeyModifiers},
};

/// The only state that a secret editor exposes to the terminal renderer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SecretPublicState {
    NotEntered,
    Entered,
    ReentryRequired,
}

impl SecretPublicState {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::NotEntered => "Not entered",
            Self::Entered => "Entered",
            Self::ReentryRequired => "Re-entry required",
        }
    }
}

/// The result of handling one terminal event in the secret editor.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum SecretEditorEffect {
    Unhandled,
    NoChange,
    Changed,
    Submitted(SecretInput),
    Cancel,
    Exit,
    Rejected,
}

/// A separate editor for one live secret request.
///
/// Its buffer is never exposed through the prompt renderer or ordinary prompt
/// assist paths. The public projection is deliberately a fixed state label.
pub(crate) struct SecretEditor {
    buffer: TextBuffer,
    public_state: SecretPublicState,
    ready: bool,
}

impl fmt::Debug for SecretEditor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretEditor")
            .field("public_state", &self.public_state)
            .field("ready", &self.ready)
            .finish_non_exhaustive()
    }
}

impl SecretEditor {
    pub(crate) fn new() -> Self {
        Self {
            buffer: TextBuffer::new(),
            public_state: SecretPublicState::NotEntered,
            ready: false,
        }
    }

    pub(crate) fn reentry_required() -> Self {
        Self {
            buffer: TextBuffer::new(),
            public_state: SecretPublicState::ReentryRequired,
            ready: false,
        }
    }

    pub(crate) const fn public_state(&self) -> SecretPublicState {
        self.public_state
    }

    pub(crate) const fn ready(&self) -> bool {
        self.ready
    }

    pub(crate) fn mark_ready(&mut self) {
        self.ready = true;
    }

    pub(crate) fn clear(&mut self) {
        self.buffer.clear();
        self.public_state = SecretPublicState::NotEntered;
        self.ready = false;
    }

    /// Returns a fixed public label used by prompt geometry and painting.
    pub(crate) fn public_text(&self) -> &'static str {
        self.public_state.label()
    }

    /// Produces a layout from the public label only; the secret never reaches it.
    pub(crate) fn public_layout(
        &self,
        width: NonZeroU16,
    ) -> Result<TextLayout, super::editor::layout::LayoutError> {
        layout_text(self.public_text(), self.public_text().len(), width)
    }

    pub(crate) fn submit(&mut self) -> Option<SecretInput> {
        if !self.ready || self.buffer.is_empty() {
            return None;
        }
        let value = self.buffer.take()?;
        self.public_state = SecretPublicState::NotEntered;
        self.ready = false;
        SecretInput::new(value).ok()
    }

    pub(crate) fn handle(&mut self, event: InputEvent) -> SecretEditorEffect {
        if !self.ready {
            return SecretEditorEffect::NoChange;
        }
        match event {
            InputEvent::Paste(text) => self.insert(&text),
            InputEvent::Key(key) => self.handle_key(key),
            InputEvent::Resize(_) | InputEvent::MouseScroll(_) => SecretEditorEffect::Unhandled,
        }
    }

    fn insert(&mut self, text: &str) -> SecretEditorEffect {
        if text.is_empty() {
            return SecretEditorEffect::NoChange;
        }
        if self
            .buffer
            .as_str()
            .len()
            .checked_add(text.len())
            .is_none_or(|length| length > SecretInput::MAX_BYTES)
        {
            return SecretEditorEffect::Rejected;
        }
        if self.buffer.insert(text) {
            self.public_state = SecretPublicState::Entered;
            SecretEditorEffect::Changed
        } else {
            SecretEditorEffect::NoChange
        }
    }

    fn handle_key(&mut self, key: super::event::KeyEvent) -> SecretEditorEffect {
        if key.action == KeyAction::Release {
            return SecretEditorEffect::Unhandled;
        }
        if key.code == KeyCode::Escape && key.modifiers == KeyModifiers::NONE {
            return SecretEditorEffect::Cancel;
        }
        if key.modifiers == KeyModifiers::CONTROL {
            match key.code {
                KeyCode::Character('u' | 'U') => {
                    let removed = self.buffer.kill_line_start();
                    return if removed.is_some() {
                        self.public_state = if self.buffer.is_empty() {
                            SecretPublicState::NotEntered
                        } else {
                            SecretPublicState::Entered
                        };
                        SecretEditorEffect::Changed
                    } else {
                        SecretEditorEffect::NoChange
                    };
                },
                KeyCode::Character('a' | 'A') => {
                    return if self.buffer.move_line_start() {
                        SecretEditorEffect::Changed
                    } else {
                        SecretEditorEffect::NoChange
                    };
                },
                KeyCode::Character('e' | 'E') => {
                    return if self.buffer.move_line_end() {
                        SecretEditorEffect::Changed
                    } else {
                        SecretEditorEffect::NoChange
                    };
                },
                KeyCode::Character('c' | 'C') => return SecretEditorEffect::Cancel,
                KeyCode::Character('d' | 'D') if self.buffer.is_empty() => {
                    return SecretEditorEffect::Exit;
                },
                _ => {},
            }
        }
        if key.code == KeyCode::Enter && key.modifiers == KeyModifiers::NONE {
            return if key.action == KeyAction::Press {
                self.submit()
                    .map_or(SecretEditorEffect::NoChange, SecretEditorEffect::Submitted)
            } else {
                SecretEditorEffect::Unhandled
            };
        }

        let changed = match key.code {
            KeyCode::Character(character)
                if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
            {
                let mut encoded = [0; 4];
                let text = character.encode_utf8(&mut encoded);
                if self.buffer.as_str().len() + text.len() > SecretInput::MAX_BYTES {
                    return SecretEditorEffect::Rejected;
                }
                self.buffer.insert(text)
            },
            KeyCode::Left if key.modifiers == KeyModifiers::NONE => self.buffer.move_left(),
            KeyCode::Right if key.modifiers == KeyModifiers::NONE => self.buffer.move_right(),
            KeyCode::Home if key.modifiers == KeyModifiers::NONE => self.buffer.move_line_start(),
            KeyCode::End if key.modifiers == KeyModifiers::NONE => self.buffer.move_line_end(),
            KeyCode::Backspace if key.modifiers == KeyModifiers::NONE => {
                self.buffer.delete_backward()
            },
            KeyCode::Delete if key.modifiers == KeyModifiers::NONE => self.buffer.delete_forward(),
            _ => return SecretEditorEffect::Unhandled,
        };
        if changed {
            self.public_state = if self.buffer.is_empty() {
                SecretPublicState::NotEntered
            } else {
                SecretPublicState::Entered
            };
            SecretEditorEffect::Changed
        } else {
            SecretEditorEffect::NoChange
        }
    }
}

/// Prompt rendering input. Implementations expose only the public projection.
pub(crate) trait PromptInput {
    fn public_text(&self) -> &str;
    fn layout(&self, width: NonZeroU16) -> Result<TextLayout, super::editor::layout::LayoutError>;
    fn newline_binding(&self) -> NewlineBinding;
}

impl PromptInput for PromptEditor {
    fn public_text(&self) -> &str {
        self.text()
    }

    fn layout(&self, width: NonZeroU16) -> Result<TextLayout, super::editor::layout::LayoutError> {
        self.layout(width)
    }

    fn newline_binding(&self) -> NewlineBinding {
        self.newline_binding()
    }
}

impl PromptInput for SecretEditor {
    fn public_text(&self) -> &str {
        self.public_text()
    }

    fn layout(&self, width: NonZeroU16) -> Result<TextLayout, super::editor::layout::LayoutError> {
        self.public_layout(width)
    }

    fn newline_binding(&self) -> NewlineBinding {
        NewlineBinding::default()
    }
}

/// Runtime selection of an ordinary or secret public prompt projection.
pub(crate) enum PromptInputView<'a> {
    Ordinary(&'a PromptEditor),
    Secret(&'a SecretEditor),
    Waiting(&'static str),
}

impl PromptInput for PromptInputView<'_> {
    fn public_text(&self) -> &str {
        match self {
            Self::Ordinary(editor) => editor.text(),
            Self::Secret(editor) => editor.public_text(),
            Self::Waiting(text) => text,
        }
    }

    fn layout(&self, width: NonZeroU16) -> Result<TextLayout, super::editor::layout::LayoutError> {
        match self {
            Self::Ordinary(editor) => editor.layout(width),
            Self::Secret(editor) => editor.layout(width),
            Self::Waiting(text) => layout_text(text, text.len(), width),
        }
    }

    fn newline_binding(&self) -> NewlineBinding {
        match self {
            Self::Ordinary(editor) => editor.newline_binding(),
            Self::Secret(editor) => editor.newline_binding(),
            Self::Waiting(_) => NewlineBinding::default(),
        }
    }
}

#[cfg(test)]
mod tests;
