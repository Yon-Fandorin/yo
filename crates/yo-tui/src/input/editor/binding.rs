//! Validated prompt key bindings independent of configuration storage.

use crate::input::event::KeyModifiers;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NewlineBinding {
    modifiers: KeyModifiers,
}

impl NewlineBinding {
    pub(crate) fn new(modifiers: KeyModifiers) -> Result<Self, NewlineBindingError> {
        if modifiers == KeyModifiers::NONE {
            return Err(NewlineBindingError::ConflictsWithSubmit);
        }

        Ok(Self { modifiers })
    }

    pub(crate) fn matches(self, modifiers: KeyModifiers) -> bool {
        self.modifiers == modifiers
    }
}

impl Default for NewlineBinding {
    fn default() -> Self {
        Self {
            modifiers: KeyModifiers::SHIFT,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NewlineBindingError {
    ConflictsWithSubmit,
}
