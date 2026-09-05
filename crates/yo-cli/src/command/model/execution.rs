use std::path::Path;

use yo_core::StartupTarget;

use crate::{
    AppError,
    command::ModelCommand,
    config,
    connection::{self, presentation},
};

pub(crate) fn run(command: ModelCommand) -> Result<String, AppError> {
    let config_path = connection::absolute_config_path(
        config::selected_path()
            .map_err(|error| AppError::single("locating Yo configuration", error))?,
    )?;
    execute(&config_path, command)
}

fn execute(config_path: &Path, command: ModelCommand) -> Result<String, AppError> {
    let repositories = connection::operation_repositories(config_path)?;
    let mut session = repositories
        .acquire()
        .map_err(|error| AppError::single("acquiring the connection operation lane", error))?;
    session
        .recover_pending_operation()
        .map_err(|error| AppError::single("recovering a pending connection operation", error))?;
    let config = config::load_from(config_path)
        .map_err(|error| AppError::single("reading Yo configuration", error))?;
    let snapshot = session
        .capture_connections()
        .map_err(|error| AppError::single("capturing stored connections", error))?;
    let catalog = snapshot
        .model_catalog()
        .map_err(|error| AppError::single("building the stored model catalog", error))?;
    let current = snapshot
        .preference()
        .and_then(StartupTarget::model)
        .cloned();
    let selection = yo_core::ModelSelectionController::new(catalog, current)
        .resolve_reference_for_activation(&command.target)
        .map_err(|error| AppError::single("resolving stored model activation target", error))?;
    let mutation = snapshot
        .prepare_model_activation(&selection, command.enabled)
        .map_err(|error| AppError::single("preparing stored model activation", error))?;
    config
        .verify_unchanged()
        .map_err(|error| AppError::single("guarding Yo configuration", error))?;
    if let Some(mutation) = mutation {
        session
            .commit_connection_mutation(&mutation)
            .map_err(|error| AppError::single("publishing stored model activation", error))?;
    }
    Ok(format!(
        "model: {}; status: {}\n",
        presentation::escape_remote_text(&selection.canonical_reference()),
        if command.enabled {
            "enabled"
        } else {
            "disabled"
        }
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
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = connection::canonical_test_temp_dir()
                .join(format!("yo-cli-model-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn stored_fixture() -> (yo_core::ConnectionAccount, yo_core::StoredModelBinding) {
        let complete = yo_core::CompleteModelBinding::from_durable_json(
            r#"{"provider":"qwencloud","account":"default","model":"stored","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{"effort":"medium"},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1"}"#,
        ).unwrap();
        let account = yo_core::ConnectionAccount::new(
            yo_core::ProviderId::new("qwencloud").unwrap(),
            yo_core::AccountId::new("default").unwrap(),
            Some("QwenCloud".to_owned()),
            Some("Default".to_owned()),
        )
        .unwrap();
        let binding =
            yo_core::StoredModelBinding::new(complete, Some("Stored".to_owned())).unwrap();
        (account, binding)
    }

    // model activation은 credential bytes를 건드리지 않고 같은 상태 재실행을 idempotent하게
    // 처리합니다.
    #[test]
    fn activation_is_idempotent_and_preserves_credential_bytes() {
        let directory = TestDirectory::new();
        let config_path = directory.0.join("config.yaml");
        fs::write(&config_path, "session: {}\n").unwrap();
        let repository =
            yo_core::LocalConnectionRepository::new(directory.0.join("connections.yaml"));
        let (account, binding) = stored_fixture();
        let provider = account.provider_id().clone();
        let account_id = account.account_id().clone();
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_model_upsert(account, binding)
            .unwrap()
            .unwrap();
        repository.commit(&mutation).unwrap();
        let credentials =
            yo_core::LocalCredentialRepository::new(directory.0.join("credentials.yaml"));
        let mutation =
            yo_core::CredentialRepository::prepare_set(&credentials, &provider, &account_id)
                .unwrap();
        let credential = yo_core::ApiCredential::new("unchanged-secret").unwrap();
        yo_core::CredentialRepository::commit(&credentials, &mutation, Some(&credential)).unwrap();
        let bytes = fs::read(credentials.path()).unwrap();
        let command = ModelCommand {
            target: "qwencloud:default:stored".to_owned(),
            enabled: false,
        };
        assert_eq!(
            execute(&config_path, command.clone()).unwrap(),
            "model: qwencloud:default:stored; status: disabled\n"
        );
        let revision = repository.capture().unwrap().revision().clone();
        execute(&config_path, command).unwrap();
        assert_eq!(repository.capture().unwrap().revision(), &revision);
        assert_eq!(fs::read(credentials.path()).unwrap(), bytes);
    }
}
