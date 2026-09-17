use std::{error::Error, fmt};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KimiAccountCapacityFailureKind {
    Configuration,
    Transport,
    HttpStatus,
    MediaType,
    Limit,
    Protocol,
    Timeout,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KimiAccountCapacityError {
    kind: KimiAccountCapacityFailureKind,
    message: String,
}

impl KimiAccountCapacityError {
    fn new(kind: KimiAccountCapacityFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    #[must_use]
    pub const fn kind(&self) -> KimiAccountCapacityFailureKind {
        self.kind
    }
}

impl fmt::Display for KimiAccountCapacityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.kind, self.message)
    }
}

impl Error for KimiAccountCapacityError {}

pub(super) fn failure(
    kind: KimiAccountCapacityFailureKind,
    message: impl Into<String>,
) -> KimiAccountCapacityError {
    KimiAccountCapacityError::new(kind, message)
}

pub(super) fn protocol_failure(message: impl Into<String>) -> KimiAccountCapacityError {
    failure(KimiAccountCapacityFailureKind::Protocol, message)
}

pub(super) fn limit_failure(message: impl Into<String>) -> KimiAccountCapacityError {
    failure(KimiAccountCapacityFailureKind::Limit, message)
}

pub(super) fn timeout_failure(message: impl Into<String>) -> KimiAccountCapacityError {
    failure(KimiAccountCapacityFailureKind::Timeout, message)
}
