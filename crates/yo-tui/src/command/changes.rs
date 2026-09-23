use super::{CommandDefinition, CommandEffect, CommandId};

pub(super) static DEFINITION: CommandDefinition = CommandDefinition::new(
    CommandId::Changes,
    "command.changes",
    "/changes",
    "review file changes reported in this conversation (not Git worktree)",
    CommandEffect::ReviewChanges,
);
