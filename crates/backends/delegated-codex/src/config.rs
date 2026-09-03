use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    time::Duration,
};

use yo_core::{AccountId, ModelId};

/// Process and compatibility settings for a local Codex app-server backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexBackendConfig {
    executable: OsString,
    working_directory: PathBuf,
    request_timeout: Duration,
    shutdown_timeout: Duration,
    read_only_review: bool,
    model_rebind_target: Option<(AccountId, ModelId)>,
}

impl CodexBackendConfig {
    pub fn new(working_directory: impl Into<PathBuf>) -> Self {
        Self {
            executable: OsString::from("codex"),
            working_directory: working_directory.into(),
            request_timeout: Duration::from_secs(30),
            shutdown_timeout: Duration::from_secs(2),
            read_only_review: false,
            model_rebind_target: None,
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

    pub fn model_rebind_target(&self) -> Option<(&AccountId, &ModelId)> {
        self.model_rebind_target
            .as_ref()
            .map(|(account, model)| (account, model))
    }

    pub(crate) fn process_arguments(&self) -> Vec<&'static str> {
        if self.read_only_review {
            vec![
                "app-server",
                "-c",
                "web_search=\"disabled\"",
                "--strict-config",
                "--listen",
                "stdio://",
            ]
        } else {
            vec!["app-server", "--listen", "stdio://"]
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

    pub fn with_model_rebind_target(mut self, account: AccountId, model: ModelId) -> Self {
        self.model_rebind_target = Some((account, model));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::CodexBackendConfig;

    // 제한 프로필은 app-server 자체 설정에서도 웹 검색을 끄고 strict parsing을 사용해,
    // backend RPC 정책보다 앞선 process 경계가 조용히 완화되지 않게 합니다.
    #[test]
    fn read_only_review_uses_the_closed_app_server_arguments() {
        let config = CodexBackendConfig::new(".").with_read_only_review(true);

        assert_eq!(
            config.process_arguments(),
            [
                "app-server",
                "-c",
                "web_search=\"disabled\"",
                "--strict-config",
                "--listen",
                "stdio://",
            ]
        );
    }

    // 일반 대화형 host 경로는 기존 argv를 그대로 보존해 리뷰 프로필 추가가 제품 기본
    // Session의 실행 정책을 바꾸지 않도록 합니다.
    #[test]
    fn standard_profile_preserves_the_existing_app_server_arguments() {
        assert_eq!(
            CodexBackendConfig::new(".").process_arguments(),
            ["app-server", "--listen", "stdio://"]
        );
    }
}
