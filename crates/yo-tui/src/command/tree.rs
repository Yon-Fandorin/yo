use super::{CommandDefinition, CommandEffect, CommandId};

pub(super) static DEFINITION: CommandDefinition = CommandDefinition::new(
    CommandId::Tree,
    "command.tree",
    "/tree",
    "browse verified session branches when idle",
    CommandEffect::ShowSessionTree,
);

pub(super) fn argument(value: &str) -> Option<&str> {
    value.trim_start().strip_prefix("/tree").and_then(|suffix| {
        suffix
            .is_empty()
            .then_some("")
            .or_else(|| suffix.strip_prefix(char::is_whitespace).map(str::trim))
    })
}
