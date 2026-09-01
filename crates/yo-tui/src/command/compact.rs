use super::{CommandDefinition, CommandEffect, CommandId};

pub(super) static DEFINITION: CommandDefinition = CommandDefinition::new(
    CommandId::Compact,
    "command.compact",
    "/compact",
    "compact idle session context",
    CommandEffect::CompactContext,
);

pub(super) fn argument(value: &str) -> Option<&str> {
    value.strip_prefix("/compact").and_then(|suffix| {
        suffix
            .is_empty()
            .then_some("")
            .or_else(|| suffix.strip_prefix(char::is_whitespace).map(str::trim))
    })
}
