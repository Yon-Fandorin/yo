use super::{CommandDefinition, CommandEffect, CommandId};

pub(super) static DEFINITION: CommandDefinition = CommandDefinition::new(
    CommandId::Find,
    "command.find",
    "/find",
    "find a finalized Chat message",
    CommandEffect::FindMessages,
);

pub(super) fn argument(value: &str) -> Option<&str> {
    value.strip_prefix("/find").and_then(|suffix| {
        suffix
            .is_empty()
            .then_some("")
            .or_else(|| suffix.strip_prefix(char::is_whitespace).map(str::trim))
    })
}
