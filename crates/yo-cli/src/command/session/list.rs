use std::{env, io, io::IsTerminal};

use yo_core::{HostWorkspacePath, session_repository::StoredSessionReader};
use yo_tui::terminal;

use super::{Command, Output};
use crate::{
    interaction::diagnostic,
    state::{config, storage},
};

pub(crate) fn run(
    storage: &storage::LocalReadStorage,
    command: Command,
) -> Result<Output, diagnostic::AppError> {
    let config = config::load()
        .map_err(|error| diagnostic::AppError::single("loading Yo configuration", error))?;
    let date_formatter = config.date_formatter().map_err(|error| {
        diagnostic::AppError::single("validating the Session date format", error)
    })?;
    let Some(reader) = storage.reader() else {
        return Ok(Output {
            stdout: String::new(),
            diagnostics: Vec::new(),
        });
    };
    let workspace = if command.all {
        None
    } else {
        let cwd = env::current_dir().map_err(|error| {
            diagnostic::AppError::single("reading the working directory", error)
        })?;
        Some(HostWorkspacePath::normalize_local(cwd).map_err(|error| {
            diagnostic::AppError::single("normalizing the current workspace", error)
        })?)
    };
    let sessions = reader
        .discover()
        .map_err(|error| diagnostic::AppError::single("discovering stored Sessions", error))?;
    let rows = sessions
        .into_iter()
        .filter(|session| {
            command.all
                || storage.workspace_host_id().is_some_and(|host| {
                    session.summary().is_some_and(|summary| {
                        let descriptor = summary.discovery().descriptor();
                        descriptor.workspace_host_id() == host
                            && workspace
                                .as_ref()
                                .is_some_and(|workspace| descriptor.workspace_path() == workspace)
                    })
                })
        })
        .map(|session| super::presentation::SessionRow::from_stored(session, &date_formatter))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| diagnostic::AppError::single("formatting stored Session dates", error))?;
    let stdout_is_terminal = io::stdout().is_terminal();
    Ok(Output {
        stdout: super::presentation::format_rows(
            &rows,
            command.all,
            command.details,
            super::presentation::output_width(stdout_is_terminal, terminal::current_width()),
            super::presentation::heading_style(stdout_is_terminal),
        )
        .map_err(|error| diagnostic::AppError::single("formatting the Session list", error))?,
        diagnostics: Vec::new(),
    })
}
