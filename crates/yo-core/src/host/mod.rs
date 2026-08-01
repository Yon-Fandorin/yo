mod local;
mod workspace;

use std::{fmt, str::FromStr};

pub use local::{LocalWorkspaceHostIdentity, LocalWorkspaceHostIdentityError};
use uuid::{Builder, Uuid, Variant, Version};
pub use workspace::{HostWorkspacePath, HostWorkspacePathError};

/// Stable opaque identity of one per-user Yo Host installation.
///
/// This identity is random and deliberately carries no machine, user, hostname,
/// path, or repository information.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorkspaceHostId(Uuid);

impl WorkspaceHostId {
    /// Generates a random RFC-compatible UUIDv4 without hiding entropy failure.
    pub fn new() -> Result<Self, WorkspaceHostIdGenerationError> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(WorkspaceHostIdGenerationError::Entropy)?;
        Ok(Self(Builder::from_random_bytes(random).into_uuid()))
    }

    /// Admits an externally supplied UUID only when it is an RFC-compatible UUIDv4.
    pub fn from_uuid(uuid: Uuid) -> Result<Self, WorkspaceHostIdError> {
        if uuid.get_version() == Some(Version::Random) && uuid.get_variant() == Variant::RFC4122 {
            Ok(Self(uuid))
        } else {
            Err(WorkspaceHostIdError)
        }
    }

    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl fmt::Display for WorkspaceHostId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for WorkspaceHostId {
    type Err = WorkspaceHostIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value)
            .map_err(|_| WorkspaceHostIdError)
            .and_then(Self::from_uuid)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkspaceHostIdError;

impl fmt::Display for WorkspaceHostIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Workspace Host identity must be a UUIDv4")
    }
}

impl std::error::Error for WorkspaceHostIdError {}

#[derive(Clone, Debug)]
pub enum WorkspaceHostIdGenerationError {
    Entropy(getrandom::Error),
}

impl fmt::Display for WorkspaceHostIdGenerationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Entropy(error) => write!(
                formatter,
                "the operating system cannot provide Workspace Host identity entropy: {error}"
            ),
        }
    }
}

impl std::error::Error for WorkspaceHostIdGenerationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Entropy(error) => Some(error),
        }
    }
}

#[cfg(test)]
mod tests;
