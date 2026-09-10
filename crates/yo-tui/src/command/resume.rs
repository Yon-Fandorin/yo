use super::{CommandDefinition, CommandEffect, CommandId};

pub(super) static DEFINITION: CommandDefinition = CommandDefinition::new(
    CommandId::Resume,
    "command.resume",
    "/resume",
    "choose a saved session, or resume its full UUID",
    CommandEffect::ResumeSession,
);

pub(super) fn argument(value: &str) -> Option<&str> {
    value.strip_prefix("/resume").and_then(|suffix| {
        suffix
            .is_empty()
            .then_some("")
            .or_else(|| suffix.strip_prefix(char::is_whitespace).map(str::trim))
    })
}
