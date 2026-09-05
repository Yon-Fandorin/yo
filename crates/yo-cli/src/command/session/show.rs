use yo_core::{
    SessionId,
    session_repository::{
        StoredDiscoveryMismatch, StoredDiscoveryValidation, StoredSessionContinuity,
        StoredSessionHistory, StoredSessionReadError, StoredSessionReader, read_stored_session,
    },
};
use yo_tui::{
    ArchivedContentPolicy, ArchivedProjectionOptions, ArchivedSessionView,
    project_archived_session_with_options,
};

use super::{Command, Content, Output, View};

pub(super) fn run(
    storage: &crate::storage::LocalReadStorage,
    session_id: SessionId,
    command: Command,
) -> Result<Output, crate::diagnostic::AppError> {
    let reader = storage
        .reader()
        .map(|reader| reader as &dyn StoredSessionReader);
    show_from_reader(reader, session_id, command)
}

pub(crate) fn read_only_resume_from(
    reader: &dyn StoredSessionReader,
    session_id: SessionId,
    glyph_profile: yo_tui::GlyphProfile,
    reason: &str,
) -> Result<Output, crate::diagnostic::AppError> {
    let mut output = show_from_reader(
        Some(reader),
        session_id,
        Command {
            session_id: Some(session_id),
            all: false,
            details: false,
            view: View::Chat,
            output: super::super::output::OutputOptions {
                format: super::super::output::OutputFormat::Text,
                glyph_profile,
            },
            limit: None,
            content: None,
        },
    )?;
    output.diagnostics.push(crate::diagnostic::CliDiagnostic::warning(format!(
        "stored Session {session_id} continuation is unavailable ({reason}); opened durable history read-only"
    )));
    Ok(output)
}

fn show_from_reader(
    reader: Option<&dyn StoredSessionReader>,
    session_id: SessionId,
    command: Command,
) -> Result<Output, crate::diagnostic::AppError> {
    let history = read_history_from_reader(reader, session_id)?;
    let view = match command.view {
        View::Chat => ArchivedSessionView::Chat,
        View::Transcript => ArchivedSessionView::Transcript,
        View::Request => ArchivedSessionView::Request,
    };
    let options = ArchivedProjectionOptions::new(
        command.limit,
        match command.content {
            Some(Content::None) => ArchivedContentPolicy::None,
            Some(Content::Preview) => ArchivedContentPolicy::Preview,
            Some(Content::Full) | None => ArchivedContentPolicy::Full,
        },
    );
    let stdout = project_archived_session_with_options(
        &history,
        view,
        command.output.glyph_profile,
        options,
    )
    .map_err(|error| {
        crate::diagnostic::AppError::single("projecting stored Session history", error)
    })?;
    let diagnostics = archival_diagnostics(
        session_id,
        command.view,
        history.continuity(),
        history.discovery_validation(),
    );
    Ok(Output {
        stdout: with_final_newline(stdout),
        diagnostics,
    })
}

pub(in crate::command) fn read_history_from_reader(
    reader: Option<&dyn StoredSessionReader>,
    session_id: SessionId,
) -> Result<StoredSessionHistory, crate::diagnostic::AppError> {
    let reader = reader.ok_or_else(|| {
        crate::diagnostic::AppError::many([format!("stored Session {session_id} was not found")])
    })?;
    read_stored_session(reader, session_id).map_err(|error| match &error {
        StoredSessionReadError::NotFound { .. } | StoredSessionReadError::Incomplete { .. } => {
            crate::diagnostic::AppError::many([error.to_string()])
        },
        StoredSessionReadError::Repository(_) | StoredSessionReadError::Invalid { .. } => {
            crate::diagnostic::AppError::single("reading stored Session history", error)
        },
    })
}

fn archival_diagnostics(
    session_id: SessionId,
    view: View,
    continuity: StoredSessionContinuity,
    discovery_validation: StoredDiscoveryValidation,
) -> Vec<crate::diagnostic::CliDiagnostic> {
    let mut diagnostics = Vec::new();
    if view == View::Chat && continuity == StoredSessionContinuity::NotObservable {
        diagnostics.push(crate::diagnostic::CliDiagnostic::warning(format!(
            "stored Session {session_id} may omit a volatile suffix; v1 durability continuity is not observable"
        )));
    }
    diagnostics.extend(discovery_diagnostics(session_id, discovery_validation));
    diagnostics
}

pub(in crate::command) fn discovery_diagnostics(
    session_id: SessionId,
    discovery_validation: StoredDiscoveryValidation,
) -> Vec<crate::diagnostic::CliDiagnostic> {
    match discovery_validation {
        StoredDiscoveryValidation::Consistent => Vec::new(),
        StoredDiscoveryValidation::Mismatch(mismatch) => {
            vec![crate::diagnostic::CliDiagnostic::warning(
                discovery_mismatch_diagnostic(session_id, mismatch),
            )]
        },
    }
}

fn discovery_mismatch_diagnostic(
    session_id: SessionId,
    mismatch: StoredDiscoveryMismatch,
) -> String {
    format!("stored Session {session_id} discovery {mismatch}")
}

pub(in crate::command) fn with_final_newline(mut output: String) -> String {
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output
}

#[cfg(test)]
mod tests;
