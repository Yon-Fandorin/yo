//! Built-in command registration, uniqueness checks, and lookup.

use std::{collections::HashSet, sync::OnceLock};

use super::{CommandDefinition, compact, exit, help, model};

const ORDERED_DEFINITIONS: &[&CommandDefinition] = &[
    &help::DEFINITION,
    &model::DEFINITION,
    &compact::DEFINITION,
    &exit::DEFINITION,
];

#[derive(Debug)]
pub(crate) struct CommandRegistry {
    definitions: &'static [&'static CommandDefinition],
}

impl CommandRegistry {
    pub(crate) fn built_in() -> &'static Self {
        static REGISTRY: OnceLock<CommandRegistry> = OnceLock::new();
        REGISTRY.get_or_init(|| {
            let mut ids = HashSet::new();
            let mut invocations = HashSet::new();
            for definition in ORDERED_DEFINITIONS {
                assert!(
                    ids.insert(definition.id()),
                    "built-in command identities must be unique"
                );
                assert!(
                    invocations.insert(definition.invocation()),
                    "built-in command invocations must be unique"
                );
            }
            Self {
                definitions: ORDERED_DEFINITIONS,
            }
        })
    }

    pub(super) fn exact_query(&self, query: &str) -> Option<&'static CommandDefinition> {
        self.definitions.iter().copied().find(|definition| {
            definition
                .invocation()
                .strip_prefix('/')
                .expect("built-in command invocations start with a slash")
                == query
        })
    }

    pub(super) fn identity(&self, identity: &str) -> Option<&'static CommandDefinition> {
        self.definitions
            .iter()
            .copied()
            .find(|definition| definition.identity() == identity)
    }

    pub(super) fn matching<'a>(
        &'a self,
        query: &'a str,
    ) -> impl Iterator<Item = &'static CommandDefinition> + 'a {
        self.definitions.iter().copied().filter(move |definition| {
            definition
                .invocation()
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
                definition.invocation(),
                definition.description()
            ));
        }
        notice
    }
}

#[cfg(test)]
mod tests;
