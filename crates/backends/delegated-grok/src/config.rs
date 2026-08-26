use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    time::Duration,
};

/// Process and compatibility settings for a local Grok Build ACP backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrokBackendConfig {
    executable: OsString,
    working_directory: PathBuf,
    request_timeout: Duration,
    shutdown_timeout: Duration,
    read_only_review: bool,
}

impl GrokBackendConfig {
    pub fn new(working_directory: impl Into<PathBuf>) -> Self {
        Self {
            executable: OsString::from("grok"),
            working_directory: working_directory.into(),
            request_timeout: Duration::from_secs(30),
            shutdown_timeout: Duration::from_secs(2),
            read_only_review: false,
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

    pub(crate) fn process_arguments(&self) -> Vec<&'static str> {
        if self.read_only_review {
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
        }
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
}

#[cfg(test)]
mod tests {
    use super::GrokBackendConfig;

    // Grok 리뷰 프로필은 읽기 도구만 허용하고 권한 질문·하위 agent·웹 검색을 process
    // 시작 시점부터 닫아 ACP event 계층에 도달하기 전에도 같은 제한을 유지합니다.
    #[test]
    fn read_only_review_uses_the_closed_agent_arguments() {
        let config = GrokBackendConfig::new(".").with_read_only_review(true);

        assert_eq!(
            config.process_arguments(),
            [
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
        );
    }

    // 일반 host Session은 기존 Grok ACP 진입 argv를 보존합니다.
    #[test]
    fn standard_profile_preserves_the_existing_agent_arguments() {
        assert_eq!(
            GrokBackendConfig::new(".").process_arguments(),
            ["agent", "stdio"]
        );
    }
}
