use std::io::IsTerminal;

use yo_core::{HostWorkspacePath, session_repository::StoredSessionReader};

use super::{Command, Output};

pub(crate) fn run(
    storage: &crate::state::storage::LocalReadStorage,
    command: Command,
) -> Result<Output, crate::interaction::diagnostic::AppError> {
    let config = crate::state::config::load().map_err(|error| {
        crate::interaction::diagnostic::AppError::single("loading Yo configuration", error)
    })?;
    let date_formatter = config.date_formatter().map_err(|error| {
        crate::interaction::diagnostic::AppError::single(
            "validating the Session date format",
            error,
        )
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
        let cwd = std::env::current_dir().map_err(|error| {
            crate::interaction::diagnostic::AppError::single("reading the working directory", error)
        })?;
        Some(HostWorkspacePath::normalize_local(cwd).map_err(|error| {
            crate::interaction::diagnostic::AppError::single(
                "normalizing the current workspace",
                error,
            )
        })?)
    };
    let sessions = reader.discover().map_err(|error| {
        crate::interaction::diagnostic::AppError::single("discovering stored Sessions", error)
    })?;
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
        .map_err(|error| {
            crate::interaction::diagnostic::AppError::single(
                "formatting stored Session dates",
                error,
            )
        })?;
    let stdout_is_terminal = std::io::stdout().is_terminal();
    Ok(Output {
        stdout: super::presentation::format_rows(
            &rows,
            command.all,
            command.details,
            super::presentation::output_width(
                stdout_is_terminal,
                yo_tui::terminal::current_width(),
            ),
            super::presentation::heading_style(stdout_is_terminal),
        )
        .map_err(|error| {
            crate::interaction::diagnostic::AppError::single("formatting the Session list", error)
        })?,
        diagnostics: Vec::new(),
    })
}
