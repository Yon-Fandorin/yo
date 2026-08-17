use std::path::Path;

use yo_core::{
    AccountId, CompleteModelBinding, ConnectionSnapshot, ManagedConnectionAccount,
    ManagedConnectionBinding, ModelCatalog, ModelCatalogEntry, ModelSelection,
    PreparedConnectionMutation, ProviderId, StartupPolicy, StartupSelectionSources, StartupTarget,
    discover_kimi_models, discover_openrouter_models, resolve_startup_target,
};

use super::{
    complete_binding_details, display_target,
    input::{
        AuthorizedCredentialFileInput, ExternalConnectInput, ModelPickerItem, TtyConnectionInput,
    },
    operation_repositories,
    presentation::{Confirmation, ConnectPreview, ManagedConnectionChange, connect_success},
    selection_for_binding,
};
use crate::{AppError, command::ConnectCommand, config};

pub(super) fn run_external_connect(
    config_path: &Path,
    command: ConnectCommand,
) -> Result<String, AppError> {
    if catalog_pair(&command.target)?.is_some()
        && (command.credential_file.is_some() || command.yes)
    {
        return Err(AppError::message(
            "Provider:Account model selection is interactive only and rejects --credential-file and --yes",
        ));
    }
    match (
        command.credential_file.clone(),
        command.yes,
        command.verbose,
    ) {
        (Some(path), true, false) => {
            let mut input = AuthorizedCredentialFileInput::new(path);
            execute_external_connect_with(config_path, command, &mut input)
        },
        (None, false, _) => {
            let mut input = TtyConnectionInput::new();
            execute_external_connect_with(config_path, command, &mut input)
        },
        _ => Err(AppError::message(
            "non-interactive external connect requires --credential-file and --yes together, without --verbose",
        )),
    }
}

fn execute_external_connect_with(
    config_path: &Path,
    command: ConnectCommand,
    input: &mut impl ExternalConnectInput,
) -> Result<String, AppError> {
    execute_external_connect_with_discovery(
        config_path,
        command,
        input,
        |seed, candidate, input| {
            let models = discover_openrouter_models(seed, candidate).map_err(|error| {
                AppError::single("discovering OpenRouter account models", error)
            })?;
            if models.is_empty() {
                return Err(AppError::message(
                    "OpenRouter discovery returned no valid ModelId",
                ));
            }
            let choices = models
                .iter()
                .map(ModelPickerItem::from_openrouter)
                .collect::<Vec<_>>();
            let Some(selected) = input.select_model(&choices)? else {
                return Ok(None);
            };
            models
                .get(selected)
                .and_then(|model| model.entry().cloned())
                .map(Some)
                .ok_or_else(|| {
                    AppError::message(
                        "the OpenRouter picker returned an invalid or disabled model selection",
                    )
                })
        },
        |session, config, prepared, candidate, discovered| {
            config
                .verify_unchanged()
                .map_err(|error| AppError::single("guarding Yo configuration", error))?;
            session
                .commit_external_connection(prepared, candidate)
                .map_err(|error| {
                    safe_discovery_source("publishing the external connection", error, discovered)
                })
        },
    )
}

fn execute_external_connect_with_discovery<I>(
    config_path: &Path,
    command: ConnectCommand,
    input: &mut I,
    discover_and_select: impl FnOnce(
        &yo_core::OpenRouterDiscoverySeed,
        &yo_core::ApiCredential,
        &mut I,
    ) -> Result<Option<ModelCatalogEntry>, AppError>,
    finalize: impl FnOnce(
        &mut yo_core::LocalConnectionOperationSession<'_>,
        &config::Config,
        yo_core::PreparedExternalConnection,
        yo_core::ApiCredential,
        bool,
    ) -> Result<(), AppError>,
) -> Result<String, AppError>
where
    I: ExternalConnectInput,
{
    execute_external_connect_with_catalogs(
        config_path,
        command,
        input,
        discover_and_select,
        |seed, candidate, input| {
            let models = discover_kimi_models(seed, candidate)
                .map_err(|error| AppError::single("discovering Kimi account models", error))?;
            if models.is_empty() {
                return Err(AppError::message("Kimi catalog returned no valid ModelId"));
            }
            let choices = models
                .iter()
                .map(ModelPickerItem::from_kimi)
                .collect::<Vec<_>>();
            let Some(selected) = input.select_model(&choices)? else {
                return Ok(None);
            };
            models
                .get(selected)
                .and_then(|model| model.entry().cloned())
                .map(Some)
                .ok_or_else(|| {
                    AppError::message(
                        "the Kimi picker returned an invalid or disabled model selection",
                    )
                })
        },
        finalize,
    )
}

fn execute_external_connect_with_catalogs<I>(
    config_path: &Path,
    command: ConnectCommand,
    input: &mut I,
    discover_openrouter_and_select: impl FnOnce(
        &yo_core::OpenRouterDiscoverySeed,
        &yo_core::ApiCredential,
        &mut I,
    ) -> Result<Option<ModelCatalogEntry>, AppError>,
    discover_kimi_and_select: impl FnOnce(
        &yo_core::KimiCatalogSeed,
        &yo_core::ApiCredential,
        &mut I,
    ) -> Result<Option<ModelCatalogEntry>, AppError>,
    finalize: impl FnOnce(
        &mut yo_core::LocalConnectionOperationSession<'_>,
        &config::Config,
        yo_core::PreparedExternalConnection,
        yo_core::ApiCredential,
        bool,
    ) -> Result<(), AppError>,
) -> Result<String, AppError>
where
    I: ExternalConnectInput,
{
    let repositories = operation_repositories(config_path)?;
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
        .map_err(|error| AppError::single("capturing managed connections", error))?;
    let (selected, preselected_candidate, remote_selected) = match catalog_pair(&command.target)? {
        Some((provider, account)) if provider.as_str() == "openrouter" => {
            let seed = config
                .openrouter_discovery_seed(&provider, &account)
                .ok_or_else(|| {
                    AppError::message(format!(
                        "OpenRouter discovery target {} is not an exact configured Provider:Account seed with a complete base profile",
                        command.target
                    ))
                })?;
            let account_reference = format!("{provider}:{account}");
            let candidate = input.read_credential(&account_reference)?;
            let Some(entry) = discover_openrouter_and_select(seed, &candidate, input)? else {
                return Ok("Connection cancelled; nothing changed.\n".to_owned());
            };
            (entry, Some(candidate), true)
        },
        Some((provider, account)) if provider.as_str() == "kimi" => {
            let seed = config
                .kimi_catalog_seed(&provider, &account)
                .ok_or_else(|| {
                    AppError::message(format!(
                        "Kimi catalog target {} is not an exact configured Provider:Account seed",
                        command.target
                    ))
                })?;
            let account_reference = format!("{provider}:{account}");
            let candidate = input.read_credential(&account_reference)?;
            let Some(entry) = discover_kimi_and_select(seed, &candidate, input)? else {
                return Ok("Connection cancelled; nothing changed.\n".to_owned());
            };
            (entry, Some(candidate), true)
        },
        Some((provider, account)) => {
            let seed = config
                .qwencloud_catalog_seed(&provider, &account)
                .ok_or_else(|| {
                    AppError::message(format!(
                        "QwenCloud catalog target {} is not an exact configured Provider:Account seed",
                        command.target
                    ))
                })?;
            let choices = seed
                .models()
                .iter()
                .map(ModelPickerItem::from_qwencloud)
                .collect::<Vec<_>>();
            let Some(selected) = input.select_model(&choices)? else {
                return Ok("Connection cancelled; nothing changed.\n".to_owned());
            };
            let entry = seed
                .models()
                .get(selected)
                .and_then(|model| model.entry().cloned())
                .ok_or_else(|| {
                    AppError::message(
                        "the QwenCloud picker returned an invalid or disabled model selection",
                    )
                })?;
            let account_reference = format!("{provider}:{account}");
            let candidate = input.read_credential(&account_reference)?;
            (entry, Some(candidate), false)
        },
        None => (
            selected_entry(&snapshot, &config, &command.target)?,
            None,
            false,
        ),
    };
    let selection = selection_for(&selected);
    let startup_policy = StartupPolicy::initial();
    let mut plan = ExternalConnectPlan::prepare(
        &snapshot,
        config.model_catalog(),
        &selection,
        &selected,
        &startup_policy,
    )
    .map_err(|error| safe_discovery_error(error, remote_selected))?;
    if remote_selected {
        plan.escape_remote_model(selection.model().as_str());
    }
    let ExternalConnectPlan {
        connection,
        bindings,
        binding_count,
        preference,
        target,
        account,
        default_after,
        managed_change,
        default_changed,
        binding_details,
    } = plan;
    let prepared = session
        .prepare_external_connection(connection, bindings)
        .map_err(|error| {
            safe_discovery_source("preparing the external connection", error, remote_selected)
        })?;

    let preview = Confirmation::Connect(Box::new(
        ConnectPreview::new(
            target,
            account,
            default_after,
            managed_change,
            prepared.credential_action(),
            default_changed,
            binding_details,
        )
        .with_verbose(command.verbose),
    ));
    if !input.confirm(&preview)? {
        return Ok("Connection cancelled; nothing changed.\n".to_owned());
    }
    let candidate = match preselected_candidate {
        Some(candidate) => candidate,
        None => {
            let account_reference = format!("{}:{}", selection.provider(), selection.account());
            input.read_credential(&account_reference)?
        },
    };
    finalize(&mut session, &config, prepared, candidate, remote_selected)?;

    Ok(connect_success(
        &display_target(Some(&StartupTarget::Model(selection.clone()))),
        binding_count,
        &display_target(preference.as_ref()),
    ))
}

struct ExternalConnectPlan {
    connection: PreparedConnectionMutation,
    bindings: Vec<CompleteModelBinding>,
    binding_count: usize,
    preference: Option<StartupTarget>,
    target: String,
    account: String,
    default_after: String,
    managed_change: ManagedConnectionChange,
    default_changed: bool,
    binding_details: Vec<super::presentation::BindingDetails>,
}

impl ExternalConnectPlan {
    fn escape_remote_model(&mut self, model_id: &str) {
        for details in &mut self.binding_details {
            details.escape_remote_model(model_id);
        }
    }

    fn prepare(
        snapshot: &ConnectionSnapshot,
        manual: &ModelCatalog,
        selection: &ModelSelection,
        selected: &ModelCatalogEntry,
        startup_policy: &StartupPolicy,
    ) -> Result<Self, AppError> {
        let complete = selected.complete_binding().cloned().ok_or_else(|| {
            AppError::message(
                "external connect requires an explicit Provider/Account profile and model binding; migrate this legacy catalog entry to model.bindings",
            )
        })?;
        let mut prospective_bindings = Vec::new();
        for entry in manual
            .entries()
            .iter()
            .filter(|entry| same_account(entry, selection))
        {
            let retained = entry.complete_binding().cloned().ok_or_else(|| {
                AppError::message(format!(
                    "Provider {} Account {} still has a legacy model entry; migrate every retained model to model.bindings before rotating this account key",
                    selection.provider(),
                    selection.account()
                ))
            })?;
            if !prospective_bindings.contains(&retained) {
                prospective_bindings.push(retained);
            }
        }
        for retained in snapshot.managed_bindings().iter().filter(|retained| {
            let binding = retained.complete().binding();
            binding.provider_id() == selection.provider()
                && binding.account_id() == selection.account()
                && retained.selection() != *selection
        }) {
            let complete = retained.complete().clone();
            if !prospective_bindings.contains(&complete) {
                prospective_bindings.push(complete);
            }
        }
        if !prospective_bindings.contains(&complete) {
            prospective_bindings.push(complete.clone());
        }

        let account = ManagedConnectionAccount::new(
            selection.provider().clone(),
            selection.account().clone(),
            selected.provider_display_name().map(str::to_owned),
            selected.account_display_name().map(str::to_owned),
        )
        .map_err(|error| AppError::single("preparing the managed account", error))?;
        let binding = ManagedConnectionBinding::new(
            complete,
            selected.model_display_name().map(str::to_owned),
        )
        .map_err(|error| AppError::single("preparing the managed model binding", error))?;
        let account_unchanged = snapshot
            .managed_accounts()
            .iter()
            .any(|current| current == &account);
        let managed_change = match snapshot
            .managed_bindings()
            .iter()
            .find(|current| current.selection() == *selection)
        {
            None => ManagedConnectionChange::Create,
            Some(current) if account_unchanged && current == &binding => {
                ManagedConnectionChange::Keep
            },
            Some(_) => ManagedConnectionChange::Update,
        };
        let prospective_catalog = snapshot
            .compose_catalog_after_managed_upsert(manual, account.clone(), binding.clone())
            .map_err(|error| {
                AppError::single("composing the prospective managed connection", error)
            })?;
        admit_external_target(&prospective_catalog, selection, startup_policy)?;
        let connection = snapshot
            .prepare_managed_connect(account, binding)
            .map_err(|error| AppError::single("preparing managed connection state", error))?;
        let preference = connection.preference().cloned();
        let binding_count = prospective_bindings.len();
        let mut presentation_bindings = prospective_bindings.clone();
        if let Some(displaced) = snapshot
            .managed_bindings()
            .iter()
            .find(|current| current.selection() == *selection)
            .map(|current| current.complete().clone())
            .filter(|displaced| !presentation_bindings.contains(displaced))
        {
            presentation_bindings.push(displaced);
        }
        let mut binding_details = presentation_bindings
            .iter()
            .map(complete_binding_details)
            .collect::<Vec<_>>();
        binding_details.sort();
        let default_changed = snapshot.preference() != preference.as_ref();
        let default_after = if !default_changed {
            format!("Keep {}", display_target(preference.as_ref()))
        } else {
            format!(
                "{}  →  {}",
                display_target(snapshot.preference()),
                display_target(preference.as_ref())
            )
        };
        Ok(Self {
            connection,
            bindings: prospective_bindings,
            binding_count,
            preference,
            target: display_target(Some(&StartupTarget::Model(selection.clone()))),
            account: super::presentation::escape_remote_text(&format!(
                "{}:{}",
                selection.provider(),
                selection.account()
            )),
            default_after,
            managed_change,
            default_changed,
            binding_details,
        })
    }

    #[cfg(test)]
    fn preview(
        &self,
        credential_action: yo_core::CredentialMutationAction,
        verbose: bool,
    ) -> Confirmation {
        Confirmation::Connect(Box::new(
            ConnectPreview::new(
                self.target.clone(),
                self.account.clone(),
                self.default_after.clone(),
                self.managed_change,
                credential_action,
                self.default_changed,
                self.binding_details.clone(),
            )
            .with_verbose(verbose),
        ))
    }
}

fn admit_external_target(
    catalog: &ModelCatalog,
    selection: &ModelSelection,
    startup_policy: &StartupPolicy,
) -> Result<(), AppError> {
    let reference = selection.canonical_reference();
    let admitted = resolve_startup_target(
        catalog,
        startup_policy,
        StartupSelectionSources {
            invocation: Some(&reference),
            stored_preference: None,
            operator_target: None,
        },
    )
    .map_err(|error| AppError::single("admitting the external connection target", error))?;
    if admitted == Some(StartupTarget::Model(selection.clone())) {
        Ok(())
    } else {
        Err(AppError::message(
            "external connection policy did not admit the exact requested model target",
        ))
    }
}

fn selected_entry(
    snapshot: &ConnectionSnapshot,
    config: &config::Config,
    reference: &str,
) -> Result<ModelCatalogEntry, AppError> {
    let manual = config.model_catalog();
    if let Some(selected) = manual
        .entries()
        .iter()
        .find(|entry| selection_for(entry).canonical_reference() == reference)
        .cloned()
    {
        return Ok(selected);
    }
    if let Some(model) = config.qwencloud_catalog_model_for_reference(reference) {
        return model.entry().cloned().ok_or_else(|| {
            let reason = match model.availability() {
                yo_core::QwenCloudCatalogAvailability::Enabled => "invalid catalog row",
                yo_core::QwenCloudCatalogAvailability::Disabled(reason) => reason.as_str(),
            };
            AppError::message(format!(
                "QwenCloud catalog model {reference:?} is disabled: {reason}; use an explicit manual binding only when its runtime interface is supported"
            ))
        });
    }
    if reference
        .split_once(':')
        .is_some_and(|(provider, _)| provider == "qwencloud")
    {
        return Err(AppError::message(format!(
            "QwenCloud catalog model {reference:?} is outside the configured catalog; use an explicit manual binding"
        )));
    }
    snapshot
        .compose_catalog(&ModelCatalog::default())
        .map_err(|error| AppError::single("reading the managed model catalog", error))?
        .entries()
        .iter()
        .find(|entry| selection_for(entry).canonical_reference() == reference)
        .cloned()
        .ok_or_else(|| {
            AppError::message(format!(
                "external connect target {reference:?} is not an exact configured or QwenCloud catalog Provider:Account:Model reference; use an explicit manual binding for models outside the selected catalog"
            ))
        })
}

fn catalog_pair(reference: &str) -> Result<Option<(ProviderId, AccountId)>, AppError> {
    let mut segments = reference.split(':');
    let Some(provider) = segments.next() else {
        return Ok(None);
    };
    let Some(account) = segments.next() else {
        return Ok(None);
    };
    if segments.next().is_some() {
        return Ok(None);
    }
    if !matches!(provider, "openrouter" | "qwencloud" | "kimi") {
        return Err(AppError::message(format!(
            "two-part external connect target {reference:?} is unsupported; only configured openrouter:Account or kimi:Account discovery and qwencloud:Account catalog selection are admitted"
        )));
    }
    let provider = ProviderId::new(provider)
        .map_err(|error| AppError::single("reading the discovery ProviderId", error))?;
    let account = AccountId::new(account)
        .map_err(|error| AppError::single("reading the discovery AccountId", error))?;
    Ok(Some((provider, account)))
}

fn safe_discovery_error(error: AppError, discovered: bool) -> AppError {
    if discovered {
        AppError::message(super::presentation::escape_remote_text(&error.to_string()))
    } else {
        error
    }
}

fn safe_discovery_source(
    context: &'static str,
    error: impl std::error::Error,
    discovered: bool,
) -> AppError {
    if discovered {
        AppError::message(format!(
            "{context}: {}",
            super::presentation::escape_remote_text(&error.to_string())
        ))
    } else {
        AppError::single(context, error)
    }
}

fn same_account(entry: &ModelCatalogEntry, selection: &ModelSelection) -> bool {
    let binding = entry.binding();
    binding.provider_id() == selection.provider() && binding.account_id() == selection.account()
}

fn selection_for(entry: &ModelCatalogEntry) -> ModelSelection {
    selection_for_binding(entry.binding())
}

#[cfg(test)]
mod discovery_tests;

#[cfg(test)]
mod kimi_catalog_tests;

#[cfg(test)]
mod qwencloud_catalog_tests;

#[cfg(test)]
mod tests {
    use super::*;

    struct CancelInput {
        summary: Option<String>,
        credential_reads: usize,
    }

    impl ExternalConnectInput for CancelInput {
        fn confirm(&mut self, preview: &Confirmation) -> Result<bool, AppError> {
            self.summary = Some(
                preview
                    .render(super::super::presentation::default_width())
                    .unwrap(),
            );
            Ok(false)
        }

        fn read_credential(&mut self, _: &str) -> Result<yo_core::ApiCredential, AppError> {
            self.credential_reads += 1;
            yo_core::ApiCredential::new("unreachable").map_err(|error| {
                AppError::single("constructing the unreachable test credential", error)
            })
        }
    }

    // 실제 external command 경로에서 exact target의 전체 확인문을 먼저 보여 주고 사용자가
    // 거절하면 credential을 읽거나 세 repository 파일을 만들지 않습니다.
    #[test]
    fn cancelled_command_stops_before_secret_or_repository_mutation() {
        let root = super::super::canonical_test_temp_dir().join(format!(
            "yo-external-cancel-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config_path = root.join("config.yaml");
        std::fs::write(&config_path, explicit_config()).unwrap();
        let mut input = CancelInput {
            summary: None,
            credential_reads: 0,
        };

        let output = execute_external_connect_with(
            &config_path,
            ConnectCommand {
                target: "vendor:team:alpha".to_owned(),
                verbose: false,
                credential_file: None,
                yes: false,
            },
            &mut input,
        )
        .unwrap();

        assert_eq!(output, "Connection cancelled; nothing changed.\n");
        let summary = input.summary.unwrap();
        assert!(summary.contains("vendor:team:alpha"));
        assert!(summary.contains("+ API key\n  Save vendor:team"));
        assert!(!summary.contains("Connection profile"));
        assert_eq!(input.credential_reads, 0);
        for name in [
            "connections.yaml",
            "credentials.yaml",
            "connection-operation.yaml",
        ] {
            assert!(!root.join(name).exists(), "{name} must remain absent");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    // `--yes` 경로는 TTY를 열지 않고 plan 준비 뒤 지정 파일을 읽으며, 안전하지 않은
    // credential 파일은 새 intent나 public/credential repository mutation 전에 실패합니다.
    #[test]
    fn non_interactive_file_failure_stops_before_new_repository_mutation() {
        let root = super::super::canonical_test_temp_dir().join(format!(
            "yo-external-file-failure-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config_path = root.join("config.yaml");
        std::fs::write(&config_path, explicit_config()).unwrap();
        let credential_path = root.join("credential");
        std::fs::write(&credential_path, b"diagnostic-sentinel-secret").unwrap();
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&credential_path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let error = run_external_connect(
            &config_path,
            ConnectCommand {
                target: "vendor:team:alpha".to_owned(),
                verbose: false,
                credential_file: Some(credential_path.clone()),
                yes: true,
            },
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("0400 or 0600"));
        assert!(!error.contains("diagnostic-sentinel-secret"));
        assert_eq!(
            std::fs::read(&credential_path).unwrap(),
            b"diagnostic-sentinel-secret"
        );
        for name in [
            "connections.yaml",
            "credentials.yaml",
            "connection-operation.yaml",
        ] {
            assert!(!root.join(name).exists(), "{name} must remain absent");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    // Parser 밖의 injected caller도 두 option 중 하나만 주거나 `--verbose`를 함께 주어
    // TTY/file 흐름을 우회할 수 없고 config와 repository를 읽기 전에 같은 오류로 닫힙니다.
    #[test]
    fn runtime_rejects_an_invalid_non_interactive_option_combination() {
        let config_path = std::path::Path::new("/not/read/config.yaml");
        for command in [
            ConnectCommand {
                target: "vendor:team:alpha".to_owned(),
                verbose: false,
                credential_file: Some("/not/read/credential".into()),
                yes: false,
            },
            ConnectCommand {
                target: "vendor:team:alpha".to_owned(),
                verbose: false,
                credential_file: None,
                yes: true,
            },
            ConnectCommand {
                target: "vendor:team:alpha".to_owned(),
                verbose: true,
                credential_file: Some("/not/read/credential".into()),
                yes: true,
            },
        ] {
            let error = run_external_connect(config_path, command)
                .unwrap_err()
                .to_string();
            assert!(error.contains("requires --credential-file and --yes together"));
        }
    }

    // 이미 같은 Provider/Account credential이 있으면 repository가 준비한 exact Replace action을
    // confirmation까지 전달해 새 key 추가라고 오해시키지 않으며 취소는 기존 secret을 보존합니다.
    #[test]
    fn cancelled_rotation_discloses_exact_credential_replacement() {
        let root = super::super::canonical_test_temp_dir().join(format!(
            "yo-external-replace-preview-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config_path = root.join("config.yaml");
        std::fs::write(&config_path, explicit_config()).unwrap();
        let credentials = yo_core::LocalCredentialRepository::new(root.join("credentials.yaml"));
        let provider = yo_core::ProviderId::new("vendor").unwrap();
        let account = yo_core::AccountId::new("team").unwrap();
        let mutation =
            yo_core::CredentialRepository::prepare_set(&credentials, &provider, &account).unwrap();
        let original = yo_core::ApiCredential::new("existing-secret").unwrap();
        yo_core::CredentialRepository::commit(&credentials, &mutation, Some(&original)).unwrap();
        let mut input = CancelInput {
            summary: None,
            credential_reads: 0,
        };

        execute_external_connect_with(
            &config_path,
            ConnectCommand {
                target: "vendor:team:alpha".to_owned(),
                verbose: false,
                credential_file: None,
                yes: false,
            },
            &mut input,
        )
        .unwrap();

        assert!(
            input
                .summary
                .unwrap()
                .contains("~ API key\n  Replace vendor:team")
        );
        assert_eq!(
            yo_core::CredentialRepository::capture(&credentials)
                .unwrap()
                .resolve(&provider, &account)
                .unwrap()
                .expose_secret(),
            "existing-secret"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    // 같은 Account에 남는 모든 complete binding을 확인 목록에 포함해야 키 교체 전에
    // 하나라도 누락한 구현을 summary와 binding_count가 함께 구별합니다.
    #[test]
    fn plan_includes_every_retained_binding_for_the_account() {
        let catalog = fixture_catalog();
        let snapshot = fixture_snapshot();
        let selection = ModelSelection::new(
            yo_core::ProviderId::new("vendor").unwrap(),
            yo_core::AccountId::new("team").unwrap(),
            yo_core::ModelId::new("alpha").unwrap(),
        );

        let selected = catalog
            .resolve_model(selection.provider(), selection.account(), selection.model())
            .unwrap();
        let plan = ExternalConnectPlan::prepare(
            &snapshot,
            &catalog,
            &selection,
            selected,
            &StartupPolicy::initial(),
        )
        .unwrap();

        assert_eq!(plan.binding_count, 2);
        let preview = plan
            .preview(yo_core::CredentialMutationAction::Replace, true)
            .render(super::super::presentation::default_width())
            .unwrap();
        assert!(preview.contains("Models (2)"));
        assert!(preview.contains("alpha, beta"));
        assert_eq!(preview.matches("Connection profile").count(), 1);
        assert_eq!(preview.matches("https://example.test/v1").count(), 1);
        let compact = plan
            .preview(yo_core::CredentialMutationAction::Replace, false)
            .render(std::num::NonZeroU16::new(160).unwrap())
            .unwrap();
        let credential_row = compact
            .split("~ API key\n")
            .nth(1)
            .unwrap()
            .split("\n= Default model")
            .next()
            .unwrap();
        assert!(
            credential_row.contains("register 2 models"),
            "credential row: {credential_row:?}"
        );
        assert!(
            credential_row.contains("alpha"),
            "credential row: {credential_row:?}"
        );
        assert!(
            credential_row.contains("beta"),
            "credential row: {credential_row:?}"
        );
        let models_row = credential_row
            .lines()
            .find(|line| line.trim_start().starts_with("Models"))
            .unwrap();
        assert_eq!(models_row.trim(), "Models          alpha, beta");
        assert!(!models_row.contains("vendor:team"));
        assert!(!compact.contains("Connection profile"));
        assert_eq!(plan.preference, Some(StartupTarget::Model(selection)));
    }

    // 같은 Account에 불완전한 binding이 하나라도 남으면 정확한 실행 profile을 admission할 수
    // 없으므로 credential/public mutation 실행 전 planning이 교체 안내로 실패합니다.
    #[test]
    fn plan_rejects_a_retained_legacy_binding() {
        let mut entries = fixture_catalog().entries().to_vec();
        let binding = yo_core::EffectiveModelBinding::new(
            yo_core::ProviderId::new("vendor").unwrap(),
            yo_core::AccountId::new("team").unwrap(),
            yo_core::ModelId::new("legacy").unwrap(),
            yo_core::ApiDialect::OpenAiResponses,
            yo_core::NormalizedEndpoint::parse("https://example.test/v1").unwrap(),
        );
        entries.push(
            yo_core::ModelCatalogEntry::new(
                binding,
                Some("Vendor".to_owned()),
                Some("Team".to_owned()),
                None,
                yo_core::ModelContextProfile::new(1000, 100, "utf8-bytes/v1").unwrap(),
            )
            .unwrap(),
        );
        let catalog = ModelCatalog::new(entries).unwrap();
        let selection = ModelSelection::new(
            yo_core::ProviderId::new("vendor").unwrap(),
            yo_core::AccountId::new("team").unwrap(),
            yo_core::ModelId::new("alpha").unwrap(),
        );

        let selected = catalog
            .resolve_model(selection.provider(), selection.account(), selection.model())
            .unwrap();
        let error = ExternalConnectPlan::prepare(
            &fixture_snapshot(),
            &catalog,
            &selection,
            selected,
            &StartupPolicy::initial(),
        )
        .err()
        .expect("legacy retained binding must fail planning");

        assert!(error.to_string().contains("migrate every retained model"));
    }

    // config의 새 complete profile과 기존 managed profile이 같은 coordinate에서 달라도
    // preview는 둘 다 비교하되 게시·admission 집합은 교체 뒤 새 profile 하나만 셉니다.
    #[test]
    fn plan_discloses_old_and_new_profiles_during_managed_replacement() {
        let catalog = ModelCatalog::new(vec![fixture_entry("alpha")]).unwrap();
        let snapshot = fixture_snapshot_with_managed(fixture_complete_at(
            "alpha",
            "https://old.example.test/v1",
            "openai-chat-completions",
            r#"{"effort":"medium"}"#,
        ));
        let selection = ModelSelection::new(
            yo_core::ProviderId::new("vendor").unwrap(),
            yo_core::AccountId::new("team").unwrap(),
            yo_core::ModelId::new("alpha").unwrap(),
        );
        let selected = catalog
            .resolve_model(selection.provider(), selection.account(), selection.model())
            .unwrap();

        let plan = ExternalConnectPlan::prepare(
            &snapshot,
            &catalog,
            &selection,
            selected,
            &StartupPolicy::initial(),
        )
        .unwrap();

        assert_eq!(plan.binding_count, 1);
        assert_eq!(
            plan.bindings,
            vec![selected.complete_binding().unwrap().clone()]
        );
        let preview = plan
            .preview(yo_core::CredentialMutationAction::Replace, true)
            .render(super::super::presentation::default_width())
            .unwrap();
        assert!(preview.contains("Connection profile 1 of 2"));
        assert!(preview.contains("Connection profile 2 of 2"));
        assert!(preview.matches("Models (1)").count() == 2);
        assert!(preview.contains("https://example.test/v1"));
        assert!(preview.contains("https://old.example.test/v1"));
        assert!(preview.contains("openai-responses"));
        assert!(preview.contains("openai-chat-completions"));
        assert!(preview.contains("~ Managed connection\n  Update vendor:team:alpha"));
        assert!(preview.matches("{}").count() >= 3);
        assert!(preview.contains(r#"{"effort":"medium"}"#));
        let compact = plan
            .preview(yo_core::CredentialMutationAction::Replace, false)
            .render(std::num::NonZeroU16::new(80).unwrap())
            .unwrap();
        assert!(compact.contains("Replace vendor:team · register 1 model"));
        assert!(compact.contains("Models          alpha"));
        assert_eq!(compact.matches("Models          alpha").count(), 1);
    }

    // 더는 runtime이 지원하지 않는 이전 managed profile도 교체 뒤 집합에서는 사라지므로,
    // 새 profile의 구조적 admission을 막거나 성공 모델 수를 부풀리지 않습니다.
    #[test]
    fn replacement_excludes_a_displaced_unsupported_profile_from_admission() {
        let catalog = ModelCatalog::new(vec![fixture_entry("alpha")]).unwrap();
        let snapshot = fixture_snapshot_with_managed(fixture_complete_with_options(
            "alpha",
            "https://old.example.test/v1",
            r#"{"retired":true}"#,
        ));
        let selection = ModelSelection::new(
            yo_core::ProviderId::new("vendor").unwrap(),
            yo_core::AccountId::new("team").unwrap(),
            yo_core::ModelId::new("alpha").unwrap(),
        );
        let selected = catalog
            .resolve_model(selection.provider(), selection.account(), selection.model())
            .unwrap();
        let expected = selected.complete_binding().unwrap().clone();
        let plan = ExternalConnectPlan::prepare(
            &snapshot,
            &catalog,
            &selection,
            selected,
            &StartupPolicy::initial(),
        )
        .unwrap();

        assert_eq!(plan.binding_count, 1);
        assert_eq!(plan.bindings, vec![expected]);
        let preview = plan
            .preview(yo_core::CredentialMutationAction::Replace, true)
            .render(super::super::presentation::default_width())
            .unwrap();
        assert!(preview.contains("Connection profile 1 of 2"));
        assert!(preview.contains("Connection profile 2 of 2"));
        assert!(preview.contains(r#"{"retired":true}"#));

        let root = super::super::canonical_test_temp_dir().join(format!(
            "yo-external-displaced-profile-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let repositories =
            yo_core::LocalConnectionOperationRepositories::in_directory(&root).unwrap();
        let mut session = repositories.acquire().unwrap();
        let prepared = session
            .prepare_external_connection(plan.connection, plan.bindings)
            .unwrap();
        assert_eq!(prepared.binding_count(), 1);
        drop(session);
        std::fs::remove_dir_all(root).unwrap();
    }

    // external connect도 startup target policy를 통과해야 하므로 Host 강제 policy에서는
    // exact external target을 확인하거나 credential을 읽기 전에 planning이 거절됩니다.
    #[test]
    fn plan_enforces_target_policy_before_confirmation() {
        let catalog = fixture_catalog();
        let selection = ModelSelection::new(
            yo_core::ProviderId::new("vendor").unwrap(),
            yo_core::AccountId::new("team").unwrap(),
            yo_core::ModelId::new("alpha").unwrap(),
        );
        let selected = catalog
            .resolve_model(selection.provider(), selection.account(), selection.model())
            .unwrap();
        let enforced_host =
            StartupPolicy::new(false, Some(StartupTarget::HostCodex), None).unwrap();

        let error = ExternalConnectPlan::prepare(
            &fixture_snapshot(),
            &catalog,
            &selection,
            selected,
            &enforced_host,
        )
        .err()
        .expect("the enforced Host policy must reject an external target");

        assert!(
            error
                .to_string()
                .contains("conflicts with the enforced startup policy")
        );
    }

    fn fixture_catalog() -> ModelCatalog {
        ModelCatalog::new(vec![fixture_entry("alpha"), fixture_entry("beta")]).unwrap()
    }

    fn explicit_config() -> &'static str {
        r#"
model:
  bindings:
    - provider: vendor
      account: team
      base_url: https://example.test/v1
      profile:
        api_dialect: openai-responses
        tokenizer_profile: utf8-bytes/v1
        input_token_limit: 1000
        max_output_tokens: 100
        reasoning_parameters: {}
        optional_request_parameters: {}
        tool_capability_policy: local-tools/v1
      models:
        - model: alpha
"#
    }

    fn fixture_entry(model: &str) -> ModelCatalogEntry {
        let complete = fixture_complete(model);
        ModelCatalogEntry::with_explicit_profile(
            complete.binding().clone(),
            Some("Vendor".to_owned()),
            Some("Team".to_owned()),
            Some(model.to_owned()),
            complete.profile().clone(),
        )
        .unwrap()
    }

    fn fixture_complete(model: &str) -> CompleteModelBinding {
        fixture_complete_with_reasoning(model, "{}")
    }

    fn fixture_complete_with_reasoning(model: &str, reasoning: &str) -> CompleteModelBinding {
        fixture_complete_at(
            model,
            "https://example.test/v1",
            "openai-responses",
            reasoning,
        )
    }

    fn fixture_complete_at(
        model: &str,
        endpoint: &str,
        dialect: &str,
        reasoning: &str,
    ) -> CompleteModelBinding {
        CompleteModelBinding::from_durable_json(&format!(
            r#"{{"provider":"vendor","account":"team","model":"{model}","connector":"{dialect}","base_url":"{endpoint}","api_dialect":"{dialect}","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{reasoning},"optional_request_parameters":{{}},"tool_capability_policy":"local-tools/v1"}}"#
        ))
        .unwrap()
    }

    fn fixture_complete_with_options(
        model: &str,
        endpoint: &str,
        options: &str,
    ) -> CompleteModelBinding {
        CompleteModelBinding::from_durable_json(&format!(
            r#"{{"provider":"vendor","account":"team","model":"{model}","connector":"openai-responses","base_url":"{endpoint}","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{{}},"optional_request_parameters":{options},"tool_capability_policy":"local-tools/v1"}}"#
        ))
        .unwrap()
    }

    fn fixture_snapshot_with_managed(complete: CompleteModelBinding) -> ConnectionSnapshot {
        let root = std::env::temp_dir().join(format!(
            "yo-external-plan-managed-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let repository = yo_core::LocalConnectionRepository::new(root.join("connections.yaml"));
        let account = ManagedConnectionAccount::new(
            yo_core::ProviderId::new("vendor").unwrap(),
            yo_core::AccountId::new("team").unwrap(),
            Some("Vendor".to_owned()),
            Some("Team".to_owned()),
        )
        .unwrap();
        let binding = ManagedConnectionBinding::new(complete, Some("alpha".to_owned())).unwrap();
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_managed_connect(account, binding)
            .unwrap();
        repository.commit(&mutation).unwrap();
        let snapshot = repository.capture().unwrap();
        std::fs::remove_dir_all(root).unwrap();
        snapshot
    }

    fn fixture_snapshot() -> ConnectionSnapshot {
        yo_core::LocalConnectionRepository::new(std::env::temp_dir().join(format!(
            "yo-external-plan-{}-missing/connections.yaml",
            std::process::id()
        )))
        .capture()
        .unwrap()
    }
}
