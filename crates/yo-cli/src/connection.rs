use std::path::Path;

mod disconnect;
mod external;
mod input;
mod presentation;

use yo_core::{
    CompleteModelBinding, ConnectionOperationExecutionError, ConnectionRepositoryError,
    ConnectionSnapshot, LocalConnectionOperationRepositories, LocalConnectionRepository,
    ModelSelection, StartupPolicy, StartupSelectionSources, StartupTarget, resolve_startup_target,
};

use crate::{
    AppError,
    command::{ConnectCommand, DefaultCommand, DisconnectCommand},
    config::{self, Config},
    storage,
};

pub(crate) fn load_startup_connections(
    config: &mut Config,
) -> Result<Option<StartupTarget>, AppError> {
    let snapshot = repository(config)
        .capture()
        .map_err(|error| AppError::single("reading managed connections", error))?;
    let preference = snapshot.preference().cloned();
    compose_managed_catalog(config, &snapshot)?;
    Ok(preference)
}

fn compose_managed_catalog(
    config: &mut Config,
    snapshot: &ConnectionSnapshot,
) -> Result<(), AppError> {
    let catalog = snapshot
        .compose_catalog(config.model_catalog())
        .map_err(|error| AppError::single("composing manual and managed model bindings", error))?;
    config.replace_model_catalog(catalog);
    Ok(())
}

pub(crate) fn run_default(command: DefaultCommand) -> Result<String, AppError> {
    let config_path = absolute_config_path(
        config::selected_path()
            .map_err(|error| AppError::single("locating Yo configuration", error))?,
    )?;
    execute_default_managed(&config_path, command)
}

pub(crate) fn run_connect(command: ConnectCommand) -> Result<String, AppError> {
    if command.target == StartupTarget::HOST_CODEX_REFERENCE {
        validate_local_connect_options(&command)?;
    }
    let config_path = absolute_config_path(
        config::selected_path()
            .map_err(|error| AppError::single("locating Yo configuration", error))?,
    )?;
    if command.target != StartupTarget::HOST_CODEX_REFERENCE {
        return external::run_external_connect(&config_path, command);
    }
    execute_local_connect_managed(&config_path, command)
}

fn validate_local_connect_options(command: &ConnectCommand) -> Result<(), AppError> {
    if command.credential_file.is_some() || command.yes {
        Err(AppError::message(
            "--credential-file and --yes are supported only for an external model connection; Local Codex uses no API credential",
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn run_disconnect(command: DisconnectCommand) -> Result<String, AppError> {
    let config_path = absolute_config_path(
        config::selected_path()
            .map_err(|error| AppError::single("locating Yo configuration", error))?,
    )?;
    disconnect::run_external_disconnect(&config_path, command)
}

fn absolute_config_path(path: std::path::PathBuf) -> Result<std::path::PathBuf, AppError> {
    if path.is_absolute() {
        return Ok(path);
    }
    std::env::current_dir()
        .map(|directory| directory.join(path))
        .map_err(|error| AppError::single("resolving the Yo configuration path", error))
}

fn operation_repositories(
    config_path: &Path,
) -> Result<LocalConnectionOperationRepositories, AppError> {
    let directory = config_path.parent().ok_or_else(|| {
        AppError::message("Yo configuration must have an absolute parent directory")
    })?;
    LocalConnectionOperationRepositories::in_directory(directory)
        .map_err(|error| AppError::single("opening connection repositories", error))
}

fn execute_default_managed(
    config_path: &Path,
    command: DefaultCommand,
) -> Result<String, AppError> {
    execute_default_managed_with(config_path, command, || Ok(()))
}

fn execute_default_managed_with(
    config_path: &Path,
    command: DefaultCommand,
    before_guard: impl FnOnce() -> Result<(), AppError>,
) -> Result<String, AppError> {
    let repositories = operation_repositories(config_path)?;
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
        .map_err(|error| AppError::single("capturing managed connections", error))?;
    compose_managed_catalog(&mut config, &snapshot)?;
    let preference = command
        .target
        .as_deref()
        .map(|reference| admit_target(&config, reference))
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

fn execute_local_connect_managed(
    config_path: &Path,
    command: ConnectCommand,
) -> Result<String, AppError> {
    execute_local_connect_managed_with(config_path, command, verify_local_codex)
}

fn execute_local_connect_managed_with(
    config_path: &Path,
    command: ConnectCommand,
    verify: impl FnOnce() -> Result<(), AppError>,
) -> Result<String, AppError> {
    let repositories = operation_repositories(config_path)?;
    let mut session = repositories
        .acquire()
        .map_err(|error| AppError::single("acquiring the connection operation lane", error))?;
    session
        .recover_pending_operation()
        .map_err(|error| AppError::single("recovering a pending connection operation", error))?;
    let config = config::load_from(config_path)
        .map_err(|error| AppError::single("reading Yo configuration", error))?;
    let admitted = admit_target(&config, &command.target)?;
    if admitted != StartupTarget::HostCodex {
        return Err(AppError::message(
            "Local Codex connect admission did not preserve the exact HostTarget",
        ));
    }
    let snapshot = session
        .capture_connections()
        .map_err(|error| AppError::single("capturing managed connections", error))?;
    let mutation = snapshot
        .preference()
        .is_none()
        .then(|| snapshot.prepare_preference(Some(StartupTarget::HostCodex)))
        .transpose()
        .map_err(|error| AppError::single("preparing the Local Codex default", error))?
        .flatten();
    verify()?;
    config
        .verify_unchanged()
        .map_err(|error| AppError::single("guarding Yo configuration", error))?;
    let Some(mutation) = mutation else {
        return Ok(format!(
            "connected: {}; default preserved as {}\n",
            StartupTarget::HOST_CODEX_REFERENCE,
            display_target(snapshot.preference())
        ));
    };
    match session.commit_connection_mutation(&mutation) {
        Ok(_) => Ok(format!(
            "connected: {}; default: {}\n",
            StartupTarget::HOST_CODEX_REFERENCE,
            StartupTarget::HOST_CODEX_REFERENCE
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
                    StartupTarget::HOST_CODEX_REFERENCE,
                    display_target(current.preference())
                ))
            } else {
                Err(AppError::message(
                    "the connection repository changed without publishing a default; retry Local Codex connect",
                ))
            }
        },
        Err(error) => Err(AppError::single(
            "publishing the Local Codex default",
            error,
        )),
    }
}

fn repository(config: &Config) -> LocalConnectionRepository {
    LocalConnectionRepository::new(config.connection_path())
}

#[cfg(test)]
fn repository_at(config_path: &Path) -> LocalConnectionRepository {
    LocalConnectionRepository::new(
        config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("connections.yaml"),
    )
}

fn admit_target(config: &Config, reference: &str) -> Result<StartupTarget, AppError> {
    resolve_startup_target(
        config.model_catalog(),
        &StartupPolicy::initial(),
        StartupSelectionSources {
            invocation: Some(reference),
            stored_preference: None,
            operator_target: None,
        },
    )
    .map_err(|error| AppError::single("admitting the startup target", error))?
    .ok_or_else(|| AppError::message("target admission returned no startup target"))
}

fn verify_local_codex() -> Result<(), AppError> {
    let workspace = std::env::current_dir()
        .map_err(|error| AppError::single("reading the working directory", error))?;
    verify_local_codex_at(&workspace)
}

fn verify_local_codex_at(workspace: &Path) -> Result<(), AppError> {
    let _workspace_host_id = storage::open_default_host_identity()
        .map_err(|error| AppError::single("opening the stable workspace Host identity", error))?;
    yo_core::CodexBackend::verify(yo_core::CodexBackendConfig::new(workspace))
        .map_err(|error| AppError::single("verifying Local Codex", error))
}

fn display_target(target: Option<&StartupTarget>) -> String {
    match target {
        None => "unset".to_owned(),
        Some(StartupTarget::HostCodex) => StartupTarget::HOST_CODEX_REFERENCE.to_owned(),
        Some(StartupTarget::Model(selection)) => selection.canonical_reference(),
    }
}

fn selection_for_binding(binding: &yo_core::EffectiveModelBinding) -> ModelSelection {
    ModelSelection::new(
        binding.provider_id().clone(),
        binding.account_id().clone(),
        binding.model_id().clone(),
    )
}

#[cfg(test)]
fn canonical_test_temp_dir() -> std::path::PathBuf {
    std::fs::canonicalize(std::env::temp_dir())
        .expect("the connection test temp directory must resolve to its physical path")
}

fn complete_binding_details(complete: &CompleteModelBinding) -> presentation::BindingDetails {
    presentation::BindingDetails::from(complete)
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
            let path = canonical_test_temp_dir().join(format!(
                "yo-cli-connection-{}-{name}-{nonce}",
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

    fn empty_config(directory: &TestDirectory) -> PathBuf {
        directory.config_path("version: 1\n")
    }

    // explicit default는 admitted HostTarget을 한 CAS로 저장하고 같은 명령 재실행은 revision을
    // 바꾸지 않으며, --unset에 해당하는 새 authorization만 다음 CAS로 preference를 지웁니다.
    #[test]
    fn explicit_default_is_idempotent_and_clear_is_a_new_cas() {
        let directory = TestDirectory::new("default");
        let config_path = empty_config(&directory);
        let repository = repository_at(&config_path);
        let command = DefaultCommand {
            target: Some("host:codex".to_owned()),
        };

        execute_default_managed_with(&config_path, command.clone(), || Ok(())).unwrap();
        let first = repository.capture().unwrap();
        execute_default_managed_with(&config_path, command, || Ok(())).unwrap();
        assert_eq!(repository.capture().unwrap().revision(), first.revision());

        execute_default_managed_with(&config_path, DefaultCommand { target: None }, || Ok(()))
            .unwrap();
        assert!(repository.capture().unwrap().preference().is_none());
        assert!(!directory.0.join("connection-operation.yaml").exists());
    }

    // target admission 뒤 config.yaml bytes가 바뀌면 final guard가 CAS 전에 실패하고 absent
    // connections.yaml을 만들지 않아 stale catalog 선택이 공개 상태로 남지 않습니다.
    #[test]
    fn changed_config_aborts_before_default_publication() {
        let directory = TestDirectory::new("config-guard");
        let config_path = empty_config(&directory);
        let repository = repository_at(&config_path);

        let error = execute_default_managed_with(
            &config_path,
            DefaultCommand {
                target: Some("host:codex".to_owned()),
            },
            || {
                fs::write(&config_path, "version: 1\ntui:\n  max_fps: 60\n").unwrap();
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

    // explicit default도 live startup과 같은 captured managed catalog를 사용해야 manual
    // config에 없는 managed-only ModelTarget을 admit하고 그대로 preference로 게시합니다.
    #[test]
    fn default_admits_a_managed_only_model_from_the_captured_snapshot() {
        let directory = TestDirectory::new("default-managed-only");
        let config_path = empty_config(&directory);
        let repository = repository_at(&config_path);
        let (account, binding) = managed_fixture("managed", "medium");
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_managed_upsert(account, binding)
            .unwrap()
            .unwrap();
        repository.commit(&mutation).unwrap();
        let host_default = repository
            .capture()
            .unwrap()
            .prepare_preference(Some(StartupTarget::HostCodex))
            .unwrap()
            .unwrap();
        repository.commit(&host_default).unwrap();
        let before_revision = repository.capture().unwrap().revision().clone();

        let output = execute_default_managed_with(
            &config_path,
            DefaultCommand {
                target: Some("qwencloud:default:managed".to_owned()),
            },
            || Ok(()),
        )
        .unwrap();

        assert_eq!(output, "default: qwencloud:default:managed\n");
        let after = repository.capture().unwrap();
        assert_eq!(
            display_target(after.preference()),
            "qwencloud:default:managed"
        );
        assert_ne!(after.revision(), &before_revision);
    }

    // 같은 coordinate의 manual legacy binding과 managed explicit binding이 다르면 default
    // admission 전에 BindingConflict로 중단하고 기존 connections.yaml bytes를 건드리지 않습니다.
    #[test]
    fn default_binding_conflict_preserves_the_captured_repository() {
        let directory = TestDirectory::new("default-binding-conflict");
        let config_path = directory.config_path(
            "version: 1\nmodel:\n  catalog:\n    - provider: qwencloud\n      account: default\n      model: managed\n      api_dialect: openai-responses\n      base_url: https://example.test/v1\n      input_token_limit: 1000\n      max_output_tokens: 100\n      tokenizer_profile: utf8-bytes/v1\n",
        );
        let repository = repository_at(&config_path);
        let (account, binding) = managed_fixture("managed", "medium");
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_managed_upsert(account, binding)
            .unwrap()
            .unwrap();
        repository.commit(&mutation).unwrap();
        let before = fs::read(repository.path()).unwrap();

        let error = execute_default_managed_with(
            &config_path,
            DefaultCommand {
                target: Some("qwencloud:default:managed".to_owned()),
            },
            || Ok(()),
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("composing manual and managed model bindings")
        );
        assert_eq!(fs::read(repository.path()).unwrap(), before);
    }

    // Local Codex 검증이 실패하면 준비된 preference bytes가 있어도 CAS를 호출하지 않고
    // repository는 unset으로 남아 실패를 성공한 첫 연결처럼 기억하지 않습니다.
    #[test]
    fn local_codex_verification_failure_writes_no_preference() {
        let directory = TestDirectory::new("verification-failure");
        let config_path = empty_config(&directory);
        let repository = repository_at(&config_path);

        let error = execute_local_connect_managed_with(
            &config_path,
            ConnectCommand {
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

    // Local Codex는 credential source가 없는 HostTarget이므로 비대화형 파일 option을
    // config나 파일 경로에 접근하기 전에 거절하고 외부 연결 의미로 재해석하지 않습니다.
    #[test]
    fn local_codex_rejects_non_interactive_credential_options() {
        let command = ConnectCommand {
            target: "host:codex".to_owned(),
            verbose: false,
            credential_file: Some("/definitely/not/read".into()),
            yes: true,
        };

        let error = validate_local_connect_options(&command)
            .unwrap_err()
            .to_string();

        assert!(error.contains("only for an external model connection"));
        assert!(error.contains("Local Codex uses no API credential"));
    }

    // pending journal과 malformed config가 함께 있어도 recovery failure가 먼저 반환되어야
    // 새 command의 config parse가 기존 recoverable operation 해결을 가리지 않습니다.
    #[test]
    fn pending_recovery_precedes_new_command_configuration_capture() {
        let directory = TestDirectory::new("recovery-before-config");
        let config_path = directory.config_path("not valid: [");
        fs::write(directory.0.join("connection-operation.yaml"), "pending\n").unwrap();

        let error = execute_default_managed_with(
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

    // Local Codex 검증 중 별도 CAS가 model default를 먼저 게시하면 stale HostTarget CAS는
    // conflict 뒤 현재 승자를 다시 읽고 보존하여 first-success retry가 기본값을 덮지 않습니다.
    #[test]
    fn local_codex_conflict_preserves_a_concurrent_preference_winner() {
        let directory = TestDirectory::new("concurrent-winner");
        let config_path = directory.config_path(
            "version: 1\nmodel:\n  catalog:\n    - provider: qwencloud\n      account: default\n      model: winner\n      api_dialect: openai-responses\n      base_url: https://example.test/v1\n      input_token_limit: 1000\n      max_output_tokens: 100\n      tokenizer_profile: utf8-bytes/v1\n",
        );
        let config = config::load_from(&config_path).unwrap();
        let repository = repository_at(&config_path);
        let racing_repository = repository.clone();
        let winner = admit_target(&config, "qwencloud:default:winner").unwrap();

        let output = execute_local_connect_managed_with(
            &config_path,
            ConnectCommand {
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

    // live startup은 connections.yaml의 typed managed binding을 manual catalog와 합친 뒤
    // preference와 같은 snapshot에서 읽어 managed-only target도 즉시 선택할 수 있습니다.
    #[test]
    fn startup_load_composes_managed_catalog_and_preference_from_one_snapshot() {
        let directory = TestDirectory::new("startup-managed-catalog");
        let config_path = empty_config(&directory);
        let repository = repository_at(&config_path);
        let (account, binding) = managed_fixture("managed", "medium");
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_managed_upsert(account, binding)
            .unwrap()
            .unwrap();
        repository.commit(&mutation).unwrap();

        let mut config = config::load_from(&config_path).unwrap();
        let preference = load_startup_connections(&mut config).unwrap();

        assert!(matches!(preference, Some(StartupTarget::Model(_))));
        let entry = config
            .model_catalog()
            .resolve_model(
                &yo_core::ProviderId::new("qwencloud").unwrap(),
                &yo_core::AccountId::new("default").unwrap(),
                &yo_core::ModelId::new("managed").unwrap(),
            )
            .unwrap();
        assert_eq!(entry.provenance(), yo_core::ModelCatalogProvenance::Managed);
        assert_eq!(entry.model_display_name(), Some("Model managed"));
    }

    fn managed_fixture(
        model: &str,
        effort: &str,
    ) -> (
        yo_core::ManagedConnectionAccount,
        yo_core::ManagedConnectionBinding,
    ) {
        let durable = format!(
            r#"{{"provider":"qwencloud","account":"default","model":"{model}","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{{"effort":"{effort}"}},"optional_request_parameters":{{}},"tool_capability_policy":"local-tools/v1","verification_profile":"semantic-terminal/v1"}}"#,
        );
        let complete = yo_core::CompleteModelBinding::from_durable_json(&durable).unwrap();
        let account = yo_core::ManagedConnectionAccount::new(
            yo_core::ProviderId::new("qwencloud").unwrap(),
            yo_core::AccountId::new("default").unwrap(),
            Some("QwenCloud".to_owned()),
            Some("Default".to_owned()),
        )
        .unwrap();
        let binding =
            yo_core::ManagedConnectionBinding::new(complete, Some(format!("Model {model}")))
                .unwrap();
        (account, binding)
    }

    // 사용자에게 돌려주는 ModelTarget은 Provider와 Account의 예약 문자를 canonical
    // uppercase escape로 표시해 출력값을 다음 exact 명령에 그대로 재사용할 수 있어야 합니다.
    #[test]
    fn model_target_output_uses_the_shared_canonical_reference() {
        let target = StartupTarget::Model(yo_core::ModelSelection::new(
            yo_core::ProviderId::new("vendor:edge").unwrap(),
            yo_core::AccountId::new("team%blue").unwrap(),
            yo_core::ModelId::new("model:latest/v1").unwrap(),
        ));

        assert_eq!(
            display_target(Some(&target)),
            "vendor%3Aedge:team%25blue:model:latest/v1"
        );
    }
}
