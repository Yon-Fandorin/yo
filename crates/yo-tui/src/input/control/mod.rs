//! Ctrl+C and Ctrl+D policy at the prompt boundary.

use std::time::Duration;

use super::{
    buffer::TextBuffer,
    event::{KeyAction, KeyCode, KeyEvent, KeyModifiers},
};

const EMPTY_CTRL_C_EXIT_WINDOW: Duration = Duration::from_millis(1_500);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ControlEffect {
    Unhandled,
    NoChange,
    BufferChanged,
    ExitArmed,
    InterruptTask,
    Exit,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ControlKeyPolicy {
    empty_ctrl_c_at: Option<Duration>,
}

impl ControlKeyPolicy {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn handle(
        &mut self,
        key: KeyEvent,
        task_active: bool,
        buffer: &mut TextBuffer,
        now: Duration,
    ) -> ControlEffect {
        if key.action == KeyAction::Release {
            return ControlEffect::Unhandled;
        }

        if is_control_character(key, 'c') {
            return self.handle_ctrl_c(key.action, task_active, buffer, now);
        }

        self.empty_ctrl_c_at = None;

        if is_control_character(key, 'd') {
            return self.handle_ctrl_d(key.action, buffer);
        }

        ControlEffect::Unhandled
    }

    pub(crate) fn cancel_exit_sequence(&mut self) {
        self.empty_ctrl_c_at = None;
    }

    fn handle_ctrl_c(
        &mut self,
        action: KeyAction,
        task_active: bool,
        buffer: &mut TextBuffer,
        now: Duration,
    ) -> ControlEffect {
        if action == KeyAction::Repeat {
            return ControlEffect::NoChange;
        }

        if task_active {
            self.empty_ctrl_c_at = None;
            return ControlEffect::InterruptTask;
        }

        if buffer.clear() {
            self.empty_ctrl_c_at = None;
            return ControlEffect::BufferChanged;
        }

        let exits = self.empty_ctrl_c_at.is_some_and(|previous| {
            now.checked_sub(previous)
                .is_some_and(|elapsed| elapsed <= EMPTY_CTRL_C_EXIT_WINDOW)
        });

        if exits {
            self.empty_ctrl_c_at = None;
            ControlEffect::Exit
        } else {
            self.empty_ctrl_c_at = Some(now);
            ControlEffect::ExitArmed
        }
    }

    fn handle_ctrl_d(&mut self, action: KeyAction, buffer: &mut TextBuffer) -> ControlEffect {
        if buffer.is_empty() {
            return if action == KeyAction::Press {
                ControlEffect::Exit
            } else {
                ControlEffect::NoChange
            };
        }

        if buffer.delete_forward() {
            ControlEffect::BufferChanged
        } else {
            ControlEffect::NoChange
        }
    }
}

fn is_control_character(key: KeyEvent, character: char) -> bool {
    key.modifiers == KeyModifiers::CONTROL
        && matches!(key.code, KeyCode::Character(value) if value.eq_ignore_ascii_case(&character))
}

#[cfg(test)]
mod tests;
