use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

use crate::{ActivityId, ActivityRef, ActivityRequestRef, RequestId, SessionId, TurnId, TurnRef};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireTurn {
    session_id: String,
    turn_id: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireActivity {
    turn: WireTurn,
    activity_id: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRequest {
    activity: WireActivity,
    request_id: u64,
}

impl From<TurnRef> for WireTurn {
    fn from(value: TurnRef) -> Self {
        Self {
            session_id: value.session_id().to_string(),
            turn_id: value.turn_id().get().get(),
        }
    }
}
impl WireTurn {
    fn core(self) -> Option<TurnRef> {
        let session = self.session_id.parse::<SessionId>().ok()?;
        if session.to_string() != self.session_id {
            return None;
        }
        Some(TurnRef::new(
            session,
            TurnId::new(NonZeroU64::new(self.turn_id)?),
        ))
    }
}
impl From<ActivityRef> for WireActivity {
    fn from(value: ActivityRef) -> Self {
        Self {
            turn: value.turn().into(),
            activity_id: value.activity_id().get().get(),
        }
    }
}
impl WireActivity {
    fn core(self) -> Option<ActivityRef> {
        Some(ActivityRef::new(
            self.turn.core()?,
            ActivityId::new(NonZeroU64::new(self.activity_id)?),
        ))
    }
}
impl From<ActivityRequestRef> for WireRequest {
    fn from(value: ActivityRequestRef) -> Self {
        Self {
            activity: value.activity().into(),
            request_id: value.request_id().get().get(),
        }
    }
}
macro_rules! codec {
    ($module:ident,$core:ty,$wire:ty,$convert:expr) => {
        pub(super) mod $module {
            use super::*;
            pub fn serialize<S: serde::Serializer>(value: &$core, s: S) -> Result<S::Ok, S::Error> {
                <$wire>::from(*value).serialize(s)
            }
            pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<$core, D::Error> {
                let value = <$wire>::deserialize(d)?;
                ($convert)(value)
                    .ok_or_else(|| serde::de::Error::custom("invalid canonical core reference"))
            }
        }
    };
}
codec!(
    request,
    ActivityRequestRef,
    WireRequest,
    |v: WireRequest| Some(ActivityRequestRef::new(
        v.activity.core()?,
        RequestId::new(NonZeroU64::new(v.request_id)?)
    ))
);
codec!(activity, ActivityRef, WireActivity, |v: WireActivity| v
    .core());
codec!(turn, TurnRef, WireTurn, |v: WireTurn| v.core());
