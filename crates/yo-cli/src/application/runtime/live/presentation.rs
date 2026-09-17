use yo_core::{
    HostWorkspacePath,
    session_repository::{SessionForkLimits, SessionTreeLimits, StoredSessionReader},
};

use super::super::LiveSession;
use crate::{
    command,
    interaction::diagnostic::AppError,
    state::{config, storage},
};

pub(super) fn show_fork_picker(current: &mut LiveSession) -> Result<(), AppError> {
    current.fork_catalog = None;
    let storage = storage::open_default_reader()
        .map_err(|error| AppError::single("reading historical fork points", error))?;
    let reader = storage
        .reader()
        .ok_or_else(|| AppError::message("the current Session has no saved context to fork"))?;
    let catalog = current
        .agent
        .capture_fork_catalog(reader, SessionForkLimits::default())
        .map_err(|error| AppError::single("capturing historical fork points", error))?;
    let picker = current
        .tui
        .show_fork_picker(&catalog)
        .map_err(AppError::message)?;
    current.fork_catalog = Some((picker, catalog));
    Ok(())
}

pub(super) fn show_session_tree(current: &mut LiveSession) -> Result<(), AppError> {
    let storage = storage::open_default_reader()
        .map_err(|error| AppError::single("reading saved sessions", error))?;
    let reader = storage
        .reader()
        .ok_or_else(|| AppError::message("no saved sessions exist"))?;
    let workspace_host = storage
        .workspace_host_id()
        .ok_or_else(|| AppError::message("saved workspace identity is unavailable"))?;
    let workspace = HostWorkspacePath::normalize_local(&current.workspace)
        .map_err(|error| AppError::single("normalizing the current workspace", error))?;
    let tree = reader
        .read_tree(workspace_host, &workspace, SessionTreeLimits::default())
        .map_err(|error| AppError::single("reading the session tree", error))?;
    current
        .tui
        .show_session_tree(&tree, current.session_id)
        .map_err(AppError::message)
}

pub(super) fn show_resume_picker(
    current: &mut LiveSession,
    config: &config::Config,
) -> Result<(), AppError> {
    use yo_tui::ResumeSessionEntry;
    let storage = storage::open_default_reader()
        .map_err(|error| AppError::single("reading saved sessions", error))?;
    let reader = storage
        .reader()
        .ok_or_else(|| AppError::message("no saved sessions exist"))?;
    let workspace = HostWorkspacePath::normalize_local(&current.workspace)
        .map_err(|error| AppError::single("normalizing the current workspace", error))?;
    let sessions = reader
        .discover()
        .map_err(|error| AppError::single("discovering saved sessions", error))?;
    let date_formatter = config.date_formatter().ok();
    let mut entries = Vec::new();
    let mut has_more = false;
    for saved in sessions {
        let Some(summary) = saved.summary() else {
            continue;
        };
        let descriptor = summary.discovery().descriptor();
        if saved.session_id() == current.session_id
            || Some(descriptor.workspace_host_id()) != storage.workspace_host_id()
            || descriptor.workspace_path() != &workspace
        {
            continue;
        }
        if entries.len() == 64 {
            has_more = true;
            break;
        }
        entries.push(ResumeSessionEntry {
            session_id: saved.session_id(),
            eligibility: saved.continuation_eligibility(),
            updated_label: date_formatter
                .as_ref()
                .and_then(|formatter| {
                    formatter
                        .format_unix_millis(summary.discovery().updated_unix_millis())
                        .ok()
                })
                .unwrap_or_else(|| "unknown update time".to_owned()),
        });
    }
    if entries.is_empty() {
        return Err(AppError::message(
            "no other saved sessions exist in this workspace",
        ));
    }
    current
        .tui
        .show_resume_picker(entries, has_more)
        .map_err(AppError::message)
}

pub(super) fn read_only_resume_output(
    reader: Option<&dyn StoredSessionReader>,
    session_id: yo_core::SessionId,
    glyph_profile: yo_tui::GlyphProfile,
    reason: &str,
) -> Result<command::SessionOutput, AppError> {
    let reader = reader
        .ok_or_else(|| AppError::many([format!("stored Session {session_id} was not found")]))?;
    command::read_only_resume_from(reader, session_id, glyph_profile, reason)
}
