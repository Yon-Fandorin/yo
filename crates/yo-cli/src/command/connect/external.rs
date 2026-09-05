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
mod tests;
