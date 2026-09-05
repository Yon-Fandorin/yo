//! Prompt-local command query, overlay, and escape lifecycle.

use super::{CommandDefinition, CommandRegistry};
use crate::overlay::{
    AcceptanceReceipt, OverlayInstanceToken, PanelSnapshot, PromptOverlaySlot, SelectionEntry,
};

#[derive(Debug)]
struct ActivePalette {
    token: OverlayInstanceToken,
    query: String,
}

#[derive(Debug, Default)]
pub(crate) struct CommandPalette {
    active: Option<ActivePalette>,
    escaped_draft: Option<String>,
}

impl CommandPalette {
    pub(crate) fn sync(
        &mut self,
        text: &str,
        cursor: usize,
        overlay: &mut PromptOverlaySlot,
        eligible: bool,
    ) {
        if self
            .escaped_draft
            .as_deref()
            .is_some_and(|escaped| escaped != text)
        {
            self.escaped_draft = None;
        }
        if self.escaped_draft.as_deref() == Some(text) {
            self.close(overlay);
            return;
        }
        let Some(query) = eligible.then(|| command_query(text, cursor)).flatten() else {
            self.close(overlay);
            return;
        };
        let snapshot = panel_snapshot(&query);
        if let Some(active) = self.active.as_mut() {
            if active.query == query && overlay.is_current(active.token) {
                return;
            }
            if overlay.refresh(active.token, snapshot.clone()).is_ok() {
                active.query = query;
                return;
            }
        }
        self.active = overlay
            .open_accepting_empty(snapshot)
            .ok()
            .map(|token| ActivePalette { token, query });
    }

    pub(crate) fn accept(
        &mut self,
        receipt: &AcceptanceReceipt,
    ) -> Option<&'static CommandDefinition> {
        let active = self.active.as_ref()?;
        if receipt.token() != active.token {
            return None;
        }
        self.active = None;
        CommandRegistry::built_in().identity(receipt.identity())
    }

    pub(crate) fn reject_visible(
        &mut self,
        token: OverlayInstanceToken,
        overlay: &mut PromptOverlaySlot,
    ) -> bool {
        if !self
            .active
            .as_ref()
            .is_some_and(|active| active.token == token)
        {
            return false;
        }
        self.close(overlay);
        true
    }

    pub(crate) fn dismiss_visible(
        &mut self,
        token: OverlayInstanceToken,
        unchanged_draft: &str,
    ) -> bool {
        let Some(active) = self.active.as_ref() else {
            return false;
        };
        if active.token != token {
            return false;
        }
        self.active = None;
        self.escaped_draft = Some(unchanged_draft.to_owned());
        true
    }

    pub(crate) fn take_escape(&mut self, text: &str) -> bool {
        if self.escaped_draft.as_deref() != Some(text) {
            return false;
        }
        self.escaped_draft = None;
        true
    }

    pub(crate) fn owns_submission(&self, text: &str, cursor: usize) -> bool {
        self.escaped_draft.as_deref() != Some(text) && command_query(text, cursor).is_some()
    }

    pub(crate) fn exact_submission(
        &self,
        text: &str,
        cursor: usize,
    ) -> Option<&'static CommandDefinition> {
        if self.escaped_draft.as_deref() == Some(text) {
            return None;
        }
        let query = command_query(text, cursor)?;
        CommandRegistry::built_in().exact_query(&query)
    }

    pub(crate) fn close(&mut self, overlay: &mut PromptOverlaySlot) {
        if let Some(active) = self.active.take() {
            let _ = overlay.close(active.token);
        }
    }

    pub(crate) fn dismiss(&mut self) {
        self.active = None;
    }
}

fn command_query(text: &str, cursor: usize) -> Option<String> {
    if cursor != text.len() {
        return None;
    }
    let prefix = text.get(..cursor)?;
    let token_start = prefix
        .char_indices()
        .rev()
        .find_map(|(index, character)| {
            character
                .is_whitespace()
                .then_some(index + character.len_utf8())
        })
        .unwrap_or(0);
    if !prefix[..token_start].chars().all(char::is_whitespace) {
        return None;
    }
    let query = prefix.get(token_start..)?.strip_prefix('/')?;
    if query.chars().any(char::is_whitespace) {
        return None;
    }
    Some(query.to_ascii_lowercase())
}

fn panel_snapshot(query: &str) -> PanelSnapshot {
    let mut entries = CommandRegistry::built_in()
        .matching(query)
        .map(|definition| {
            SelectionEntry::enabled_with_context(
                definition.identity(),
                definition.invocation(),
                Some(definition.description().to_owned()),
                None,
            )
        })
        .collect::<Vec<_>>();
    if entries.is_empty() {
        entries.push(SelectionEntry::disabled(
            "command-status",
            "No matching commands",
            None,
            "not selectable",
        ));
    }
    PanelSnapshot::new("Commands", entries).expect("the built-in command palette is valid")
}

#[cfg(test)]
mod tests;
