//! Request-bound secret input with no ordinary prompt editing state.

use std::{
    fmt,
    num::NonZeroU16,
    time::{SystemTime, UNIX_EPOCH},
};

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
    RecoveryAvailable,
    Recovered,
}

impl SecretPublicState {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::NotEntered => "Not entered",
            Self::Entered => "Entered",
            Self::ReentryRequired => "Re-entry required",
            Self::RecoveryAvailable => "Recovery available",
            Self::Recovered => "Recovered",
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
    RecoveryDisclosureRequested,
    StoreRecovery(SecretInput),
    RecoverRequested,
    ForgetRecovery,
    RetentionUnavailable,
    RetentionDaysRequested,
    RetentionDaysRejected,
    RetentionChanged(SecretRetention),
}

/// The user's local choice; it is never sent to the model.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum SecretRetention {
    #[default]
    UseOnce,
    ForDays {
        days: u16,
        expires_at: u64,
    },
    UntilDeleted,
}

/// A separate editor for one live secret request.
///
/// Its buffer is never exposed through the prompt renderer or ordinary prompt
/// assist paths. The public projection is deliberately a fixed state label.
pub(crate) struct SecretEditor {
    buffer: TextBuffer,
    public_state: SecretPublicState,
    ready: bool,
    recovery_disclosed: bool,
    retention_offered: bool,
    retention_available: bool,
    retention: SecretRetention,
    retention_days_input: Option<String>,
    public_label: String,
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
            recovery_disclosed: false,
            retention_offered: false,
            retention_available: false,
            retention: SecretRetention::UseOnce,
            retention_days_input: None,
            public_label: SecretPublicState::NotEntered.label().to_owned(),
        }
    }

    pub(crate) fn reentry_required() -> Self {
        Self {
            buffer: TextBuffer::new(),
            public_state: SecretPublicState::ReentryRequired,
            ready: false,
            recovery_disclosed: false,
            retention_offered: false,
            retention_available: false,
            retention: SecretRetention::UseOnce,
            retention_days_input: None,
            public_label: SecretPublicState::ReentryRequired.label().to_owned(),
        }
    }

    pub(crate) fn with_retention_offer(mut self, available: bool) -> Self {
        self.retention_offered = true;
        self.retention_available = available;
        self.refresh_public_label();
        self
    }

    pub(crate) const fn retention(&self) -> SecretRetention {
        self.retention
    }

    fn refresh_public_label(&mut self) {
        let state = if self.public_state == SecretPublicState::RecoveryAvailable {
            "Recovery available · Ctrl-R to load; Enter to submit"
        } else {
            self.public_state.label()
        };
        if !self.retention_offered {
            self.public_label = state.to_owned();
            return;
        }
        let policy = if let Some(days) = &self.retention_days_input {
            format!("Store for… days: {days}")
        } else {
            match self.retention {
                SecretRetention::UseOnce => "Use once".to_owned(),
                SecretRetention::ForDays { days, .. } => format!("Store for {days} days"),
                SecretRetention::UntilDeleted => "Store until deleted".to_owned(),
            }
        };
        self.public_label = format!("{state} · {policy}");
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

    pub(crate) fn mark_recovery_available(&mut self) {
        self.public_state = SecretPublicState::RecoveryAvailable;
        self.recovery_disclosed = false;
        self.refresh_public_label();
    }

    pub(crate) fn mark_recovery_disclosed(&mut self) {
        self.recovery_disclosed = true;
    }

    pub(crate) fn mark_recovery_forgotten(&mut self) {
        self.public_state = if self.buffer.is_empty() {
            SecretPublicState::NotEntered
        } else {
            SecretPublicState::Entered
        };
        self.recovery_disclosed = false;
        self.refresh_public_label();
    }

    pub(crate) fn restore(&mut self, input: SecretInput) {
        self.buffer.clear();
        let value = input.into_inner();
        let inserted = self.buffer.insert(&value);
        debug_assert!(inserted || value.is_empty());
        self.public_state = SecretPublicState::Recovered;
        self.recovery_disclosed = false;
        self.ready = true;
        self.refresh_public_label();
    }

    pub(crate) fn restore_unsubmitted(&mut self, input: SecretInput) {
        self.restore(input);
        self.public_state = SecretPublicState::Entered;
        self.refresh_public_label();
    }

    pub(crate) fn clear(&mut self) {
        self.buffer.clear();
        self.public_state = SecretPublicState::NotEntered;
        self.ready = false;
        self.recovery_disclosed = false;
        self.retention_days_input = None;
        self.retention = SecretRetention::UseOnce;
        self.refresh_public_label();
    }

    /// Returns a fixed public label used by prompt geometry and painting.
    pub(crate) fn public_text(&self) -> &str {
        &self.public_label
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
        self.refresh_public_label();
        SecretInput::new(value).ok()
    }

    pub(crate) fn handle(&mut self, event: InputEvent) -> SecretEditorEffect {
        if !self.ready {
            return SecretEditorEffect::NoChange;
        }
        match event {
            InputEvent::Paste(text) if self.retention_days_input.is_some() => {
                let days = self
                    .retention_days_input
                    .as_mut()
                    .expect("guarded day entry");
                if text.is_empty() {
                    return SecretEditorEffect::NoChange;
                }
                if !text.bytes().all(|byte| byte.is_ascii_digit()) || days.len() + text.len() > 3 {
                    return SecretEditorEffect::RetentionDaysRejected;
                }
                days.push_str(&text);
                self.refresh_public_label();
                SecretEditorEffect::Changed
            },
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
            self.recovery_disclosed = false;
            self.refresh_public_label();
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
        if matches!(key.code, KeyCode::Character('c' | 'C'))
            && key.modifiers == KeyModifiers::CONTROL
        {
            return SecretEditorEffect::Cancel;
        }
        if let Some(days) = self.retention_days_input.as_mut() {
            let effect = match (key.code, key.modifiers) {
                (KeyCode::Character(digit @ '0'..='9'), KeyModifiers::NONE) if days.len() < 3 => {
                    days.push(digit);
                    SecretEditorEffect::Changed
                },
                (KeyCode::Character('0'..='9'), KeyModifiers::NONE) => {
                    SecretEditorEffect::RetentionDaysRejected
                },
                (KeyCode::Backspace, KeyModifiers::NONE) => {
                    days.pop();
                    SecretEditorEffect::Changed
                },
                (KeyCode::Enter, KeyModifiers::NONE) => match days.parse::<u16>() {
                    Ok(value @ 1..=365) => {
                        let expires_at = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .ok()
                            .and_then(|duration| {
                                duration.as_secs().checked_add(u64::from(value) * 86_400)
                            });
                        if let Some(expires_at) = expires_at {
                            self.retention_days_input = None;
                            self.retention = SecretRetention::ForDays {
                                days: value,
                                expires_at,
                            };
                            SecretEditorEffect::RetentionChanged(self.retention)
                        } else {
                            SecretEditorEffect::RetentionDaysRejected
                        }
                    },
                    _ => SecretEditorEffect::RetentionDaysRejected,
                },
                (KeyCode::Character('s' | 'S'), KeyModifiers::CONTROL) => {
                    self.retention_days_input = None;
                    self.retention = SecretRetention::UntilDeleted;
                    SecretEditorEffect::RetentionChanged(self.retention)
                },
                _ => SecretEditorEffect::NoChange,
            };
            self.refresh_public_label();
            return effect;
        }
        if key.modifiers == KeyModifiers::CONTROL {
            match key.code {
                KeyCode::Character('s' | 'S') if self.retention_offered => {
                    if !self.retention_available {
                        return SecretEditorEffect::RetentionUnavailable;
                    }
                    let effect = match self.retention {
                        SecretRetention::UseOnce => {
                            self.retention_days_input = Some(String::new());
                            SecretEditorEffect::RetentionDaysRequested
                        },
                        SecretRetention::ForDays { .. } => {
                            self.retention = SecretRetention::UntilDeleted;
                            SecretEditorEffect::RetentionChanged(self.retention)
                        },
                        SecretRetention::UntilDeleted => {
                            self.retention = SecretRetention::UseOnce;
                            SecretEditorEffect::RetentionChanged(self.retention)
                        },
                    };
                    self.refresh_public_label();
                    return effect;
                },
                KeyCode::Character('r' | 'R') => {
                    if self.public_state == SecretPublicState::RecoveryAvailable
                        && self.buffer.is_empty()
                    {
                        return SecretEditorEffect::RecoverRequested;
                    }
                    if self.buffer.is_empty() {
                        return SecretEditorEffect::NoChange;
                    }
                    if !self.recovery_disclosed {
                        return SecretEditorEffect::RecoveryDisclosureRequested;
                    }
                    let input = SecretInput::new(self.buffer.as_str().to_owned())
                        .expect("the editor enforces the secret input bound");
                    return SecretEditorEffect::StoreRecovery(input);
                },
                KeyCode::Character('f' | 'F') => {
                    return SecretEditorEffect::ForgetRecovery;
                },
                KeyCode::Character('u' | 'U') => {
                    let removed = self.buffer.kill_line_start();
                    return if removed.is_some() {
                        self.public_state = if self.buffer.is_empty() {
                            SecretPublicState::NotEntered
                        } else {
                            SecretPublicState::Entered
                        };
                        self.recovery_disclosed = false;
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
            self.recovery_disclosed = false;
            self.refresh_public_label();
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
