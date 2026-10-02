use super::{CommandDefinition, CommandEffect, CommandId};

pub(super) static DEFINITION: CommandDefinition = CommandDefinition::new(
    CommandId::Output,
    "command.output",
    "/output",
    "browse retained tool output (Up/Down, Left/Right items, F1 Chat)",
    CommandEffect::ReviewOutput,
);
