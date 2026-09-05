//! Built-in command identities, effects, and immutable definitions.

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum CommandId {
    Help,
    Model,
    Compact,
    Exit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommandEffect {
    ShowHelp,
    SelectModel,
    CompactContext,
    ExitProcess,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CommandDefinition {
    id: CommandId,
    identity: &'static str,
    invocation: &'static str,
    description: &'static str,
    effect: CommandEffect,
}

impl CommandDefinition {
    pub(super) const fn new(
        id: CommandId,
        identity: &'static str,
        invocation: &'static str,
        description: &'static str,
        effect: CommandEffect,
    ) -> Self {
        Self {
            id,
            identity,
            invocation,
            description,
            effect,
        }
    }

    pub(super) const fn id(self) -> CommandId {
        self.id
    }

    pub(super) const fn identity(self) -> &'static str {
        self.identity
    }

    pub(super) const fn description(self) -> &'static str {
        self.description
    }

    pub(crate) const fn invocation(self) -> &'static str {
        self.invocation
    }

    pub(crate) const fn effect(self) -> CommandEffect {
        self.effect
    }
}
