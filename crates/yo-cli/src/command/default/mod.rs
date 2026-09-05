use clap::Args;

mod execution;

#[derive(Args, Clone, Debug, Eq, PartialEq)]
#[group(required = true, multiple = false)]
pub(super) struct Arguments {
    /// Exact admitted startup target to persist.
    #[arg(value_name = "TARGET", allow_hyphen_values = true)]
    target: Option<String>,

    /// Clear the stored startup default.
    #[arg(long)]
    unset: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Command {
    pub(crate) target: Option<String>,
}

impl Arguments {
    pub(super) fn into_command(
        self,
        output: super::output::OutputOptions,
    ) -> Result<Command, clap::Error> {
        super::output::validate(output, "default", false, false)?;
        Ok(Command {
            target: self.target,
        })
    }
}

pub(crate) use execution::run;
