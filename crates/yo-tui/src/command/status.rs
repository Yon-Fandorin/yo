use super::{CommandDefinition, CommandEffect, CommandId};

pub(super) static DEFINITION: CommandDefinition = CommandDefinition::new(
    CommandId::Status,
    "command.status",
    "/status",
    "show the current Session identity and observed status",
    CommandEffect::ShowStatus,
);
