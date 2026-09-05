use std::num::NonZeroUsize;

use clap::{Args, ValueEnum};

use super::output::OutputOptions;

mod list;
mod output;
mod presentation;
mod show;

pub(crate) use output::Output;
pub(crate) use show::read_only_resume_from;
use show::run as run_show;
pub(super) use show::{discovery_diagnostics, read_history_from_reader, with_final_newline};

#[derive(Args, Clone, Debug, Eq, PartialEq)]
pub(super) struct Arguments {
    /// Session to inspect.
    #[arg(value_name = "SESSION_ID", conflicts_with_all = ["all", "details"])]
    session_id: Option<yo_core::SessionId>,

    /// Include Sessions outside the current workspace.
    #[arg(long, conflicts_with = "session_id")]
    all: bool,

    /// Include stored metadata in the Session list.
    #[arg(long, conflicts_with = "session_id")]
    details: bool,

    /// Select the stored Session projection.
    #[arg(long, value_enum, value_name = "VIEW", requires = "session_id")]
    view: Option<View>,

    /// Show at most the newest N semantic Transcript records.
    #[arg(long, value_name = "N", requires = "session_id")]
    limit: Option<NonZeroUsize>,

    /// Select how much record payload the Transcript exposes.
    #[arg(long, value_enum, value_name = "MODE", requires = "session_id")]
    content: Option<Content>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum View {
    Chat,
    Transcript,
    Request,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum Content {
    None,
    Preview,
    Full,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Command {
    pub(crate) session_id: Option<yo_core::SessionId>,
    pub(crate) all: bool,
    pub(crate) details: bool,
    pub(crate) view: View,
    pub(crate) output: OutputOptions,
    pub(crate) limit: Option<NonZeroUsize>,
    pub(crate) content: Option<Content>,
}

impl Arguments {
    pub(super) fn into_command(self, output: OutputOptions) -> Result<Command, clap::Error> {
        let direct = self.session_id.is_some();
        let view = self.view.unwrap_or(View::Chat);
        if view != View::Transcript && (self.limit.is_some() || self.content.is_some()) {
            return Err(super::raw_command_error(
                clap::error::ErrorKind::ArgumentConflict,
                "--limit and --content are supported only with --view transcript",
            ));
        }
        super::output::validate(
            output,
            if direct { "session" } else { "session list" },
            false,
            direct,
        )?;
        Ok(Command {
            session_id: self.session_id,
            all: self.all,
            details: self.details,
            view,
            output,
            limit: self.limit,
            content: self.content,
        })
    }
}

pub(crate) fn run(command: Command) -> Result<Output, crate::diagnostic::AppError> {
    let storage = crate::storage::open_default_reader().map_err(|error| {
        crate::diagnostic::AppError::single("opening read-only local Yo storage", error)
    })?;
    match command.session_id {
        Some(session_id) => run_show(&storage, session_id, command),
        None => list::run(&storage, command),
    }
}
