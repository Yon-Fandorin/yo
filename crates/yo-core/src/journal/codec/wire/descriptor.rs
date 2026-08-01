use serde::{Deserialize, Serialize};

use super::{
    JournalCodecError,
    identity::{WireSessionId, session_id_from},
};
use crate::{HostWorkspacePath, SessionDescriptor, SessionStartTime, WorkspaceHostId};

const SCHEMA: &str = "yo.session-descriptor/v1";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireSessionDescriptor {
    schema: String,
    session_id: WireSessionId,
    workspace_host_id: uuid::Uuid,
    workspace_path: WireWorkspacePath,
    start_time_unix_millis: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(
    tag = "encoding",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum WireWorkspacePath {
    Utf8(String),
    UnixBytes(Vec<u8>),
}

impl From<&SessionDescriptor> for WireSessionDescriptor {
    fn from(descriptor: &SessionDescriptor) -> Self {
        let bytes = descriptor.workspace_path().as_unix_bytes();
        let workspace_path = match std::str::from_utf8(bytes) {
            Ok(path) => WireWorkspacePath::Utf8(path.to_owned()),
            Err(_) => WireWorkspacePath::UnixBytes(bytes.to_vec()),
        };
        Self {
            schema: SCHEMA.to_owned(),
            session_id: WireSessionId::from(descriptor.session_id()),
            workspace_host_id: descriptor.workspace_host_id().as_uuid(),
            workspace_path,
            start_time_unix_millis: descriptor.started_at().unix_millis(),
        }
    }
}

impl TryFrom<WireSessionDescriptor> for SessionDescriptor {
    type Error = JournalCodecError;

    fn try_from(descriptor: WireSessionDescriptor) -> Result<Self, Self::Error> {
        if descriptor.schema != SCHEMA {
            return Err(JournalCodecError::new(format!(
                "unsupported Session descriptor schema {:?}",
                descriptor.schema
            )));
        }
        let session_id = session_id_from(descriptor.session_id, "Session descriptor")?;
        let workspace_host_id =
            WorkspaceHostId::from_uuid(descriptor.workspace_host_id).map_err(|_| {
                JournalCodecError::new("Session descriptor Host identity must be a UUIDv4")
            })?;
        let bytes = match descriptor.workspace_path {
            WireWorkspacePath::Utf8(path) => path.into_bytes(),
            WireWorkspacePath::UnixBytes(bytes) => bytes,
        };
        let workspace_path =
            HostWorkspacePath::from_unix_bytes(bytes).map_err(JournalCodecError::new)?;
        let recorded_start = SessionStartTime::from_unix_millis(descriptor.start_time_unix_millis);
        let descriptor =
            SessionDescriptor::for_session(session_id, workspace_host_id, workspace_path);
        if descriptor.started_at() != recorded_start {
            return Err(JournalCodecError::new(
                "Session descriptor start time does not match its UUIDv7 identity",
            ));
        }
        Ok(descriptor)
    }
}
