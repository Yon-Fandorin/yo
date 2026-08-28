//! Prompt-local built-in commands and their shallow composition registry.

mod exit;
mod help;
mod model;

use std::{collections::HashSet, sync::OnceLock};

use crate::overlay::{
    AcceptanceReceipt, OverlayInstanceToken, PanelSnapshot, PromptOverlaySlot, SelectionEntry,
};

const ORDERED_DEFINITIONS: &[&CommandDefinition] =
    &[&help::DEFINITION, &model::DEFINITION, &exit::DEFINITION];

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum CommandId {
    Help,
    Model,
    Exit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommandEffect {
    ShowHelp,
    SelectModel,
    ExitProcess,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CommandDefinition {
    id: CommandId,
    identity: &'static str,
    invocation: &'static str,
    description: &'static str,
    effect: CommandEffect,
}

#[derive(Debug)]
pub(crate) struct CommandRegistry {
    definitions: &'static [&'static CommandDefinition],
}

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

impl CommandDefinition {
    const fn new(
        id: CommandId,
        identity: &'static str,
        invocation: &'static str,
        description: &'static str,
        effect: CommandEffect,
    ) -> Self {
        Self {
            id,
            identity,
            invocation,
            description,
            effect,
        }
    }

    pub(crate) const fn invocation(self) -> &'static str {
        self.invocation
    }

    pub(crate) const fn effect(self) -> CommandEffect {
        self.effect
    }
}

impl CommandRegistry {
    pub(crate) fn built_in() -> &'static Self {
        static REGISTRY: OnceLock<CommandRegistry> = OnceLock::new();
        REGISTRY.get_or_init(|| {
            let mut ids = HashSet::new();
            let mut invocations = HashSet::new();
            for definition in ORDERED_DEFINITIONS {
                assert!(
                    ids.insert(definition.id),
                    "built-in command identities must be unique"
                );
                assert!(
                    invocations.insert(definition.invocation),
                    "built-in command invocations must be unique"
                );
            }
            Self {
                definitions: ORDERED_DEFINITIONS,
            }
        })
    }

    fn exact_query(&self, query: &str) -> Option<&'static CommandDefinition> {
        self.definitions.iter().copied().find(|definition| {
            definition
                .invocation
                .strip_prefix('/')
                .expect("built-in command invocations start with a slash")
                == query
        })
    }

    fn identity(&self, identity: &str) -> Option<&'static CommandDefinition> {
        self.definitions
            .iter()
            .copied()
            .find(|definition| definition.identity == identity)
    }

    fn matching<'a>(
        &'a self,
        query: &'a str,
    ) -> impl Iterator<Item = &'static CommandDefinition> + 'a {
        self.definitions.iter().copied().filter(move |definition| {
            definition
                .invocation
                .strip_prefix('/')
                .expect("built-in command invocations start with a slash")
                .to_ascii_lowercase()
                .starts_with(query)
        })
    }

    pub(crate) fn help_notice(&self) -> String {
        let mut notice = String::from("Available commands:");
        for definition in self.definitions {
            notice.push_str(&format!(
                "\n  {:<7} {}",
                definition.invocation, definition.description
            ));
        }
        notice
    }
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

pub(crate) fn model_argument(value: &str) -> Option<&str> {
    model::argument(value)
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
                definition.identity,
                definition.invocation,
                Some(definition.description.to_owned()),
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
mod tests {
    use super::{CommandId, CommandRegistry, command_query};

    // draft 전체의 첫 slash token만 ASCII case를 정규화한 query가 되고, 앞쪽 공백은
    // command 소유권을 바꾸지 않는다.
    #[test]
    fn first_slash_token_produces_a_normalized_query() {
        assert_eq!(command_query("/MO", 3).as_deref(), Some("mo"));
        assert_eq!(command_query("  /he", 5).as_deref(), Some("he"));
    }

    // 일반 문장에 포함된 slash, argument가 붙은 command, cursor 뒤 text가 남은 draft는
    // palette query가 아니므로 command controller가 입력을 가로채지 않는다.
    #[test]
    fn embedded_or_completed_slash_tokens_are_not_commands() {
        assert_eq!(command_query("explain /", 9), None);
        assert_eq!(command_query("/model other", 12), None);
        assert_eq!(command_query("/h keep", 2), None);
    }

    // registry filtering 결과는 각 module definition을 help, model, exit 순으로 합성한
    // 안정된 제품 순서를 그대로 보존한다.
    #[test]
    fn command_filter_preserves_module_declared_order() {
        assert_eq!(
            CommandRegistry::built_in()
                .matching("")
                .map(|definition| definition.id)
                .collect::<Vec<_>>(),
            vec![CommandId::Help, CommandId::Model, CommandId::Exit]
        );
    }

    // terminal control text를 query로 받아도 panel row에 투영하지 않고 안전한 disabled
    // no-match snapshot을 만들 수 있어야 한다.
    #[test]
    fn unsafe_query_text_is_not_projected_into_the_panel() {
        super::panel_snapshot("\u{1b}");
    }
}
