use super::{CommandDefinition, CommandEffect, CommandId};

pub(super) static DEFINITION: CommandDefinition = CommandDefinition::new(
    CommandId::Changes,
    "command.changes",
    "/changes",
    "review observed file changes (Left/Right files, F1 Chat)",
    CommandEffect::ReviewChanges,
);
