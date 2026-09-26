//! Built-in command identities, effects, and immutable definitions.

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum CommandId {
    Help,
    New,
    Interview,
    Fork,
    Tree,
    Resume,
    Secrets,
    Prompt,
    Model,
    Status,
    Compact,
    Copy,
    Output,
    Preview,
    Attach,
    Exit,
    Find,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommandEffect {
    ShowHelp,
    NewSession,
    Interview,
    ForkSession,
    ShowSessionTree,
    ResumeSession,
    ShowSecrets,
    InsertPrompt,
    SelectModel,
    ShowStatus,
    CompactContext,
    CopyAnswer,
    ReviewOutput,
    OpenPreview,
    AttachImage,
    ExitProcess,
    FindMessages,
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

    pub(crate) const fn id(self) -> CommandId {
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
