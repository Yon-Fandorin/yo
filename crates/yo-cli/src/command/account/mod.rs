use clap::Args;

use super::output::OutputOptions;

#[derive(Args, Clone, Debug, Eq, PartialEq)]
pub(super) struct Arguments {
    /// Optional account source: a Provider, or an exact Provider:Account pair.
    #[arg(value_name = "SOURCE")]
    source: Option<String>,

    /// Re-observe the selected account source before displaying it.
    #[arg(long)]
    refresh: bool,

    /// Show the full multi-line capacity details instead of the account table (text only).
    #[arg(long)]
    detail: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Command {
    pub(crate) source: Option<String>,
    pub(crate) refresh: bool,
    pub(crate) detail: bool,
    pub(crate) output: OutputOptions,
}

impl Arguments {
    pub(super) fn into_command(self, output: OutputOptions) -> Result<Command, clap::Error> {
        super::output::validate(output, "account", true, true)?;
        Ok(Command {
            source: self.source,
            refresh: self.refresh,
            detail: self.detail,
            output,
        })
    }
}
