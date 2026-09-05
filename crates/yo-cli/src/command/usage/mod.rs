use clap::Args;

use super::output::OutputOptions;

mod execution;

#[cfg(test)]
mod tests;

#[derive(Args, Clone, Debug, Eq, PartialEq)]
pub(super) struct Arguments {
    /// Session whose usage should be shown.
    #[arg(value_name = "SESSION_ID")]
    session_id: yo_core::SessionId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Command {
    pub(crate) session_id: yo_core::SessionId,
    pub(crate) output: OutputOptions,
}

impl Arguments {
    pub(super) fn into_command(self, output: OutputOptions) -> Result<Command, clap::Error> {
        super::output::validate(output, "usage", false, true)?;
        Ok(Command {
            session_id: self.session_id,
            output,
        })
    }
}

pub(crate) use execution::run;
