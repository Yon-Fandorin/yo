//! Top-level CLI grammar and normalized command values.

mod account;
mod connect;
mod default;
mod disconnect;
mod error;
mod live;
mod model;
mod output;
mod parser;
mod print;
mod session;
mod usage;

#[cfg(test)]
mod connection_presentation_tests;

#[cfg(test)]
mod tests;

pub(crate) use account::{
    AccountCompletion, AccountRunOutput, Command as AccountCommand, run as run_account,
};
pub(crate) use connect::{Command as ConnectCommand, run as run_connect};
pub(crate) use default::{Command as DefaultCommand, run as run_default};
pub(crate) use disconnect::{Command as DisconnectCommand, run as run_disconnect};
use error::raw_command_error;
pub(crate) use live::{LiveOptions, LiveSelection};
pub(crate) use model::{Command as ModelCommand, run as run_model_activation};
pub(crate) use output::OutputFormat;
#[cfg(test)]
pub(crate) use output::OutputOptions;
pub(crate) use parser::parse;
pub(crate) use print::PrintOptions;
pub(crate) use session::{
    Command as SessionCommand, Output as SessionOutput, read_only_resume_from, run as run_session,
};
#[cfg(test)]
pub(crate) use session::{Content as SessionContent, View as SessionView};
pub(crate) use usage::{Command as UsageCommand, run as run_usage};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Command {
    Account(AccountCommand),
    Connect(ConnectCommand),
    Disconnect(DisconnectCommand),
    Default(DefaultCommand),
    Model(ModelCommand),
    Live(LiveOptions),
    Print(PrintOptions),
    Session(SessionCommand),
    Usage(UsageCommand),
}
