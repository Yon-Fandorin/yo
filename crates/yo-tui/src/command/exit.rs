use super::{CommandDefinition, CommandEffect, CommandId};

pub(super) static DEFINITION: CommandDefinition = CommandDefinition::new(
    CommandId::Exit,
    "command.exit",
    "/exit",
    "close yo",
    CommandEffect::ExitProcess,
);
