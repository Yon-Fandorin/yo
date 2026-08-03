use std::{fmt, str::FromStr};

use uuid::{Builder, Uuid, Variant, Version};

/// Stable opaque identity used to correlate one immutable submission snapshot.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SubmissionId(Uuid);

/// Failure to generate a random submission identity.
#[derive(Clone, Debug)]
pub enum SubmissionIdGenerationError {
    Entropy(getrandom::Error),
}

/// A parsed identity that is not an RFC-compatible UUIDv4.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubmissionIdError;

impl SubmissionId {
    /// Generates a random identity without hiding operating-system entropy failure.
    pub fn new() -> Result<Self, SubmissionIdGenerationError> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(SubmissionIdGenerationError::Entropy)?;
        Ok(Self(Builder::from_random_bytes(random).into_uuid()))
    }

    /// Admits only RFC-compatible UUIDv4 identities.
    pub fn from_uuid(uuid: Uuid) -> Result<Self, SubmissionIdError> {
        if uuid.get_version() == Some(Version::Random) && uuid.get_variant() == Variant::RFC4122 {
            Ok(Self(uuid))
        } else {
            Err(SubmissionIdError)
        }
    }

    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl fmt::Display for SubmissionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for SubmissionId {
    type Err = SubmissionIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value)
            .map_err(|_| SubmissionIdError)
            .and_then(Self::from_uuid)
    }
}

impl fmt::Display for SubmissionIdGenerationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Entropy(error) => write!(
                formatter,
                "the operating system cannot provide submission identity entropy: {error}"
            ),
        }
    }
}

impl std::error::Error for SubmissionIdGenerationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Entropy(error) => Some(error),
        }
    }
}

impl fmt::Display for SubmissionIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("submission identity must be a UUIDv4")
    }
}

impl std::error::Error for SubmissionIdError {}
