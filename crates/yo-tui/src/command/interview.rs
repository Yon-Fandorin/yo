use super::{CommandDefinition, CommandEffect, CommandId};
pub(super) static DEFINITION: CommandDefinition = CommandDefinition::new(
    CommandId::Interview,
    "command.interview",
    "/interview",
    "recover, reopen and edit a saved interview; send explicitly as a new conversation",
    CommandEffect::Interview,
);
