//! Built-in command registration, uniqueness checks, and lookup.

use std::{collections::HashSet, sync::OnceLock};

use yo_core::ActivityDocument;

use super::{
    CommandDefinition, CommandId, attach, compact, copy, exit, find, fork, help, interview, model,
    new, output, preview, prompt, resume, secrets, status, tree,
};

const ORDERED_DEFINITIONS: &[&CommandDefinition] = &[
    &help::DEFINITION,
    &model::DEFINITION,
    &status::DEFINITION,
    &compact::DEFINITION,
    &copy::DEFINITION,
    &output::DEFINITION,
    &preview::DEFINITION,
    &attach::DEFINITION,
    &exit::DEFINITION,
    &find::DEFINITION,
    &new::DEFINITION,
    &interview::DEFINITION,
    &fork::DEFINITION,
    &tree::DEFINITION,
    &resume::DEFINITION,
    &secrets::DEFINITION,
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

    pub(crate) fn invocation_in(&self, draft: &str) -> Option<&'static CommandDefinition> {
        let token = draft.split_whitespace().next()?;
        self.definitions
            .iter()
            .copied()
            .find(|definition| definition.invocation().eq_ignore_ascii_case(token))
    }

    pub(super) fn identity(&self, identity: &str) -> Option<&'static CommandDefinition> {
        self.definitions
            .iter()
            .copied()
            .find(|definition| definition.identity() == identity)
    }

    pub(crate) fn matching<'a>(
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

    pub(crate) fn help_document(&self, available: &[CommandId]) -> ActivityDocument {
        let mut commands = String::new();
        for definition in self
            .definitions
            .iter()
            .filter(|definition| available.contains(&definition.id()))
        {
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
