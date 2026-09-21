use super::{CommandDefinition, CommandEffect, CommandId};

pub(super) static DEFINITION: CommandDefinition = CommandDefinition::new(
    CommandId::Copy,
    "copy",
    "/copy",
    "Send the latest completed assistant answer to the terminal clipboard",
    CommandEffect::CopyAnswer,
);
