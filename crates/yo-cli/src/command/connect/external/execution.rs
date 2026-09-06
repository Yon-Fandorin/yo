use std::path::Path;

use yo_core::{ModelCatalogEntry, StartupPolicy, StartupTarget};
use yo_provider_kimi::KimiCatalogSeed;
use yo_provider_openrouter::OpenRouterDiscoverySeed;
use yo_provider_qwencloud::QwenCloudCatalogSeed;

use super::{
    super::{
        Command as ConnectCommand,
        presentation::{Confirmation, ConnectPreview, connect_success},
    },
    catalog::{
        catalog_pair, discover_kimi_and_select, discover_openrouter_and_select, select_qwencloud,
        selected_entry, selection_for, stored_account_reference,
    },
    plan::ExternalConnectPlan,
};
use crate::{
    AppError,
    command::connect::input::ExternalConnectInput,
    state::{
        config,
        connection::{display_target, operation_repositories},
    },
};

pub(super) fn execute_external_connect_with(
    config_path: &Path,
    command: ConnectCommand,
    input: &mut impl ExternalConnectInput,
) -> Result<String, AppError> {
    execute_external_connect_with_discovery(
        config_path,
        command,
        input,
        discover_openrouter_and_select,
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

pub(super) fn execute_external_connect_with_discovery<I>(
    config_path: &Path,
    command: ConnectCommand,
    input: &mut I,
    discover_and_select: impl FnOnce(
        &OpenRouterDiscoverySeed,
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
        discover_kimi_and_select,
        finalize,
    )
}

pub(super) fn execute_external_connect_with_catalogs<I>(
    config_path: &Path,
    command: ConnectCommand,
    input: &mut I,
    discover_openrouter_and_select: impl FnOnce(
        &OpenRouterDiscoverySeed,
        &yo_core::ApiCredential,
        &mut I,
    ) -> Result<Option<ModelCatalogEntry>, AppError>,
    discover_kimi_and_select: impl FnOnce(
        &KimiCatalogSeed,
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
                .catalog_seed(&provider, &account)
                .map(OpenRouterDiscoverySeed::from_connection_seed)
                .transpose()
                .map(Option::flatten)
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
                .catalog_seed(&provider, &account)
                .map(KimiCatalogSeed::from_connection_seed)
                .transpose()
                .map(Option::flatten)
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
                .catalog_seed(&provider, &account)
                .map(QwenCloudCatalogSeed::from_connection_seed)
                .transpose()
                .map(Option::flatten)
                .map_err(|error| AppError::single("reading the stored QwenCloud seed", error))?
                .ok_or_else(|| {
                    AppError::message(format!(
                        "QwenCloud catalog target {} is not an exact stored Provider:Account seed",
                        command.target
                    ))
                })?;
            let Some(entry) = select_qwencloud(&seed, input)? else {
                return Ok("Connection cancelled; nothing changed.\n".to_owned());
            };
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
        .prepare_external_connection(
            &crate::execution::model::NativeBindingAdmission,
            connection,
            bindings,
        )
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
