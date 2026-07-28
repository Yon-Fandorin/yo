use std::num::NonZeroU64;

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

identity!(SessionId);
identity!(TurnId);
identity!(ActivityId);
identity!(RequestId);

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
