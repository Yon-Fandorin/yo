use std::{io::IsTerminal, num::NonZeroU16, path::Path};

use yo_core::{
    HostWorkspacePath, SessionId,
    session_repository::{
        ContinuationEligibility, StoredSession, StoredSessionContinuity, StoredSessionReadError,
        StoredSessionReader, StoredSessionUnavailableReason, read_stored_session,
    },
};
use yo_tui::{
    ArchivedSessionView,
    plain::{
        Column, ColumnBehavior, ContinuationLayout, HeadingStyle, ListSpec, OutputWidth,
        render_list,
    },
    project_archived_session,
};

use super::{
    AppError,
    command::{SessionCommand, SessionView},
};

pub(crate) struct Output {
    pub(crate) stdout: String,
    pub(crate) diagnostics: Vec<String>,
}

pub(crate) fn run(command: SessionCommand) -> Result<Output, AppError> {
    let storage = super::storage::open_default_reader()
        .map_err(|error| AppError::single("opening read-only local Yo storage", error))?;
    match command.session_id {
        Some(session_id) => show(&storage, session_id, command),
        None => list(&storage, command),
    }
}

fn show(
    storage: &super::storage::LocalReadStorage,
    session_id: SessionId,
    command: SessionCommand,
) -> Result<Output, AppError> {
    let reader = storage
        .reader()
        .ok_or_else(|| AppError::many([format!("stored Session {session_id} was not found")]))?;
    let history = read_stored_session(reader, session_id).map_err(|error| match &error {
        StoredSessionReadError::NotFound { .. } | StoredSessionReadError::Incomplete { .. } => {
            AppError::many([error.to_string()])
        },
        StoredSessionReadError::Repository(_) | StoredSessionReadError::Invalid { .. } => {
            AppError::single("reading stored Session history", error)
        },
    })?;
    let view = match command.view {
        SessionView::Chat => ArchivedSessionView::Chat,
        SessionView::Transcript => ArchivedSessionView::Transcript,
    };
    let stdout = project_archived_session(&history, view, command.glyph_profile)
        .map_err(|error| AppError::single("projecting stored Session history", error))?;
    let diagnostics = archival_diagnostics(
        session_id,
        command.view,
        history.continuity(),
        history.discovery_consistent(),
    );
    Ok(Output {
        stdout: with_final_newline(stdout),
        diagnostics,
    })
}

fn archival_diagnostics(
    session_id: SessionId,
    view: SessionView,
    continuity: StoredSessionContinuity,
    discovery_consistent: bool,
) -> Vec<String> {
    let mut diagnostics = Vec::new();
    if view == SessionView::Chat && continuity == StoredSessionContinuity::NotObservable {
        diagnostics.push(format!(
            "stored Session {session_id} may omit a volatile suffix; v1 durability continuity is not observable"
        ));
    }
    if !discovery_consistent {
        diagnostics.push(format!(
            "stored Session {session_id} discovery metadata disagrees with its Journal descriptor"
        ));
    }
    diagnostics
}

fn list(
    storage: &super::storage::LocalReadStorage,
    command: SessionCommand,
) -> Result<Output, AppError> {
    let config = super::config::load()
        .map_err(|error| AppError::single("loading Yo configuration", error))?;
    let date_formatter = config
        .date_formatter()
        .map_err(|error| AppError::single("validating the Session date format", error))?;
    let Some(reader) = storage.reader() else {
        return Ok(Output {
            stdout: String::new(),
            diagnostics: Vec::new(),
        });
    };
    let workspace = if command.all {
        None
    } else {
        let cwd = std::env::current_dir()
            .map_err(|error| AppError::single("reading the working directory", error))?;
        Some(
            HostWorkspacePath::normalize_local(cwd)
                .map_err(|error| AppError::single("normalizing the current workspace", error))?,
        )
    };
    let sessions = reader
        .discover()
        .map_err(|error| AppError::single("discovering stored Sessions", error))?;
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
        .map(|session| SessionRow::from_stored(session, &date_formatter))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| AppError::single("formatting stored Session dates", error))?;
    let stdout_is_terminal = std::io::stdout().is_terminal();
    Ok(Output {
        stdout: format_rows(
            &rows,
            command.all,
            command.details,
            output_width(stdout_is_terminal, yo_tui::terminal::current_width()),
            heading_style(stdout_is_terminal),
        )
        .map_err(|error| AppError::single("formatting the Session list", error))?,
        diagnostics: Vec::new(),
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SessionRow {
    resume: String,
    status: String,
    workspace: String,
    updated: String,
    started: String,
    version: String,
    continuation: String,
    path: String,
    detail: String,
}

impl SessionRow {
    fn from_stored(
        session: StoredSession,
        dates: &super::config::DateFormatter,
    ) -> Result<Self, super::config::ConfigError> {
        let resume = session.session_id().to_string();
        let continuation = eligibility_text(session.continuation_eligibility()).to_owned();
        Ok(match session {
            StoredSession::Available(summary) => {
                let discovery = summary.discovery();
                let descriptor = discovery.descriptor();
                let path = descriptor.workspace_path().to_string();
                Self {
                    resume,
                    status: "available".to_owned(),
                    workspace: workspace_label(&path),
                    updated: dates.format_unix_millis(discovery.updated_unix_millis())?,
                    started: dates.format_unix_millis(descriptor.started_at().unix_millis())?,
                    version: "v1".to_owned(),
                    continuation,
                    path,
                    detail: String::new(),
                }
            },
            StoredSession::Unavailable { reason, .. } => Self {
                resume,
                status: unavailable_status(&reason).to_owned(),
                workspace: "?".to_owned(),
                updated: "?".to_owned(),
                started: "?".to_owned(),
                version: "?".to_owned(),
                continuation,
                path: "?".to_owned(),
                detail: terminal_safe(&reason.to_string()),
            },
        })
    }
}

fn format_rows(
    rows: &[SessionRow],
    all: bool,
    details: bool,
    width: OutputWidth,
    heading_style: HeadingStyle,
) -> Result<String, yo_tui::plain::ListError> {
    if rows.is_empty() {
        return Ok(String::new());
    }
    let mut columns = vec![
        Column {
            heading: "RESUME",
            behavior: ColumnBehavior::Pinned,
        },
        Column {
            heading: "STATUS",
            behavior: ColumnBehavior::Pinned,
        },
    ];
    if all {
        columns.push(Column {
            heading: "WORKSPACE",
            behavior: ColumnBehavior::Collapsible {
                priority: 4,
                continuation: ContinuationLayout::Flow,
            },
        });
    }
    columns.extend([
        Column {
            heading: "UPDATED",
            behavior: ColumnBehavior::Pinned,
        },
        Column {
            heading: "STARTED",
            behavior: ColumnBehavior::Collapsible {
                priority: 3,
                continuation: ContinuationLayout::Flow,
            },
        },
    ]);
    if details {
        columns.extend([
            Column {
                heading: "VERSION",
                behavior: ColumnBehavior::Collapsible {
                    priority: 2,
                    continuation: ContinuationLayout::Flow,
                },
            },
            Column {
                heading: "CONTINUATION",
                behavior: ColumnBehavior::Collapsible {
                    priority: 2,
                    continuation: ContinuationLayout::Flow,
                },
            },
            Column {
                heading: "PATH",
                behavior: ColumnBehavior::Collapsible {
                    priority: 1,
                    continuation: ContinuationLayout::Block,
                },
            },
            Column {
                heading: "DETAIL",
                behavior: ColumnBehavior::Collapsible {
                    priority: 1,
                    continuation: ContinuationLayout::Block,
                },
            },
        ]);
    }
    let data = rows
        .iter()
        .map(|row| {
            let mut values = vec![row.resume.clone(), row.status.clone()];
            if all {
                values.push(row.workspace.clone());
            }
            values.extend([row.updated.clone(), row.started.clone()]);
            if details {
                values.extend([
                    row.version.clone(),
                    row.continuation.clone(),
                    row.path.clone(),
                    row.detail.clone(),
                ]);
            }
            values
        })
        .collect::<Vec<_>>();
    render_list(
        ListSpec {
            columns: &columns,
            gap: NonZeroU16::new(2).expect("the Session list gap is nonzero"),
            heading_style,
        },
        &data,
        width,
    )
}

fn output_width(stdout_is_terminal: bool, observed: std::io::Result<NonZeroU16>) -> OutputWidth {
    if stdout_is_terminal {
        OutputWidth::Bounded(
            observed.unwrap_or_else(|_| NonZeroU16::new(80).expect("80 is nonzero")),
        )
    } else {
        OutputWidth::Unbounded
    }
}

fn heading_style(stdout_is_terminal: bool) -> HeadingStyle {
    if stdout_is_terminal {
        HeadingStyle::BoldAnsi
    } else {
        HeadingStyle::Plain
    }
}

fn terminal_safe(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control() {
            output.extend(character.escape_default());
        } else {
            output.push(character);
        }
    }
    output
}

fn eligibility_text(value: ContinuationEligibility) -> &'static str {
    match value {
        ContinuationEligibility::Eligible => "eligible",
        ContinuationEligibility::Unavailable => "unavailable",
        ContinuationEligibility::Unknown => "unknown",
    }
}

fn unavailable_status(reason: &StoredSessionUnavailableReason) -> &'static str {
    match reason {
        StoredSessionUnavailableReason::UnsupportedSchema { .. } => "unsupported",
        StoredSessionUnavailableReason::Quarantined { .. } => "quarantined",
        StoredSessionUnavailableReason::Corrupt { .. } => "corrupt",
        StoredSessionUnavailableReason::Unreadable { .. } => "unreadable",
        StoredSessionUnavailableReason::NoCompleteEnvelope => "incomplete",
    }
}

fn workspace_label(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(path)
        .to_owned()
}

fn with_final_newline(mut output: String) -> String {
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output
}

#[cfg(test)]
mod tests;
