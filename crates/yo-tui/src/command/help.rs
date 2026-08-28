use super::{CommandDefinition, CommandEffect, CommandId};

pub(super) static DEFINITION: CommandDefinition = CommandDefinition::new(
    CommandId::Help,
    "command.help",
    "/help",
    "show available commands",
    CommandEffect::ShowHelp,
);
