use std::{fmt, path::Path};

use yo_core::{
    HostWorkspacePath, SessionId, WorkspaceHostId,
    session_repository::{
        ContinuationEligibility, StoredSession, StoredSessionReader,
        read_stored_session_continuation,
    },
};

use super::AppError;
use crate::state::storage;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum LiveSelection {
    #[default]
    New,
    Resume(SessionId),
    Continue,
}

pub(crate) enum LivePreparation {
    New,
    Resume {
        session_id: SessionId,
        failure_selection: LiveSelection,
        storage: storage::LocalReadStorage,
    },
    ReadOnly {
        session_id: SessionId,
        reason: String,
        storage: storage::LocalReadStorage,
    },
}

pub(super) use super::runtime::{ResumeFailureDisposition, ResumeFailureStage};

pub(super) fn classify_launch_failure(
    selection: LiveSelection,
    stage: ResumeFailureStage,
    detail: impl fmt::Display,
) -> ResumeFailureDisposition {
    let selection = match selection {
        LiveSelection::New => super::runtime::LaunchFailureSelection::New,
        LiveSelection::Resume(session_id) => {
            super::runtime::LaunchFailureSelection::Resume(session_id)
        },
        LiveSelection::Continue => super::runtime::LaunchFailureSelection::Continue,
    };
    super::runtime::classify_launch_failure(selection, stage, detail)
}

pub(crate) fn prepare(selection: LiveSelection, cwd: &Path) -> Result<LivePreparation, AppError> {
    let was_continue = selection == LiveSelection::Continue;
    let selection = match selection {
        LiveSelection::New => return Ok(LivePreparation::New),
        LiveSelection::Resume(session_id) => session_id,
        LiveSelection::Continue => select_continue(cwd)?,
    };
    let storage = storage::open_default_reader()
        .map_err(|error| AppError::single("opening read-only local Yo storage", error))?;
    let continuation = storage
        .reader()
        .ok_or_else(|| AppError::many([format!("stored Session {selection} was not found")]))
        .and_then(|reader| {
            read_stored_session_continuation(reader, selection)
                .map_err(|error| AppError::single("validating stored Session continuation", error))
        });
    if let Ok(ref continuation) = continuation
        && storage.workspace_host_id() == Some(continuation.descriptor().workspace_host_id())
    {
        return Ok(LivePreparation::Resume {
            session_id: selection,
            failure_selection: resolved_failure_selection(was_continue, selection),
            storage,
        });
    }
    if was_continue {
        return Err(continuation.err().unwrap_or_else(|| {
            AppError::many([format!(
                "stored Session {selection} belongs to another workspace host"
            )])
        }));
    }
    let reason = continuation.err().map_or_else(
        || "the Session belongs to another workspace host".to_owned(),
        |error| error.to_string(),
    );
    Ok(LivePreparation::ReadOnly {
        session_id: selection,
        reason,
        storage,
    })
}

fn resolved_failure_selection(was_continue: bool, session_id: SessionId) -> LiveSelection {
    if was_continue {
        LiveSelection::Continue
    } else {
        LiveSelection::Resume(session_id)
    }
}

fn select_continue(cwd: &Path) -> Result<SessionId, AppError> {
    let storage = storage::open_default_reader()
        .map_err(|error| AppError::single("opening read-only local Yo storage", error))?;
    let reader = storage.reader().ok_or_else(|| {
        AppError::many(["no resumable Session exists in the current workspace".to_owned()])
    })?;
    let host = storage.workspace_host_id().ok_or_else(|| {
        AppError::many(["the local workspace host identity is unavailable".to_owned()])
    })?;
    let workspace = HostWorkspacePath::normalize_local(cwd)
        .map_err(|error| AppError::single("normalizing the current workspace", error))?;
    select_continue_from(
        reader
            .discover()
            .map_err(|error| AppError::single("discovering stored Sessions", error))?
            .into_iter()
            .filter_map(ContinueCandidate::from_stored),
        host,
        &workspace,
        |candidate| {
            read_stored_session_continuation(reader, candidate.session_id).is_ok_and(
                |continuation| {
                    let descriptor = continuation.descriptor();
                    descriptor.workspace_host_id() == candidate.host
                        && descriptor.workspace_path() == &candidate.workspace
                },
            )
        },
    )
    .ok_or_else(|| {
        AppError::many(["no resumable Session exists in the current workspace".to_owned()])
    })
}

fn select_continue_from(
    sessions: impl IntoIterator<Item = ContinueCandidate>,
    host: WorkspaceHostId,
    workspace: &HostWorkspacePath,
    mut recover_unknown: impl FnMut(&ContinueCandidate) -> bool,
) -> Option<SessionId> {
    sessions
        .into_iter()
        .find(|session| {
            session.host == host
                && &session.workspace == workspace
                && match session.eligibility {
                    ContinuationEligibility::Eligible => true,
                    ContinuationEligibility::Unknown => recover_unknown(session),
                    ContinuationEligibility::Unavailable => false,
                }
        })
        .map(|session| session.session_id)
}

#[derive(Clone)]
struct ContinueCandidate {
    session_id: SessionId,
    eligibility: ContinuationEligibility,
    host: WorkspaceHostId,
    workspace: HostWorkspacePath,
}

impl ContinueCandidate {
    fn from_stored(session: StoredSession) -> Option<Self> {
        let summary = session.summary()?;
        let descriptor = summary.discovery().descriptor();
        Some(Self {
            session_id: session.session_id(),
            eligibility: session.continuation_eligibility(),
            host: descriptor.workspace_host_id(),
            workspace: descriptor.workspace_path().clone(),
        })
    }
}

#[cfg(test)]
mod tests;
