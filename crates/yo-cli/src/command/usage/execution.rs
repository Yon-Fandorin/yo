use yo_core::session_repository::StoredSessionReader;
use yo_tui::project_archived_usage;

use super::{
    super::session::{
        Output as SessionOutput, discovery_diagnostics, read_history_from_reader,
        with_final_newline,
    },
    Command,
};

pub(crate) fn run(command: Command) -> Result<SessionOutput, crate::diagnostic::AppError> {
    let storage = crate::storage::open_default_reader().map_err(|error| {
        crate::diagnostic::AppError::single("opening read-only local Yo storage", error)
    })?;
    let reader = storage
        .reader()
        .map(|reader| reader as &dyn StoredSessionReader);
    show_from_reader(reader, command)
}

pub(crate) fn show_from_reader(
    reader: Option<&dyn StoredSessionReader>,
    command: Command,
) -> Result<SessionOutput, crate::diagnostic::AppError> {
    let history = read_history_from_reader(reader, command.session_id)?;
    let stdout =
        project_archived_usage(&history, command.output.glyph_profile).map_err(|error| {
            crate::diagnostic::AppError::single("projecting stored Session history", error)
        })?;
    Ok(SessionOutput {
        stdout: with_final_newline(stdout),
        diagnostics: discovery_diagnostics(command.session_id, history.discovery_validation()),
    })
}
