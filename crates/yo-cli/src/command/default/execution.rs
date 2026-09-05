use std::path::Path;

use crate::{
    AppError,
    command::DefaultCommand,
    config,
    connection::{self, display_target},
};

pub(crate) fn run(command: DefaultCommand) -> Result<String, AppError> {
    let config_path = connection::absolute_config_path(
        config::selected_path()
            .map_err(|error| AppError::single("locating Yo configuration", error))?,
    )?;
    execute_with_lane(&config_path, command, || Ok(()))
}

fn load_snapshot_catalog(
    config: &mut config::Config,
    snapshot: &yo_core::ConnectionSnapshot,
) -> Result<(), AppError> {
    let catalog = snapshot
        .model_catalog()
        .map_err(|error| AppError::single("building the stored model catalog", error))?;
    config.replace_model_catalog(catalog);
    Ok(())
}

pub(super) fn execute_with_lane(
    config_path: &Path,
    command: DefaultCommand,
    before_guard: impl FnOnce() -> Result<(), AppError>,
) -> Result<String, AppError> {
    let repositories = connection::operation_repositories(config_path)?;
    let mut session = repositories
        .acquire()
        .map_err(|error| AppError::single("acquiring the connection operation lane", error))?;
    session
        .recover_pending_operation()
        .map_err(|error| AppError::single("recovering a pending connection operation", error))?;
    let mut config = config::load_from(config_path)
        .map_err(|error| AppError::single("reading Yo configuration", error))?;
    let snapshot = session
        .capture_connections()
        .map_err(|error| AppError::single("capturing stored connections", error))?;
    load_snapshot_catalog(&mut config, &snapshot)?;
    let preference = command
        .target
        .as_deref()
        .map(|reference| connection::admit_target(&config, reference))
        .transpose()?;
    let mutation = snapshot
        .prepare_preference(preference.clone())
        .map_err(|error| AppError::single("preparing the startup default", error))?;
    before_guard()?;
    config
        .verify_unchanged()
        .map_err(|error| AppError::single("guarding Yo configuration", error))?;
    if let Some(mutation) = mutation {
        session
            .commit_connection_mutation(&mutation)
            .map_err(|error| AppError::single("publishing the startup default", error))?;
    }
    Ok(format!(
        "default: {}\n",
        display_target(preference.as_ref())
    ))
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
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = connection::canonical_test_temp_dir().join(format!(
                "yo-cli-default-{}-{name}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn config_path(&self, contents: &str) -> PathBuf {
            let path = self.0.join("config.yaml");
            fs::write(&path, contents).unwrap();
            path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // 같은 default를 반복하면 revision을 늘리지 않고 clear만 새로운 preference CAS를 만듭니다.
    #[test]
    fn explicit_default_is_idempotent_and_clear_is_a_new_cas() {
        let directory = TestDirectory::new("idempotent");
        let config_path = directory.config_path("session: {}\n");
        let repository =
            yo_core::LocalConnectionRepository::new(directory.0.join("connections.yaml"));
        let command = DefaultCommand {
            target: Some("host:codex".to_owned()),
        };
        execute_with_lane(&config_path, command.clone(), || Ok(())).unwrap();
        let first = repository.capture().unwrap();
        execute_with_lane(&config_path, command, || Ok(())).unwrap();
        assert_eq!(repository.capture().unwrap().revision(), first.revision());
        execute_with_lane(&config_path, DefaultCommand { target: None }, || Ok(())).unwrap();
        assert!(repository.capture().unwrap().preference().is_none());
    }

    // admission 뒤 config가 바뀌면 final guard가 CAS 전에 실패하여 stale default를 게시하지
    // 않습니다.
    #[test]
    fn changed_config_aborts_before_default_publication() {
        let directory = TestDirectory::new("config-guard");
        let config_path = directory.config_path("session: {}\n");
        let repository =
            yo_core::LocalConnectionRepository::new(directory.0.join("connections.yaml"));
        let error = execute_with_lane(
            &config_path,
            DefaultCommand {
                target: Some("host:codex".to_owned()),
            },
            || {
                fs::write(&config_path, "tui:\n  max_fps: 60\n").unwrap();
                Ok(())
            },
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("changed while this command was preparing")
        );
        assert!(!repository.path().exists());
    }

    // default는 capture한 stored catalog로 config에 없는 durable model target도 admit합니다.
    #[test]
    fn default_admits_a_stored_model_from_the_captured_snapshot() {
        let directory = TestDirectory::new("stored-only");
        let config_path = directory.config_path("session: {}\n");
        let repository =
            yo_core::LocalConnectionRepository::new(directory.0.join("connections.yaml"));
        let complete = yo_core::CompleteModelBinding::from_durable_json(
            r#"{"provider":"qwencloud","account":"default","model":"stored","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{"effort":"medium"},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1"}"#,
        )
        .unwrap();
        let account = yo_core::ConnectionAccount::new(
            yo_core::ProviderId::new("qwencloud").unwrap(),
            yo_core::AccountId::new("default").unwrap(),
            Some("QwenCloud".to_owned()),
            Some("Default".to_owned()),
        )
        .unwrap();
        let binding =
            yo_core::StoredModelBinding::new(complete, Some("Model stored".to_owned())).unwrap();
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_model_upsert(account, binding)
            .unwrap()
            .unwrap();
        repository.commit(&mutation).unwrap();

        let output = execute_with_lane(
            &config_path,
            DefaultCommand {
                target: Some("qwencloud:default:stored".to_owned()),
            },
            || Ok(()),
        )
        .unwrap();
        assert_eq!(output, "default: qwencloud:default:stored\n");
        assert_eq!(
            display_target(repository.capture().unwrap().preference()),
            "qwencloud:default:stored"
        );
    }

    // pending operation recovery failure가 malformed config보다 먼저 반환되어 recovery 계약을
    // 보존합니다.
    #[test]
    fn pending_recovery_precedes_new_command_configuration_capture() {
        let directory = TestDirectory::new("recovery-before-config");
        let config_path = directory.config_path("not valid: [");
        fs::write(directory.0.join("connection-operation.yaml"), "pending\n").unwrap();
        let error = execute_with_lane(
            &config_path,
            DefaultCommand {
                target: Some("host:codex".to_owned()),
            },
            || Ok(()),
        )
        .unwrap_err();
        assert!(error.to_string().contains("pending connection operation"));
        assert!(!error.to_string().contains("invalid configuration"));
    }
}
