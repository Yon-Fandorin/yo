use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    time::Duration,
};

/// Process and compatibility settings for a local Codex app-server backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexBackendConfig {
    executable: OsString,
    working_directory: PathBuf,
    request_timeout: Duration,
    shutdown_timeout: Duration,
}

impl CodexBackendConfig {
    pub fn new(working_directory: impl Into<PathBuf>) -> Self {
        Self {
            executable: OsString::from("codex"),
            working_directory: working_directory.into(),
            request_timeout: Duration::from_secs(30),
            shutdown_timeout: Duration::from_secs(2),
        }
    }

    pub fn executable(&self) -> &OsStr {
        &self.executable
    }

    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    pub fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    pub fn shutdown_timeout(&self) -> Duration {
        self.shutdown_timeout
    }

    pub fn with_executable(mut self, executable: impl Into<OsString>) -> Self {
        self.executable = executable.into();
        self
    }

    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    pub fn with_shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }
}
