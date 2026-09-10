//! Built-in command registration, uniqueness checks, and lookup.

use std::{collections::HashSet, sync::OnceLock};

use yo_core::ActivityDocument;

use super::{
    CommandDefinition, attach, changes, compact, exit, fork, help, model, new, output, preview,
    prompt, resume, tree,
};

const ORDERED_DEFINITIONS: &[&CommandDefinition] = &[
    &help::DEFINITION,
    &model::DEFINITION,
    &compact::DEFINITION,
    &changes::DEFINITION,
    &output::DEFINITION,
    &preview::DEFINITION,
    &attach::DEFINITION,
    &exit::DEFINITION,
    &new::DEFINITION,
    &fork::DEFINITION,
    &tree::DEFINITION,
    &resume::DEFINITION,
    &prompt::DEFINITION,
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

    pub(crate) fn help_document(&self) -> ActivityDocument {
        let mut commands = String::new();
        for definition in self.definitions {
            commands.push_str(&format!(
                "- `{}`: {}\n",
                definition.invocation(),
                definition.description()
            ));
        }
        help::document(commands)
    }
}

#[cfg(test)]
mod tests;
