use std::{
    fmt,
    num::NonZeroU64,
    str::FromStr,
    time::{SystemTime, SystemTimeError, UNIX_EPOCH},
};

use uuid::{Builder, Uuid, Variant, Version};

macro_rules! identity {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(NonZeroU64);

        impl $name {
            pub const fn new(value: NonZeroU64) -> Self {
                Self(value)
            }

            pub const fn get(self) -> NonZeroU64 {
                self.0
            }
        }

        impl From<NonZeroU64> for $name {
            fn from(value: NonZeroU64) -> Self {
                Self::new(value)
            }
        }
    };
}

identity!(TurnId);
identity!(ActivityId);
identity!(RequestId);

/// Stable identity of one Yo Session.
///
/// Newly created Sessions always use UUIDv7. The legacy numeric form remains
/// representable only so pre-contract Journal records can be read without
/// inventing a false UUID or rewriting durable history.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SessionId(Uuid);

const LEGACY_PREFIX: [u8; 8] = *b"YOLEGACY";

impl SessionId {
    /// Generates a new UUIDv7 Session identity without hiding clock or entropy failure.
    pub fn new() -> Result<Self, SessionIdGenerationError> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(SessionIdGenerationError::Clock)?;
        let millis = timestamp
            .as_secs()
            .saturating_mul(1_000)
            .saturating_add(u64::from(timestamp.subsec_millis()));
        let mut random = [0_u8; 10];
        getrandom::fill(&mut random).map_err(SessionIdGenerationError::Entropy)?;
        Ok(Self(
            Builder::from_unix_timestamp_millis(millis, &random).into_uuid(),
        ))
    }

    /// Admits an externally supplied UUID only when it is a UUIDv7.
    pub fn from_uuid(uuid: Uuid) -> Result<Self, SessionIdError> {
        if uuid.get_version() == Some(Version::SortRand) && uuid.get_variant() == Variant::RFC4122 {
            Ok(Self(uuid))
        } else {
            Err(SessionIdError)
        }
    }

    /// Returns the public UUID of a conforming Session.
    pub fn as_uuid(self) -> Option<Uuid> {
        if self.0.get_version() == Some(Version::SortRand)
            && self.0.get_variant() == Variant::RFC4122
        {
            Some(self.0)
        } else {
            None
        }
    }

    pub(crate) fn from_legacy(value: NonZeroU64) -> Self {
        let mut bytes = [0_u8; 16];
        bytes[..8].copy_from_slice(&LEGACY_PREFIX);
        bytes[8..].copy_from_slice(&value.get().to_be_bytes());
        Self(Uuid::from_bytes(bytes))
    }

    pub(crate) fn legacy_value(self) -> Option<NonZeroU64> {
        let bytes = self.0.as_bytes();
        if bytes[..8] != LEGACY_PREFIX {
            return None;
        }
        let mut value = [0_u8; 8];
        value.copy_from_slice(&bytes[8..]);
        NonZeroU64::new(u64::from_be_bytes(value))
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.legacy_value() {
            Some(value) => write!(formatter, "legacy:{value}"),
            None => self.0.fmt(formatter),
        }
    }
}

impl FromStr for SessionId {
    type Err = SessionIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value)
            .map_err(|_| SessionIdError)
            .and_then(Self::from_uuid)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionIdError;

impl fmt::Display for SessionIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Session identity must be a UUIDv7")
    }
}

impl std::error::Error for SessionIdError {}

#[derive(Clone, Debug)]
pub enum SessionIdGenerationError {
    Clock(SystemTimeError),
    Entropy(getrandom::Error),
}

impl fmt::Display for SessionIdGenerationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Clock(error) => write!(
                formatter,
                "the system clock cannot produce UUIDv7 time: {error}"
            ),
            Self::Entropy(error) => write!(
                formatter,
                "the operating system cannot provide UUIDv7 entropy: {error}"
            ),
        }
    }
}

impl std::error::Error for SessionIdGenerationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Clock(error) => Some(error),
            Self::Entropy(error) => Some(error),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TurnRef {
    session_id: SessionId,
    turn_id: TurnId,
}

impl TurnRef {
    pub const fn new(session_id: SessionId, turn_id: TurnId) -> Self {
        Self {
            session_id,
            turn_id,
        }
    }

    pub const fn session_id(self) -> SessionId {
        self.session_id
    }

    pub const fn turn_id(self) -> TurnId {
        self.turn_id
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ActivityRef {
    turn: TurnRef,
    activity_id: ActivityId,
}

impl ActivityRef {
    pub const fn new(turn: TurnRef, activity_id: ActivityId) -> Self {
        Self { turn, activity_id }
    }

    pub const fn turn(self) -> TurnRef {
        self.turn
    }

    pub const fn session_id(self) -> SessionId {
        self.turn.session_id()
    }

    pub const fn turn_id(self) -> TurnId {
        self.turn.turn_id()
    }

    pub const fn activity_id(self) -> ActivityId {
        self.activity_id
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ActivityRequestRef {
    activity: ActivityRef,
    request_id: RequestId,
}

impl ActivityRequestRef {
    pub const fn new(activity: ActivityRef, request_id: RequestId) -> Self {
        Self {
            activity,
            request_id,
        }
    }

    pub const fn activity(self) -> ActivityRef {
        self.activity
    }

    pub const fn request_id(self) -> RequestId {
        self.request_id
    }
}
