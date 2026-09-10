use super::{CommandDefinition, CommandEffect, CommandId};

pub(super) static DEFINITION: CommandDefinition = CommandDefinition::new(
    CommandId::New,
    "command.new",
    "/new",
    "start an independent session when idle",
    CommandEffect::NewSession,
);
