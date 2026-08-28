use super::{CommandDefinition, CommandEffect, CommandId};

pub(super) static DEFINITION: CommandDefinition = CommandDefinition::new(
    CommandId::Model,
    "command.model",
    "/model",
    "select the session model",
    CommandEffect::SelectModel,
);

pub(super) fn argument(value: &str) -> Option<&str> {
    value.strip_prefix("/model").and_then(|suffix| {
        suffix
            .is_empty()
            .then_some("")
            .or_else(|| suffix.strip_prefix(char::is_whitespace).map(str::trim))
    })
}
