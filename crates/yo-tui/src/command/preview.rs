use super::{CommandDefinition, CommandEffect, CommandId};

pub(super) static DEFINITION: CommandDefinition = CommandDefinition::new(
    CommandId::Preview,
    "command.preview",
    "/preview",
    "open offline interactive UI preview (repeat to return)",
    CommandEffect::OpenPreview,
);
