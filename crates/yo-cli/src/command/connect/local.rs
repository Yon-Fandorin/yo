use std::path::Path;

use yo_core::{
    ConnectionOperationExecutionError, ConnectionRepositoryError, HostId, StartupTarget,
};

use super::Command as ConnectCommand;
use crate::{
    AppError, config,
    connection::{self, display_target},
    storage,
};

fn verify_local_host_with_codex_warning_observer(
    host: &HostId,
    warning_observer: Option<yo_backend_delegated_codex::CodexWarningObserver>,
) -> Result<(), AppError> {
    let workspace = std::env::current_dir()
        .map_err(|error| AppError::single("reading the working directory", error))?;
    let _workspace_host_id = storage::open_default_host_identity()
        .map_err(|error| AppError::single("opening the stable workspace Host identity", error))?;
    crate::host::verify_at_with_codex_warning_observer(host, &workspace, warning_observer)
}

pub(super) fn validate_options(command: &ConnectCommand) -> Result<(), AppError> {
    if command.credential_file.is_some() || command.yes {
        Err(AppError::message(
            "--credential-file and --yes are supported only for an external model connection; agent hosts use their own CLI login",
        ))
    } else {
        Ok(())
    }
}

pub(super) fn run(
    config_path: &Path,
    command: ConnectCommand,
    host: HostId,
    warning_observer: Option<yo_backend_delegated_codex::CodexWarningObserver>,
) -> Result<String, AppError> {
    let verification_host = host.clone();
    execute_with_lane(config_path, command, move || {
        verify_local_host_with_codex_warning_observer(&verification_host, warning_observer)
    })
}

pub(super) fn execute_with_lane(
    config_path: &Path,
    command: ConnectCommand,
    verify: impl FnOnce() -> Result<(), AppError>,
) -> Result<String, AppError> {
    let repositories = connection::operation_repositories(config_path)?;
    let mut session = repositories
        .acquire()
        .map_err(|error| AppError::single("acquiring the connection operation lane", error))?;
    session
        .recover_pending_operation()
        .map_err(|error| AppError::single("recovering a pending connection operation", error))?;
    let config = config::load_from(config_path)
        .map_err(|error| AppError::single("reading Yo configuration", error))?;
    let admitted = connection::admit_target(&config, &command.target)?;
    let StartupTarget::Host(host) = &admitted else {
        return Err(AppError::message(
            "local host connect admission did not preserve a HostTarget",
        ));
    };
    let reference = host.reference();
    let snapshot = session
        .capture_connections()
        .map_err(|error| AppError::single("capturing stored connections", error))?;
    let mutation = snapshot
        .preference()
        .is_none()
        .then(|| snapshot.prepare_preference(Some(admitted.clone())))
        .transpose()
        .map_err(|error| AppError::single("preparing the local agent host default", error))?
        .flatten();
    verify()?;
    config
        .verify_unchanged()
        .map_err(|error| AppError::single("guarding Yo configuration", error))?;
    let Some(mutation) = mutation else {
        return Ok(format!(
            "connected: {}; default preserved as {}\n",
            reference,
            display_target(snapshot.preference())
        ));
    };
    match session.commit_connection_mutation(&mutation) {
        Ok(_) => Ok(format!(
            "connected: {}; default: {}\n",
            reference, reference
        )),
        Err(ConnectionOperationExecutionError::PublicCommit(
            ConnectionRepositoryError::Conflict { .. },
        )) => {
            let current = session.capture_connections().map_err(|error| {
                AppError::single("inspecting the concurrent connection winner", error)
            })?;
            if current.preference().is_some() {
                Ok(format!(
                    "connected: {}; default preserved as {}\n",
                    reference,
                    display_target(current.preference())
                ))
            } else {
                Err(AppError::message(
                    "the connection repository changed without publishing a default; retry the local agent host connection",
                ))
            }
        },
        Err(error) => Err(AppError::single(
            "publishing the local agent host default",
            error,
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = connection::canonical_test_temp_dir().join(format!(
                "yo-cli-local-connect-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn config(directory: &TestDirectory) -> PathBuf {
        let path = directory.0.join("config.yaml");
        fs::write(&path, "session: {}\n").unwrap();
        path
    }

    // Local host 검증 실패는 준비된 preference를 게시하지 않고 repository를 absent로 남깁니다.
    #[test]
    fn local_codex_verification_failure_writes_no_preference() {
        let directory = TestDirectory::new();
        let config_path = config(&directory);
        let repository =
            yo_core::LocalConnectionRepository::new(directory.0.join("connections.yaml"));
        let error = execute_with_lane(
            &config_path,
            ConnectCommand {
                from: None,
                target: "host:codex".to_owned(),
                verbose: false,
                credential_file: None,
                yes: false,
            },
            || Err(AppError::message("verification failed")),
        )
        .unwrap_err();
        assert!(error.to_string().contains("verification failed"));
        assert!(!repository.path().exists());
    }

    // 첫 성공 local Grok 연결은 기존 default가 없을 때만 generic HostTarget을 원자적으로
    // 게시합니다.
    #[test]
    fn local_grok_connect_publishes_the_generic_host_target() {
        let directory = TestDirectory::new();
        let config_path = config(&directory);
        let repository =
            yo_core::LocalConnectionRepository::new(directory.0.join("connections.yaml"));
        let output = execute_with_lane(
            &config_path,
            ConnectCommand {
                from: None,
                target: "host:grok".to_owned(),
                verbose: false,
                credential_file: None,
                yes: false,
            },
            || Ok(()),
        )
        .unwrap();
        assert_eq!(output, "connected: host:grok; default: host:grok\n");
        assert_eq!(
            repository.capture().unwrap().preference(),
            Some(&StartupTarget::host_grok())
        );
    }

    // HostTarget은 external credential channel을 소유하지 않으므로 file/yes option을 즉시
    // 거절합니다.
    #[test]
    fn local_codex_rejects_non_interactive_credential_options() {
        let command = ConnectCommand {
            from: None,
            target: "host:codex".to_owned(),
            verbose: false,
            credential_file: Some("/definitely/not/read".into()),
            yes: true,
        };
        let error = validate_options(&command).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("only for an external model connection")
        );
        assert!(
            error
                .to_string()
                .contains("agent hosts use their own CLI login")
        );
    }

    // 별도 preference winner가 먼저 게시되면 stale local HostTarget CAS는 winner를 보존합니다.
    #[test]
    fn local_codex_conflict_preserves_a_concurrent_preference_winner() {
        let directory = TestDirectory::new();
        let config_path = config(&directory);
        let repository =
            yo_core::LocalConnectionRepository::new(directory.0.join("connections.yaml"));
        let racing_repository = repository.clone();
        let winner = StartupTarget::Model(yo_core::ModelSelection::new(
            yo_core::ProviderId::new("qwencloud").unwrap(),
            yo_core::AccountId::new("default").unwrap(),
            yo_core::ModelId::new("winner").unwrap(),
        ));
        let output = execute_with_lane(
            &config_path,
            ConnectCommand {
                from: None,
                target: "host:codex".to_owned(),
                verbose: false,
                credential_file: None,
                yes: false,
            },
            move || {
                let mutation = racing_repository
                    .capture()
                    .unwrap()
                    .prepare_preference(Some(winner))
                    .unwrap()
                    .unwrap();
                racing_repository.commit(&mutation).unwrap();
                Ok(())
            },
        )
        .unwrap();
        assert!(output.contains("default preserved as qwencloud:default:winner"));
        assert!(matches!(
            repository.capture().unwrap().preference(),
            Some(StartupTarget::Model(_))
        ));
    }
}
