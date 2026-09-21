use super::{CommandDefinition, CommandEffect, CommandId};
pub(super) static DEFINITION: CommandDefinition = CommandDefinition::new(
    CommandId::Interview,
    "command.interview",
    "/interview",
    "continue or discard this Session's live interview draft; view a finished draft read-only",
    CommandEffect::Interview,
);
