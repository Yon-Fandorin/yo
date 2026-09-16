use std::{ffi::OsString, path::PathBuf, time::Duration};

use crate::{BackendFailure, BackendFailureKind};

pub const DEFAULT_MAX_JSONL_MESSAGE_BYTES: usize = 1024 * 1024;

/// Process launch and resource bounds for one stdio JSONL peer.
#[derive(Clone, Debug)]
pub struct StdioJsonlConfig {
    pub(super) executable: PathBuf,
    pub(super) arguments: Vec<OsString>,
    pub(super) working_directory: PathBuf,
    pub(super) process_name: &'static str,
    pub(super) thread_name: &'static str,
    pub(super) shutdown_timeout: Duration,
    pub(super) maximum_message_bytes: usize,
    pub(super) stderr_diagnostic: Option<fn(&str) -> Option<&'static str>>,
}

impl StdioJsonlConfig {
    pub fn new(
        process_name: &'static str,
        thread_name: &'static str,
        executable: impl Into<PathBuf>,
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            executable: executable.into(),
            arguments: Vec::new(),
            working_directory: working_directory.into(),
            process_name,
            thread_name,
            shutdown_timeout: Duration::from_secs(2),
            maximum_message_bytes: DEFAULT_MAX_JSONL_MESSAGE_BYTES,
            stderr_diagnostic: None,
        }
    }

    #[must_use]
    pub fn with_arguments<I, S>(mut self, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.arguments = arguments.into_iter().map(Into::into).collect();
        self
    }

    #[must_use]
    pub const fn with_shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }

    #[must_use]
    pub const fn with_maximum_message_bytes(mut self, bytes: usize) -> Self {
        self.maximum_message_bytes = bytes;
        self
    }

    /// Restricts captured stderr to a backend-owned fixed diagnostic. Returning None
    /// suppresses the raw output. Without this hook, existing stderr behavior is retained.
    #[must_use]
    pub fn with_stderr_diagnostic(mut self, diagnostic: fn(&str) -> Option<&'static str>) -> Self {
        self.stderr_diagnostic = Some(diagnostic);
        self
    }

    pub(super) fn validate(&self) -> Result<(), BackendFailure> {
        if self.process_name.is_empty()
            || self.thread_name.is_empty()
            || self.shutdown_timeout.is_zero()
            || self.maximum_message_bytes == 0
            || self.maximum_message_bytes > usize::MAX - 2
            || !self.working_directory.is_absolute()
        {
            return Err(initialization_failure(
                self.process_name,
                "invalid stdio JSONL process configuration",
            ));
        }
        Ok(())
    }
}

pub(super) fn initialization_failure(
    process_name: &str,
    message: impl Into<String>,
) -> BackendFailure {
    BackendFailure::new(
        BackendFailureKind::Initialization,
        format!("{process_name}: {}", message.into()),
    )
}
