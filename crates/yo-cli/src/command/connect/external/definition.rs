use std::path::Path;

use yo_core::{
    ConnectionAccount, ConnectionCatalogSeed, ConnectionSnapshot, ModelSelection,
    StoredModelBinding,
};

use super::super::{
    Command as ConnectCommand,
    import::ImportedDefinition,
    presentation::{Confirmation, ImportPreview, import_success},
};
use crate::{
    AppError,
    command::connect::input::ExternalConnectInput,
    state::{
        config,
        connection::{complete_binding_details, display_target, operation_repositories},
    },
};

pub(super) fn execute_definition_import_with(
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
        .prepare_external_definition(
            &crate::execution::model::NativeBindingAdmission,
            mutation,
            &provider,
            &account_id,
            complete_bindings,
        )
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
                    current.discovery_definition() == replacement.discovery_definition()
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
    let Some((endpoint, profile)) = seed.discovery_definition() else {
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
