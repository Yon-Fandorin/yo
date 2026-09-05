use clap::{Args, Subcommand};

mod execution;

#[derive(Args, Clone, Debug, Eq, PartialEq)]
pub(super) struct Arguments {
    #[command(subcommand)]
    action: ActionArguments,
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
enum ActionArguments {
    /// Admit one stored model binding for new work.
    Enable(TargetArguments),
    /// Reject one stored model binding for new work while preserving it for later use.
    Disable(TargetArguments),
}

#[derive(Args, Clone, Debug, Eq, PartialEq)]
struct TargetArguments {
    /// Exact stored model target to mutate.
    #[arg(value_name = "TARGET", allow_hyphen_values = true)]
    target: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Command {
    pub(crate) target: String,
    pub(crate) enabled: bool,
}

impl Arguments {
    pub(super) fn into_command(
        self,
        output: super::output::OutputOptions,
    ) -> Result<Command, clap::Error> {
        super::output::validate(output, "model", false, false)?;
        let (target, enabled) = match self.action {
            ActionArguments::Enable(arguments) => (arguments.target, true),
            ActionArguments::Disable(arguments) => (arguments.target, false),
        };
        Ok(Command { target, enabled })
    }
}

pub(crate) use execution::run;
