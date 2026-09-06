use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    time::Duration,
};

pub const NATIVE_SANDBOX_REVIEW_PROFILE: &str = "grok-native-read-only/v1alpha1";
pub const OUTER_SANDBOX_REVIEW_PROFILE: &str = "yo-bwrap-read-only/v1alpha1";
pub const OUTER_SANDBOX_REVIEW_ENV: &str = "YO_GROK_REVIEW_ISOLATION";
pub const OUTER_SANDBOX_SENTINEL: &str = "/run/yo-grok-review-sandbox";
pub const REVIEW_RUNNER_CAPABILITIES: &[u8] = include_bytes!("../review-runner-capabilities.json");

/// Process and compatibility settings for a local Grok Build ACP backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrokBackendConfig {
    executable: OsString,
    working_directory: PathBuf,
    request_timeout: Duration,
    shutdown_timeout: Duration,
    read_only_review: bool,
    outer_sandboxed_review: bool,
    usage_log_path: Option<PathBuf>,
}

impl GrokBackendConfig {
    pub fn new(working_directory: impl Into<PathBuf>) -> Self {
        Self {
            executable: OsString::from("grok"),
            working_directory: working_directory.into(),
            request_timeout: Duration::from_secs(30),
            shutdown_timeout: Duration::from_secs(2),
            read_only_review: false,
            outer_sandboxed_review: false,
            usage_log_path: default_usage_log_path(),
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

    pub fn read_only_review(&self) -> bool {
        self.read_only_review
    }

    pub(crate) fn outer_sandboxed_review(&self) -> bool {
        self.outer_sandboxed_review
    }

    pub(crate) fn usage_log_path(&self) -> Option<&Path> {
        self.usage_log_path.as_deref()
    }

    pub(crate) fn process_arguments(&self) -> Vec<OsString> {
        let arguments = if self.outer_sandboxed_review {
            vec![
                "--sandbox",
                "off",
                "--permission-mode",
                "dontAsk",
                "--tools",
                "",
                "--no-subagents",
                "--disable-web-search",
                "agent",
                "stdio",
            ]
        } else if self.read_only_review {
            vec![
                "--sandbox",
                "read-only",
                "--permission-mode",
                "dontAsk",
                "--tools",
                "Read,Grep",
                "--no-subagents",
                "--disable-web-search",
                "agent",
                "stdio",
            ]
        } else {
            vec!["agent", "stdio"]
        };
        arguments.into_iter().map(OsString::from).collect()
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

    pub fn with_read_only_review(mut self, enabled: bool) -> Self {
        self.read_only_review = enabled;
        self
    }

    /// Selects the Yo-owned outer isolation used only by the bounded no-tools review runner.
    pub fn with_outer_sandboxed_review(mut self, enabled: bool) -> Self {
        self.outer_sandboxed_review = enabled;
        self
    }

    /// Overrides the bounded unified-log source used for the latest billing snapshot.
    pub fn with_usage_log_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.usage_log_path = Some(path.into());
        self
    }
}

fn default_usage_log_path() -> Option<PathBuf> {
    let root = std::env::var_os("GROK_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".grok")))?;
    root.is_absolute().then(|| root.join("logs/unified.jsonl"))
}

#[cfg(test)]
mod tests;
