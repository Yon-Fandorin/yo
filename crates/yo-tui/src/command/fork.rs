use super::{CommandDefinition, CommandEffect, CommandId};

pub(super) static DEFINITION: CommandDefinition = CommandDefinition::new(
    CommandId::Fork,
    "command.fork",
    "/fork",
    "branch when idle; /fork at chooses an earlier point",
    CommandEffect::ForkSession,
);

pub(super) fn argument(value: &str) -> Option<&str> {
    value.trim_start().strip_prefix("/fork").and_then(|suffix| {
        suffix
            .is_empty()
            .then_some("")
            .or_else(|| suffix.strip_prefix(char::is_whitespace).map(str::trim))
    })
}
