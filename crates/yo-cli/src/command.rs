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
mod tests;

pub(crate) use account::Command as AccountCommand;
pub(crate) use connect::Command as ConnectCommand;
pub(crate) use default::Command as DefaultCommand;
pub(crate) use disconnect::Command as DisconnectCommand;
use error::raw_command_error;
pub(crate) use live::{LiveOptions, LiveSelection, SandboxMode};
pub(crate) use model::Command as ModelCommand;
pub(crate) use output::{OutputFormat, OutputOptions};
pub(crate) use parser::parse;
pub(crate) use print::PrintOptions;
pub(crate) use session::{
    Command as SessionCommand, Content as SessionContent, View as SessionView,
};
pub(crate) use usage::Command as UsageCommand;

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
