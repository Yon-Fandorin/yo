use std::{num::NonZeroU16, path::Path};

use yo_core::session_repository::{
    ContinuationEligibility, StoredSession, StoredSessionUnavailableReason,
};
use yo_tui::plain::{
    Column, ColumnBehavior, ContinuationLayout, HeadingStyle, ListSpec, OutputWidth, render_list,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SessionRow {
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
    pub(super) fn from_stored(
        session: StoredSession,
        dates: &crate::state::config::DateFormatter,
    ) -> Result<Self, crate::state::config::ConfigError> {
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

pub(super) fn format_rows(
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

pub(super) fn output_width(
    stdout_is_terminal: bool,
    observed: std::io::Result<NonZeroU16>,
) -> OutputWidth {
    if stdout_is_terminal {
        OutputWidth::Bounded(
            observed.unwrap_or_else(|_| NonZeroU16::new(80).expect("80 is nonzero")),
        )
    } else {
        OutputWidth::Unbounded
    }
}

pub(super) fn heading_style(stdout_is_terminal: bool) -> HeadingStyle {
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

#[cfg(test)]
mod tests;
