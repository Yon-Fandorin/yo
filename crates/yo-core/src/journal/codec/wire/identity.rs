use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

use super::JournalCodecError;
use crate::{ActivityId, ActivityRef, ActivityRequestRef, RequestId, SessionId, TurnId, TurnRef};

#[derive(Clone, Copy, Deserialize, Serialize)]
pub(super) struct WireTurnRef {
    pub(super) session_id: WireSessionId,
    pub(super) turn_id: u64,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(untagged)]
pub(super) enum WireSessionId {
    Uuid(uuid::Uuid),
    Legacy(u64),
}

#[derive(Clone, Copy, Deserialize, Serialize)]
pub(super) struct WireActivityRef {
    pub(super) turn: WireTurnRef,
    pub(super) activity_id: u64,
}

#[derive(Deserialize, Serialize)]
pub(super) struct WireActivityRequestRef {
    pub(super) activity: WireActivityRef,
    pub(super) request_id: u64,
}

impl From<TurnRef> for WireTurnRef {
    fn from(turn: TurnRef) -> Self {
        Self {
            session_id: WireSessionId::from(turn.session_id()),
            turn_id: turn.turn_id().get().get(),
        }
    }
}

impl TryFrom<WireTurnRef> for TurnRef {
    type Error = JournalCodecError;

    fn try_from(turn: WireTurnRef) -> Result<Self, Self::Error> {
        Ok(Self::new(
            session_id_from(turn.session_id, "Turn Session")?,
            TurnId::new(non_zero(turn.turn_id, "Turn")?),
        ))
    }
}

impl From<ActivityRef> for WireActivityRef {
    fn from(activity: ActivityRef) -> Self {
        Self {
            turn: WireTurnRef::from(activity.turn()),
            activity_id: activity.activity_id().get().get(),
        }
    }
}

impl TryFrom<WireActivityRef> for ActivityRef {
    type Error = JournalCodecError;

    fn try_from(activity: WireActivityRef) -> Result<Self, Self::Error> {
        Ok(Self::new(
            TurnRef::try_from(activity.turn)?,
            ActivityId::new(non_zero(activity.activity_id, "Activity")?),
        ))
    }
}

impl From<ActivityRequestRef> for WireActivityRequestRef {
    fn from(request: ActivityRequestRef) -> Self {
        Self {
            activity: WireActivityRef::from(request.activity()),
            request_id: request.request_id().get().get(),
        }
    }
}

impl TryFrom<WireActivityRequestRef> for ActivityRequestRef {
    type Error = JournalCodecError;

    fn try_from(request: WireActivityRequestRef) -> Result<Self, Self::Error> {
        Ok(Self::new(
            ActivityRef::try_from(request.activity)?,
            request_id_from(request.request_id)?,
        ))
    }
}

impl From<SessionId> for WireSessionId {
    fn from(value: SessionId) -> Self {
        match value.as_uuid() {
            Some(uuid) => Self::Uuid(uuid),
            None => Self::Legacy(
                value
                    .legacy_value()
                    .expect("a Session identity has one representation")
                    .get(),
            ),
        }
    }
}

pub(super) fn session_id_from(
    value: WireSessionId,
    name: &str,
) -> Result<SessionId, JournalCodecError> {
    match value {
        WireSessionId::Uuid(uuid) => SessionId::from_uuid(uuid)
            .map_err(|_| JournalCodecError::new(format!("{name} identity must be a UUIDv7"))),
        WireSessionId::Legacy(value) => Ok(SessionId::from_legacy(non_zero(value, name)?)),
    }
}

pub(super) fn request_id_from(value: u64) -> Result<RequestId, JournalCodecError> {
    Ok(RequestId::new(non_zero(value, "Request")?))
}

fn non_zero(value: u64, name: &str) -> Result<NonZeroU64, JournalCodecError> {
    NonZeroU64::new(value)
        .ok_or_else(|| JournalCodecError::new(format!("{name} identity must be non-zero")))
}
