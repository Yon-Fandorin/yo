use std::path::Path;

use yo_core::{
    AccountId, CompleteModelBinding, ConnectionAccount, ConnectionCatalogSeed, ConnectionSnapshot,
    ModelCatalog, ModelCatalogEntry, ModelSelection, PreparedConnectionMutation, ProviderId,
    StartupPolicy, StartupSelectionSources, StartupTarget, StoredModelBinding,
    discover_kimi_models, discover_openrouter_models, resolve_startup_target,
};

use super::{
    Command as ConnectCommand,
    import::{self, ImportedDefinition},
    presentation::{
        Confirmation, ConnectPreview, ImportPreview, StoredConnectionChange, connect_success,
        import_success,
    },
};
use crate::{
    AppError,
    command::connect::{
        input::{AuthorizedCredentialFileInput, ExternalConnectInput},
        picker::ModelPickerItem,
    },
    interaction::prompt::TtyPrompt,
    state::{
        config,
        connection::{
            complete_binding_details, display_target, operation_repositories, selection_for_binding,
        },
    },
};

pub(super) fn run_external_connect(
    config_path: &Path,
    command: ConnectCommand,
) -> Result<String, AppError> {
    if looks_like_two_part_target(&command.target)
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
            let mut input = TtyPrompt::new();
            execute_external_connect_with(config_path, command, &mut input)
        },
        _ => Err(AppError::message(
            "non-interactive external connect requires --credential-file and --yes together, without --verbose",
        )),
    }
}

pub(super) fn run_definition_import(
    config_path: &Path,
    command: ConnectCommand,
) -> Result<String, AppError> {
    if !command.target.is_empty() {
        return Err(AppError::message(
            "--from cannot be combined with an exact connection target",
        ));
    }
    let source = command
        .from
        .as_deref()
        .ok_or_else(|| AppError::message("definition import requires --from"))?;
    if let Some(path) = command.credential_file.as_deref()
        && !path.is_absolute()
    {
        return Err(AppError::message(
            "definition import requires an absolute --credential-file path",
        ));
    }
    let input_mode = match (
        command.credential_file.clone(),
        command.yes,
        command.verbose,
    ) {
        (Some(path), true, false) => Some(path),
        (None, false, _) => None,
        _ => {
            return Err(AppError::message(
                "non-interactive definition import requires absolute --credential-file and --yes together, without --verbose",
            ));
        },
    };
    let definition = import::read(source)?;
    match input_mode {
        Some(path) => {
            let mut input = AuthorizedCredentialFileInput::new(path);
            execute_definition_import_with(config_path, command, definition, &mut input)
        },
        None => {
            let mut input = TtyPrompt::new();
            execute_definition_import_with(config_path, command, definition, &mut input)
        },
    }
}

fn execute_definition_import_with(
    config_path: &Path,
    command: ConnectCommand,
    definition: ImportedDefinition,
    input: &mut impl ExternalConnectInput,
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
    let snapshot = session
        .capture_connections()
        .map_err(|error| AppError::single("capturing stored connections", error))?;

    let provider = definition.provider().clone();
    let account_id = definition.account_id().clone();
    let account_reference = definition.account.canonical_reference();
    let definition_kind = if definition.catalog_seed.is_some() {
        "Replace the complete catalog or discovery seed for this account".to_owned()
    } else {
        format!(
            "Replace the complete explicit set with {} {}",
            definition.bindings.len(),
            if definition.bindings.len() == 1 {
                "model"
            } else {
                "models"
            }
        )
    };
    let changes = definition_changes(
        &snapshot,
        &definition.account,
        &definition.bindings,
        definition.catalog_seed.as_ref(),
    );
    let details = definition
        .bindings
        .iter()
        .map(|binding| complete_binding_details(binding.complete()))
        .collect::<Vec<_>>();
    let complete_bindings = definition
        .bindings
        .iter()
        .map(|binding| binding.complete().clone())
        .collect::<Vec<_>>();
    let mutation = snapshot
        .prepare_group_replace(
            definition.account,
            definition.bindings,
            definition.catalog_seed,
        )
        .map_err(|error| AppError::single("preparing the grouped definition replacement", error))?;
    let preference_after = mutation.preference().cloned();
    let default_changed = snapshot.preference() != preference_after.as_ref();
    let default_after = if default_changed {
        format!(
            "{}  →  {}",
            display_target(snapshot.preference()),
            display_target(preference_after.as_ref())
        )
    } else {
        format!("Keep {}", display_target(preference_after.as_ref()))
    };
    let prepared = session
        .prepare_external_definition(mutation, &provider, &account_id, complete_bindings)
        .map_err(|error| {
            AppError::single("structurally admitting the grouped definition", error)
        })?;
    let preview = Confirmation::Import(Box::new(ImportPreview::new(
        account_reference.clone(),
        changes.added,
        changes.changed,
        changes.removed,
        changes.definition_changed,
        changes.account_transition,
        changes.account_changed,
        changes.seed_transition,
        changes.seed_changed,
        changes.resume_risk,
        definition_kind,
        prepared.credential_action(),
        default_after,
        default_changed,
        details,
        command.verbose,
    )));
    if !input.confirm(&preview)? {
        return Ok("Connection import cancelled; nothing changed.\n".to_owned());
    }
    let credential = input.read_credential(&account_reference)?;
    config
        .verify_unchanged()
        .map_err(|error| AppError::single("guarding Yo configuration", error))?;
    let registered = prepared.binding_count();
    let success = import_success(
        &account_reference,
        registered,
        &display_target(preference_after.as_ref()),
    )
    .map_err(|error| AppError::single("formatting the connection import success", error))?;
    session
        .commit_external_connection(prepared, credential)
        .map_err(|error| AppError::single("publishing the grouped definition", error))?;
    Ok(success)
}

struct DefinitionChanges {
    added: Vec<String>,
    changed: Vec<String>,
    removed: Vec<String>,
    definition_changed: bool,
    account_transition: String,
    account_changed: bool,
    seed_transition: String,
    seed_changed: bool,
    resume_risk: Vec<String>,
}

fn definition_changes(
    snapshot: &ConnectionSnapshot,
    replacement_account: &ConnectionAccount,
    replacements: &[StoredModelBinding],
    replacement_seed: Option<&ConnectionCatalogSeed>,
) -> DefinitionChanges {
    let provider = replacement_account.provider_id();
    let account = replacement_account.account_id();
    let current_account = snapshot
        .accounts()
        .iter()
        .find(|stored| stored.provider_id() == provider && stored.account_id() == account);
    let current_seed = snapshot
        .catalog_seeds()
        .iter()
        .find(|seed| seed.provider() == provider && seed.account() == account);
    let current = snapshot
        .models()
        .iter()
        .filter(|binding| {
            let stored = binding.complete().binding();
            stored.provider_id() == provider && stored.account_id() == account
        })
        .collect::<Vec<_>>();
    let mut added = replacements
        .iter()
        .filter(|replacement| {
            !current
                .iter()
                .any(|stored| stored.selection() == replacement.selection())
        })
        .map(|binding| binding.selection().model().to_string())
        .collect::<Vec<_>>();
    let mut changed = replacements
        .iter()
        .filter(|replacement| {
            current.iter().any(|stored| {
                stored.selection() == replacement.selection() && **stored != **replacement
            })
        })
        .map(|binding| binding.selection().model().to_string())
        .collect::<Vec<_>>();
    let mut removed = current
        .iter()
        .filter(|stored| {
            !replacements
                .iter()
                .any(|replacement| replacement.selection() == stored.selection())
        })
        .map(|binding| binding.selection().model().to_string())
        .collect::<Vec<_>>();
    added.sort();
    changed.sort();
    removed.sort();
    let complete_binding_changes = replacements
        .iter()
        .filter(|replacement| {
            current.iter().any(|stored| {
                stored.selection() == replacement.selection()
                    && stored.complete() != replacement.complete()
            })
        })
        .map(|binding| binding.selection().model().to_string())
        .collect::<Vec<_>>();
    let mut resume_risk = complete_binding_changes
        .iter()
        .chain(&removed)
        .map(|model| {
            ModelSelection::new(
                provider.clone(),
                account.clone(),
                yo_core::ModelId::new(model).expect("stored ModelId remains valid"),
            )
            .canonical_reference()
        })
        .collect::<Vec<_>>();
    resume_risk.sort();
    let account_changed = current_account != Some(replacement_account);
    let seed_changed = !same_catalog_seed_definition(current_seed, replacement_seed);
    let account_transition = transition(
        &account_metadata_summary(current_account),
        &account_metadata_summary(Some(replacement_account)),
        account_changed,
    );
    let seed_transition = transition(
        &catalog_seed_summary(current_seed),
        &catalog_seed_summary(replacement_seed),
        seed_changed,
    );
    let definition_changed = account_changed
        || seed_changed
        || !added.is_empty()
        || !changed.is_empty()
        || !removed.is_empty();
    DefinitionChanges {
        added,
        changed,
        removed,
        definition_changed,
        account_transition,
        account_changed,
        seed_transition,
        seed_changed,
        resume_risk,
    }
}

fn same_catalog_seed_definition(
    current: Option<&ConnectionCatalogSeed>,
    replacement: Option<&ConnectionCatalogSeed>,
) -> bool {
    match (current, replacement) {
        (None, None) => true,
        (Some(current), Some(replacement)) => {
            match (current.built_in_profile(), replacement.built_in_profile()) {
                (Some(current), Some(replacement)) => current == replacement,
                (None, None) => {
                    current.openrouter_definition() == replacement.openrouter_definition()
                },
                _ => false,
            }
        },
        _ => false,
    }
}

fn transition(before: &str, after: &str, changed: bool) -> String {
    if changed {
        format!("{before}  →  {after}")
    } else {
        format!("Keep {after}")
    }
}

fn account_metadata_summary(account: Option<&ConnectionAccount>) -> String {
    let Some(account) = account else {
        return "no stored account metadata".to_owned();
    };
    let provider = account
        .provider_display_name()
        .map(crate::interaction::connection::escape_remote_text)
        .unwrap_or_else(|| "unset".to_owned());
    let account = account
        .account_display_name()
        .map(crate::interaction::connection::escape_remote_text)
        .unwrap_or_else(|| "unset".to_owned());
    format!("provider_display_name={provider}; account_display_name={account}")
}

fn catalog_seed_summary(seed: Option<&ConnectionCatalogSeed>) -> String {
    let Some(seed) = seed else {
        return "none".to_owned();
    };
    if let Some(catalog) = seed.built_in_profile() {
        return format!("built-in catalog {catalog}");
    }
    let Some((endpoint, profile)) = seed.openrouter_definition() else {
        return "invalid catalog seed".to_owned();
    };
    let max_output_tokens = profile
        .context()
        .max_output_tokens()
        .map_or_else(|| "unknown".to_owned(), |value| value.to_string());
    format!(
        "OpenRouter discovery endpoint={}; api_dialect={}; tokenizer_profile={}; input_token_limit={}; max_output_tokens={}; reasoning_parameters={}; optional_request_parameters={}; tool_capability_policy={}; replay_profile={}",
        endpoint,
        profile.api_dialect(),
        profile.context().tokenizer_profile(),
        profile.context().input_token_limit(),
        max_output_tokens,
        profile.reasoning_parameters().to_json_value(),
        profile.optional_request_parameters().to_json_value(),
        profile.tool_capability_policy(),
        profile.replay_profile(),
    )
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
        .map_err(|error| AppError::single("capturing stored connections", error))?;
    let (selected, preselected_candidate, remote_selected) = match catalog_pair(
        &snapshot,
        &command.target,
    )? {
        Some((provider, account)) if provider.as_str() == "openrouter" => {
            let seed = snapshot
                .openrouter_discovery_seed(&provider, &account)
                .map_err(|error| AppError::single("reading the stored OpenRouter seed", error))?
                .ok_or_else(|| {
                    AppError::message(format!(
                        "OpenRouter discovery target {} is not an exact stored Provider:Account seed with a complete base profile",
                        command.target
                    ))
                })?;
            let account_reference = stored_account_reference(&snapshot, &provider, &account)?;
            let candidate = input.read_credential(&account_reference)?;
            let Some(entry) = discover_openrouter_and_select(&seed, &candidate, input)? else {
                return Ok("Connection cancelled; nothing changed.\n".to_owned());
            };
            (entry, Some(candidate), true)
        },
        Some((provider, account)) if provider.as_str() == "kimi" => {
            let seed = snapshot
                .kimi_catalog_seed(&provider, &account)
                .map_err(|error| AppError::single("reading the stored Kimi seed", error))?
                .ok_or_else(|| {
                    AppError::message(format!(
                        "Kimi catalog target {} is not an exact stored Provider:Account seed",
                        command.target
                    ))
                })?;
            let account_reference = stored_account_reference(&snapshot, &provider, &account)?;
            let candidate = input.read_credential(&account_reference)?;
            let Some(entry) = discover_kimi_and_select(&seed, &candidate, input)? else {
                return Ok("Connection cancelled; nothing changed.\n".to_owned());
            };
            (entry, Some(candidate), true)
        },
        Some((provider, account)) => {
            let seed = snapshot
                .qwencloud_catalog_seed(&provider, &account)
                .map_err(|error| AppError::single("reading the stored QwenCloud seed", error))?
                .ok_or_else(|| {
                    AppError::message(format!(
                        "QwenCloud catalog target {} is not an exact stored Provider:Account seed",
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
            let account_reference = stored_account_reference(&snapshot, &provider, &account)?;
            let candidate = input.read_credential(&account_reference)?;
            (entry, Some(candidate), false)
        },
        None => (selected_entry(&snapshot, &command.target)?, None, false),
    };
    let selection = selection_for(&selected);
    let startup_policy = StartupPolicy::initial();
    let mut plan = ExternalConnectPlan::prepare(&snapshot, &selection, &selected, &startup_policy)
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
        stored_change,
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
            stored_change,
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
            let account_reference =
                stored_account_reference(&snapshot, selection.provider(), selection.account())?;
            input.read_credential(&account_reference)?
        },
    };
    let success = connect_success(
        &display_target(Some(&StartupTarget::Model(selection.clone()))),
        binding_count,
        &display_target(preference.as_ref()),
    )
    .map_err(|error| AppError::single("formatting the connection success", error))?;
    finalize(&mut session, &config, prepared, candidate, remote_selected)?;

    Ok(success)
}

struct ExternalConnectPlan {
    connection: PreparedConnectionMutation,
    bindings: Vec<CompleteModelBinding>,
    binding_count: usize,
    preference: Option<StartupTarget>,
    target: String,
    account: String,
    default_after: String,
    stored_change: StoredConnectionChange,
    default_changed: bool,
    binding_details: Vec<crate::interaction::connection::BindingDetails>,
}

impl ExternalConnectPlan {
    fn escape_remote_model(&mut self, model_id: &str) {
        for details in &mut self.binding_details {
            details.escape_remote_model(model_id);
        }
    }

    fn prepare(
        snapshot: &ConnectionSnapshot,
        selection: &ModelSelection,
        selected: &ModelCatalogEntry,
        startup_policy: &StartupPolicy,
    ) -> Result<Self, AppError> {
        let complete = selected.complete_binding().cloned().ok_or_else(|| {
            AppError::message(
                "the selected external model is missing its complete stored connection profile",
            )
        })?;
        let mut prospective_bindings = Vec::new();
        for retained in snapshot.models().iter().filter(|retained| {
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

        let account = ConnectionAccount::new(
            selection.provider().clone(),
            selection.account().clone(),
            selected.provider_display_name().map(str::to_owned),
            selected.account_display_name().map(str::to_owned),
        )
        .map_err(|error| AppError::single("preparing the stored account", error))?;
        let binding =
            StoredModelBinding::new(complete, selected.model_display_name().map(str::to_owned))
                .map_err(|error| AppError::single("preparing the stored model binding", error))?;
        let account_unchanged = snapshot
            .accounts()
            .iter()
            .any(|current| current == &account);
        let stored_change = match snapshot
            .models()
            .iter()
            .find(|current| current.selection() == *selection)
        {
            None => StoredConnectionChange::Create,
            Some(current) if account_unchanged && current == &binding => {
                StoredConnectionChange::Keep
            },
            Some(_) => StoredConnectionChange::Update,
        };
        let prospective_catalog = snapshot
            .catalog_after_model_upsert(account.clone(), binding.clone())
            .map_err(|error| {
                AppError::single("composing the prospective stored connection", error)
            })?;
        admit_external_target(&prospective_catalog, selection, startup_policy)?;
        let connection = snapshot
            .prepare_model_connect(account, binding)
            .map_err(|error| AppError::single("preparing stored connection state", error))?;
        let preference = connection.preference().cloned();
        let binding_count = prospective_bindings.len();
        let mut presentation_bindings = prospective_bindings.clone();
        if let Some(displaced) = snapshot
            .models()
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
            account: crate::interaction::connection::escape_remote_text(&format!(
                "{}:{}",
                selection.provider(),
                selection.account()
            )),
            default_after,
            stored_change,
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
                self.stored_change,
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
    reference: &str,
) -> Result<ModelCatalogEntry, AppError> {
    let catalog = snapshot
        .model_catalog()
        .map_err(|error| AppError::single("reading the stored model catalog", error))?;
    if let Some(entry) = catalog
        .entries()
        .iter()
        .find(|entry| selection_for(entry).canonical_reference() == reference)
        .cloned()
    {
        return Ok(entry);
    }
    for stored_seed in snapshot.catalog_seeds() {
        let Some(seed) = stored_seed
            .qwencloud_seed()
            .map_err(|error| AppError::single("reading the stored QwenCloud seed", error))?
        else {
            continue;
        };
        if let Some(row) = seed.models().iter().find(|row| {
            ModelSelection::new(
                row.provider().clone(),
                row.account().clone(),
                row.model_id().clone(),
            )
            .canonical_reference()
                == reference
        }) {
            return row.entry().cloned().ok_or_else(|| {
                let reason = match row.availability() {
                    yo_core::QwenCloudCatalogAvailability::Enabled => "invalid catalog row",
                    yo_core::QwenCloudCatalogAvailability::Disabled(reason) => reason.as_str(),
                };
                AppError::message(format!(
                    "QwenCloud catalog model {reference:?} is disabled: {reason}"
                ))
            });
        }
    }
    Err(AppError::message(format!(
        "external connect target {reference:?} is not an exact stored Provider:Account:Model reference; import its definition with yo connect --from"
    )))
}

fn catalog_pair(
    snapshot: &ConnectionSnapshot,
    reference: &str,
) -> Result<Option<(ProviderId, AccountId)>, AppError> {
    if let Some(account) = snapshot.accounts().iter().find(|account| {
        matches!(
            account.provider_id().as_str(),
            "openrouter" | "qwencloud" | "kimi"
        ) && account.canonical_reference() == reference
            && snapshot.catalog_seeds().iter().any(|seed| {
                seed.provider() == account.provider_id() && seed.account() == account.account_id()
            })
    }) {
        return Ok(Some((
            account.provider_id().clone(),
            account.account_id().clone(),
        )));
    }
    let mut segments = reference.split(':');
    let Some(provider) = segments.next() else {
        return Ok(None);
    };
    let Some(_account) = segments.next() else {
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
    Err(AppError::message(format!(
        "catalog target {reference:?} is not an exact stored Provider:Account seed"
    )))
}

fn looks_like_two_part_target(reference: &str) -> bool {
    let mut segments = reference.split(':');
    segments.next().is_some() && segments.next().is_some() && segments.next().is_none()
}

fn stored_account_reference(
    snapshot: &ConnectionSnapshot,
    provider: &ProviderId,
    account: &AccountId,
) -> Result<String, AppError> {
    snapshot
        .accounts()
        .iter()
        .find(|stored| stored.provider_id() == provider && stored.account_id() == account)
        .map(ConnectionAccount::canonical_reference)
        .ok_or_else(|| {
            AppError::message(format!(
                "stored Provider {provider} and Account {account} has no account definition"
            ))
        })
}

fn safe_discovery_error(error: AppError, discovered: bool) -> AppError {
    if discovered {
        AppError::message(crate::interaction::connection::escape_remote_text(
            &error.to_string(),
        ))
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
            crate::interaction::connection::escape_remote_text(&error.to_string())
        ))
    } else {
        AppError::single(context, error)
    }
}

fn selection_for(entry: &ModelCatalogEntry) -> ModelSelection {
    selection_for_binding(entry.binding())
}

#[cfg(test)]
fn seed_stored_definition(root: &Path, contents: &str) {
    let definition = import::parse(contents).unwrap();
    let repository = yo_core::LocalConnectionRepository::new(root.join("connections.yaml"));
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_group_replace(
            definition.account,
            definition.bindings,
            definition.catalog_seed,
        )
        .unwrap();
    repository.commit(&mutation).unwrap();
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

    struct ImportInput {
        expected_account: &'static str,
        preview: Option<String>,
        confirmations: usize,
        credential_reads: usize,
    }

    impl ExternalConnectInput for ImportInput {
        fn confirm(
            &mut self,
            preview: &dyn crate::interaction::connection::ConfirmationView,
        ) -> Result<bool, AppError> {
            self.confirmations += 1;
            self.preview = Some(
                preview
                    .render_styled(
                        crate::interaction::connection::default_width(),
                        crate::interaction::PresentationStyle::Plain,
                    )
                    .unwrap(),
            );
            Ok(true)
        }

        fn read_credential(&mut self, account: &str) -> Result<yo_core::ApiCredential, AppError> {
            assert_eq!(account, self.expected_account);
            self.credential_reads += 1;
            yo_core::ApiCredential::new("group-secret")
                .map_err(|error| AppError::single("constructing import test credential", error))
        }
    }

    // 한 grouped document의 모든 모델은 한 번의 preview와 credential capture 뒤 동일 public
    // revision에 게시되고, definition-only import는 임의 default를 만들지 않습니다.
    #[test]
    fn grouped_import_publishes_multiple_models_without_selecting_a_default() {
        let root = crate::state::connection::canonical_test_temp_dir().join(format!(
            "yo-grouped-import-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config_path = root.join("config.yaml");
        std::fs::write(&config_path, "session: {}\n").unwrap();
        let definition = import::parse(explicit_multi_definition()).unwrap();
        let mut input = ImportInput {
            expected_account: "vendor:team",
            preview: None,
            confirmations: 0,
            credential_reads: 0,
        };

        let output = execute_definition_import_with(
            &config_path,
            ConnectCommand {
                target: String::new(),
                from: Some(root.join("definition.yaml")),
                verbose: true,
                credential_file: None,
                yes: false,
            },
            definition,
            &mut input,
        )
        .unwrap();

        assert!(output.contains("Account     vendor:team"));
        assert!(output.contains("Registered  2 model profiles"));
        assert!(output.contains("Default     unset"));
        assert_eq!(input.confirmations, 1);
        assert_eq!(input.credential_reads, 1);
        let preview = input.preview.unwrap();
        assert!(preview.contains("Add models"));
        assert!(preview.contains("alpha, beta"));
        assert!(preview.contains("No stored complete binding is changed or removed"));
        let snapshot = yo_core::LocalConnectionRepository::new(root.join("connections.yaml"))
            .capture()
            .unwrap();
        assert_eq!(snapshot.models().len(), 2);
        assert!(snapshot.preference().is_none());
        let credential = yo_core::CredentialRepository::capture(
            &yo_core::LocalCredentialRepository::new(root.join("credentials.yaml")),
        )
        .unwrap();
        assert_eq!(
            credential
                .resolve(
                    &ProviderId::new("vendor").unwrap(),
                    &AccountId::new("team").unwrap(),
                )
                .unwrap()
                .expose_secret(),
            "group-secret"
        );
        assert!(!root.join("connection-operation.yaml").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    // Catalog-only definitions still bind one account credential and one public seed, but they
    // do not invent a routable model or preference during import.
    #[test]
    fn grouped_catalog_import_publishes_a_seed_without_inventing_a_model() {
        let root = crate::state::connection::canonical_test_temp_dir().join(format!(
            "yo-grouped-catalog-import-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config_path = root.join("config.yaml");
        std::fs::write(&config_path, "session: {}\n").unwrap();
        let definition = import::parse(
            "provider: qwencloud\naccount: default\ncatalog: qwencloud-token-plan-team-intl/v1\n",
        )
        .unwrap();
        let mut input = ImportInput {
            expected_account: "qwencloud:default",
            preview: None,
            confirmations: 0,
            credential_reads: 0,
        };

        let output = execute_definition_import_with(
            &config_path,
            ConnectCommand {
                target: String::new(),
                from: Some(root.join("definition.yaml")),
                verbose: false,
                credential_file: None,
                yes: false,
            },
            definition,
            &mut input,
        )
        .unwrap();

        assert!(output.contains("Account     qwencloud:default"));
        assert!(output.contains("Registered  0 model profiles"));
        assert_eq!(input.confirmations, 1);
        assert_eq!(input.credential_reads, 1);
        let snapshot = yo_core::LocalConnectionRepository::new(root.join("connections.yaml"))
            .capture()
            .unwrap();
        assert!(snapshot.models().is_empty());
        assert_eq!(snapshot.catalog_seeds().len(), 1);
        assert!(snapshot.preference().is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    // Preview는 모델 목록 외에도 account label과 exact catalog seed 전이를 보여 주고,
    // 변경·삭제되는 complete binding을 사용하는 저장 Session의 resume 위험을 숨기지 않습니다.
    #[test]
    fn grouped_import_previews_seed_metadata_and_resume_transitions() {
        let root = crate::state::connection::canonical_test_temp_dir().join(format!(
            "yo-grouped-transition-preview-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config_path = root.join("config.yaml");
        std::fs::write(&config_path, "session: {}\n").unwrap();
        let repository = yo_core::LocalConnectionRepository::new(root.join("connections.yaml"));
        let old = import::parse(
            "provider: qwencloud\nprovider_display_name: Old Qwen\naccount: team\naccount_display_name: Old Team\ncatalog: qwencloud-coding-plan-intl/v1\n",
        )
        .unwrap();
        let retained_then_removed = StoredModelBinding::new(
            CompleteModelBinding::from_durable_json(
                r#"{"provider":"qwencloud","account":"team","model":"qwen3-coder-plus","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1"}"#,
            )
            .unwrap(),
            Some("Qwen3 Coder Plus".to_owned()),
        )
        .unwrap();
        let initial = repository
            .capture()
            .unwrap()
            .prepare_group_replace(old.account, vec![retained_then_removed], old.catalog_seed)
            .unwrap();
        repository.commit(&initial).unwrap();
        let replacement = import::parse(
            "provider: qwencloud\nprovider_display_name: New Qwen\naccount: team\naccount_display_name: New Team\ncatalog: qwencloud-token-plan-team-intl/v1\n",
        )
        .unwrap();
        let mut input = CancelInput {
            summary: None,
            credential_reads: 0,
        };

        let output = execute_definition_import_with(
            &config_path,
            ConnectCommand {
                target: String::new(),
                from: Some(root.join("definition.yaml")),
                verbose: false,
                credential_file: None,
                yes: false,
            },
            replacement,
            &mut input,
        )
        .unwrap();

        assert_eq!(output, "Connection import cancelled; nothing changed.\n");
        assert_eq!(input.credential_reads, 0);
        let preview = input.summary.unwrap();
        assert!(preview.contains("Old Qwen"));
        assert!(preview.contains("New Qwen"));
        let compact = preview.split_whitespace().collect::<String>();
        assert!(compact.contains("qwencloud-coding-plan-intl/v1"));
        assert!(
            compact.contains("qwencloud-token-plan-team-intl/v1"),
            "preview: {preview}"
        );
        assert!(compact.contains("qwencloud:team:qwen3-coder-plus"));
        assert!(preview.contains("May not resume"));
        assert!(preview.contains("Remove models"));
        std::fs::remove_dir_all(root).unwrap();
    }

    // Account display metadata and the catalog source are separate preview fields, so changing
    // only labels must not report a duplicate `same catalog -> same catalog` transition.
    #[test]
    fn grouped_import_keeps_an_unchanged_seed_when_only_account_metadata_changes() {
        let root = crate::state::connection::canonical_test_temp_dir().join(format!(
            "yo-grouped-metadata-only-preview-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config_path = root.join("config.yaml");
        std::fs::write(&config_path, "session: {}\n").unwrap();
        seed_stored_definition(
            &root,
            "provider: qwencloud\nprovider_display_name: Old Qwen\naccount: team\naccount_display_name: Old Team\ncatalog: qwencloud-coding-plan-intl/v1\n",
        );
        let replacement = import::parse(
            "provider: qwencloud\nprovider_display_name: New Qwen\naccount: team\naccount_display_name: New Team\ncatalog: qwencloud-coding-plan-intl/v1\n",
        )
        .unwrap();
        let mut input = CancelInput {
            summary: None,
            credential_reads: 0,
        };

        let output = execute_definition_import_with(
            &config_path,
            ConnectCommand {
                target: String::new(),
                from: Some(root.join("definition.yaml")),
                verbose: false,
                credential_file: None,
                yes: false,
            },
            replacement,
            &mut input,
        )
        .unwrap();

        assert_eq!(output, "Connection import cancelled; nothing changed.\n");
        let preview = input.summary.unwrap();
        assert!(preview.contains("Old Qwen"));
        assert!(preview.contains("New Qwen"));
        assert!(preview.contains("Keep built-in catalog"));
        assert_eq!(
            preview.matches("qwencloud-coding-plan-intl/v1").count(),
            1,
            "unchanged seed must appear once as a kept value: {preview}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    // Exact target resolution compares canonical spellings made by stored typed coordinates;
    // encoded Account separators and vendor-owned ModelId colons are never split heuristically.
    #[test]
    fn selected_entry_uses_canonical_coordinates_with_encoded_accounts_and_colon_models() {
        let root = crate::state::connection::canonical_test_temp_dir().join(format!(
            "yo-canonical-selected-entry-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        seed_stored_definition(
            &root,
            "provider: qwencloud\naccount: 'team:west'\ncatalog: qwencloud-coding-plan-intl/v1\n",
        );
        seed_stored_definition(
            &root,
            r#"
provider: qwencloud
account: other
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
  - model: 'vendor:model'
"#,
        );
        let snapshot = yo_core::LocalConnectionRepository::new(root.join("connections.yaml"))
            .capture()
            .unwrap();

        let (_, catalog_account) = catalog_pair(&snapshot, "qwencloud:team%3Awest")
            .unwrap()
            .unwrap();
        assert_eq!(catalog_account.as_str(), "team:west");
        let catalog = selected_entry(&snapshot, "qwencloud:team%3Awest:qwen3-coder-plus").unwrap();
        assert_eq!(selection_for(&catalog).account().as_str(), "team:west");
        let stored = selected_entry(&snapshot, "qwencloud:other:vendor:model").unwrap();
        assert_eq!(selection_for(&stored).model().as_str(), "vendor:model");
        std::fs::remove_dir_all(root).unwrap();
    }

    struct CancelInput {
        summary: Option<String>,
        credential_reads: usize,
    }

    impl ExternalConnectInput for CancelInput {
        fn confirm(
            &mut self,
            preview: &dyn crate::interaction::connection::ConfirmationView,
        ) -> Result<bool, AppError> {
            self.summary = Some(
                preview
                    .render_styled(
                        crate::interaction::connection::default_width(),
                        crate::interaction::PresentationStyle::Plain,
                    )
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
        let root = crate::state::connection::canonical_test_temp_dir().join(format!(
            "yo-external-cancel-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config_path = root.join("config.yaml");
        std::fs::write(&config_path, "session: {}\n").unwrap();
        seed_stored_definition(&root, explicit_definition());
        let before = std::fs::read(root.join("connections.yaml")).unwrap();
        let mut input = CancelInput {
            summary: None,
            credential_reads: 0,
        };

        let output = execute_external_connect_with(
            &config_path,
            ConnectCommand {
                from: None,
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
        assert_eq!(
            std::fs::read(root.join("connections.yaml")).unwrap(),
            before
        );
        for name in ["credentials.yaml", "connection-operation.yaml"] {
            assert!(!root.join(name).exists(), "{name} must remain absent");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    // `--yes` 경로는 TTY를 열지 않고 plan 준비 뒤 지정 파일을 읽으며, 안전하지 않은
    // credential 파일은 새 intent나 public/credential repository mutation 전에 실패합니다.
    #[test]
    fn non_interactive_file_failure_stops_before_new_repository_mutation() {
        let root = crate::state::connection::canonical_test_temp_dir().join(format!(
            "yo-external-file-failure-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config_path = root.join("config.yaml");
        std::fs::write(&config_path, "session: {}\n").unwrap();
        seed_stored_definition(&root, explicit_definition());
        let before = std::fs::read(root.join("connections.yaml")).unwrap();
        let credential_path = root.join("credential");
        std::fs::write(&credential_path, b"diagnostic-sentinel-secret").unwrap();
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&credential_path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let error = run_external_connect(
            &config_path,
            ConnectCommand {
                from: None,
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
        assert_eq!(
            std::fs::read(root.join("connections.yaml")).unwrap(),
            before
        );
        for name in ["credentials.yaml", "connection-operation.yaml"] {
            assert!(!root.join(name).exists(), "{name} must remain absent");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    // Parser 밖의 injected caller도 두 option 중 하나만 주거나 `--verbose`를 함께 주어
    // TTY/file 흐름을 우회할 수 없고 config와 repository를 읽기 전에 같은 오류로 닫힙니다.
    #[test]
    fn runtime_rejects_an_invalid_non_interactive_option_combination() {
        let config_path = Path::new("/not/read/config.yaml");
        for command in [
            ConnectCommand {
                from: None,
                target: "vendor:team:alpha".to_owned(),
                verbose: false,
                credential_file: Some("/not/read/credential".into()),
                yes: false,
            },
            ConnectCommand {
                from: None,
                target: "vendor:team:alpha".to_owned(),
                verbose: false,
                credential_file: None,
                yes: true,
            },
            ConnectCommand {
                from: None,
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
        let root = crate::state::connection::canonical_test_temp_dir().join(format!(
            "yo-external-replace-preview-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config_path = root.join("config.yaml");
        std::fs::write(&config_path, "session: {}\n").unwrap();
        seed_stored_definition(&root, explicit_definition());
        let credentials = yo_core::LocalCredentialRepository::new(root.join("credentials.yaml"));
        let provider = ProviderId::new("vendor").unwrap();
        let account = AccountId::new("team").unwrap();
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
                from: None,
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
        let snapshot = fixture_snapshot_with_stored(fixture_complete("beta"));
        let selection = ModelSelection::new(
            ProviderId::new("vendor").unwrap(),
            AccountId::new("team").unwrap(),
            yo_core::ModelId::new("alpha").unwrap(),
        );

        let selected = catalog
            .resolve_model(selection.provider(), selection.account(), selection.model())
            .unwrap();
        let plan = ExternalConnectPlan::prepare(
            &snapshot,
            &selection,
            selected,
            &StartupPolicy::initial(),
        )
        .unwrap();

        assert_eq!(plan.binding_count, 2);
        let preview = plan
            .preview(yo_core::CredentialMutationAction::Replace, true)
            .render(crate::interaction::connection::default_width())
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
        assert_eq!(
            plan.preference,
            Some(StartupTarget::Model(ModelSelection::new(
                ProviderId::new("vendor").unwrap(),
                AccountId::new("team").unwrap(),
                yo_core::ModelId::new("beta").unwrap(),
            )))
        );
    }

    // 새 complete profile과 기존 stored profile이 같은 coordinate에서 달라도
    // preview는 둘 다 비교하되 게시·admission 집합은 교체 뒤 새 profile 하나만 셉니다.
    #[test]
    fn plan_discloses_old_and_new_profiles_during_stored_replacement() {
        let catalog = ModelCatalog::new(vec![fixture_entry("alpha")]).unwrap();
        let snapshot = fixture_snapshot_with_stored(fixture_complete_at(
            "alpha",
            "https://old.example.test/v1",
            "openai-chat-completions",
            r#"{"effort":"medium"}"#,
        ));
        let selection = ModelSelection::new(
            ProviderId::new("vendor").unwrap(),
            AccountId::new("team").unwrap(),
            yo_core::ModelId::new("alpha").unwrap(),
        );
        let selected = catalog
            .resolve_model(selection.provider(), selection.account(), selection.model())
            .unwrap();

        let plan = ExternalConnectPlan::prepare(
            &snapshot,
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
            .render(crate::interaction::connection::default_width())
            .unwrap();
        assert!(preview.contains("Connection profile 1 of 2"));
        assert!(preview.contains("Connection profile 2 of 2"));
        assert!(preview.matches("Models (1)").count() == 2);
        assert!(preview.contains("https://example.test/v1"));
        assert!(preview.contains("https://old.example.test/v1"));
        assert!(preview.contains("openai-responses"));
        assert!(preview.contains("openai-chat-completions"));
        assert!(preview.contains("~ Stored connection\n  Update vendor:team:alpha"));
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

    // 더는 runtime이 지원하지 않는 이전 stored profile도 교체 뒤 집합에서는 사라지므로,
    // 새 profile의 구조적 admission을 막거나 성공 모델 수를 부풀리지 않습니다.
    #[test]
    fn replacement_excludes_a_displaced_unsupported_profile_from_admission() {
        let catalog = ModelCatalog::new(vec![fixture_entry("alpha")]).unwrap();
        let snapshot = fixture_snapshot_with_stored(fixture_complete_with_options(
            "alpha",
            "https://old.example.test/v1",
            r#"{"retired":true}"#,
        ));
        let selection = ModelSelection::new(
            ProviderId::new("vendor").unwrap(),
            AccountId::new("team").unwrap(),
            yo_core::ModelId::new("alpha").unwrap(),
        );
        let selected = catalog
            .resolve_model(selection.provider(), selection.account(), selection.model())
            .unwrap();
        let expected = selected.complete_binding().unwrap().clone();
        let plan = ExternalConnectPlan::prepare(
            &snapshot,
            &selection,
            selected,
            &StartupPolicy::initial(),
        )
        .unwrap();

        assert_eq!(plan.binding_count, 1);
        assert_eq!(plan.bindings, vec![expected]);
        let preview = plan
            .preview(yo_core::CredentialMutationAction::Replace, true)
            .render(crate::interaction::connection::default_width())
            .unwrap();
        assert!(preview.contains("Connection profile 1 of 2"));
        assert!(preview.contains("Connection profile 2 of 2"));
        assert!(preview.contains(r#"{"retired":true}"#));

        let root = crate::state::connection::canonical_test_temp_dir().join(format!(
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
            ProviderId::new("vendor").unwrap(),
            AccountId::new("team").unwrap(),
            yo_core::ModelId::new("alpha").unwrap(),
        );
        let selected = catalog
            .resolve_model(selection.provider(), selection.account(), selection.model())
            .unwrap();
        let enforced_host =
            StartupPolicy::new(false, Some(StartupTarget::host_codex()), None).unwrap();

        let error =
            ExternalConnectPlan::prepare(&fixture_snapshot(), &selection, selected, &enforced_host)
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

    fn explicit_definition() -> &'static str {
        r#"
provider: vendor
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

    fn explicit_multi_definition() -> &'static str {
        r#"
provider: vendor
provider_display_name: Vendor
account: team
account_display_name: Team
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
  - model: beta
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

    fn fixture_snapshot_with_stored(complete: CompleteModelBinding) -> ConnectionSnapshot {
        let root = std::env::temp_dir().join(format!(
            "yo-external-plan-stored-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let repository = yo_core::LocalConnectionRepository::new(root.join("connections.yaml"));
        let account = ConnectionAccount::new(
            ProviderId::new("vendor").unwrap(),
            AccountId::new("team").unwrap(),
            Some("Vendor".to_owned()),
            Some("Team".to_owned()),
        )
        .unwrap();
        let binding = StoredModelBinding::new(complete, Some("alpha".to_owned())).unwrap();
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_model_connect(account, binding)
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
