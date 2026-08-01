use std::time::{SystemTime, UNIX_EPOCH};

use super::{SessionId, SessionIdGenerationError};
use crate::{HostWorkspacePath, WorkspaceHostId};

/// Millisecond-resolution wall-clock time recorded when a Session starts.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SessionStartTime(u64);

impl SessionStartTime {
    pub(super) fn now() -> Result<Self, SessionIdGenerationError> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(SessionIdGenerationError::Clock)?;
        Ok(Self(
            timestamp
                .as_secs()
                .saturating_mul(1_000)
                .saturating_add(u64::from(timestamp.subsec_millis())),
        ))
    }

    pub const fn unix_millis(self) -> u64 {
        self.0
    }

    pub(crate) const fn from_unix_millis(value: u64) -> Self {
        Self(value)
    }

    fn from_session_id(session_id: SessionId) -> Self {
        let timestamp = session_id
            .as_uuid()
            .get_timestamp()
            .expect("SessionId admits UUIDv7 identities only");
        let (seconds, nanos) = timestamp.to_unix();
        Self(
            seconds
                .saturating_mul(1_000)
                .saturating_add(u64::from(nanos / 1_000_000)),
        )
    }
}

/// Stable discovery metadata recorded before later Session activity becomes durable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionDescriptor {
    session_id: SessionId,
    workspace_host_id: WorkspaceHostId,
    workspace_path: HostWorkspacePath,
    started_at: SessionStartTime,
}

impl SessionDescriptor {
    /// Creates one UUIDv7 Session and its explicit start time from one clock reading.
    pub fn new(
        workspace_host_id: WorkspaceHostId,
        workspace_path: HostWorkspacePath,
    ) -> Result<Self, SessionIdGenerationError> {
        let started_at = SessionStartTime::now()?;
        let session_id = SessionId::at(started_at)?;
        Ok(Self {
            session_id,
            workspace_host_id,
            workspace_path,
            started_at,
        })
    }

    pub fn for_session(
        session_id: SessionId,
        workspace_host_id: WorkspaceHostId,
        workspace_path: HostWorkspacePath,
    ) -> Self {
        let started_at = SessionStartTime::from_session_id(session_id);
        Self {
            session_id,
            workspace_host_id,
            workspace_path,
            started_at,
        }
    }

    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub const fn workspace_host_id(&self) -> WorkspaceHostId {
        self.workspace_host_id
    }

    pub const fn workspace_path(&self) -> &HostWorkspacePath {
        &self.workspace_path
    }

    pub const fn started_at(&self) -> SessionStartTime {
        self.started_at
    }
}
