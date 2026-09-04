use yo_core::session_repository::StoredSessionReader;
use yo_tui::project_archived_usage;

use super::{AppError, command::UsageCommand, session};

/// Opens the existing read-only Session path and renders its typed Usage projection.
pub(crate) fn run(command: UsageCommand) -> Result<session::Output, AppError> {
    let storage = super::storage::open_default_reader()
        .map_err(|error| AppError::single("opening read-only local Yo storage", error))?;
    let reader = storage
        .reader()
        .map(|reader| reader as &dyn StoredSessionReader);
    show_from_reader(reader, command)
}

pub(crate) fn show_from_reader(
    reader: Option<&dyn StoredSessionReader>,
    command: UsageCommand,
) -> Result<session::Output, AppError> {
    let history = session::read_history_from_reader(reader, command.session_id)?;
    let stdout = project_archived_usage(&history, command.output.glyph_profile)
        .map_err(|error| AppError::single("projecting stored Session history", error))?;
    Ok(session::Output {
        stdout: session::with_final_newline(stdout),
        diagnostics: session::discovery_diagnostics(
            command.session_id,
            history.discovery_validation(),
        ),
    })
}
