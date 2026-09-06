//! External provider connection command facade.

mod catalog;
mod definition;
mod execution;
mod plan;

use std::path::Path;

use definition::execute_definition_import_with;
use execution::execute_external_connect_with;

use super::{Command as ConnectCommand, import};
use crate::{
    AppError, command::connect::input::AuthorizedCredentialFileInput,
    interaction::prompt::TtyPrompt,
};

pub(super) fn run_external_connect(
    config_path: &Path,
    command: ConnectCommand,
) -> Result<String, AppError> {
    if catalog::looks_like_two_part_target(&command.target)
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

#[cfg(test)]
use yo_core::{
    AccountId, CompleteModelBinding, ConnectionAccount, ConnectionSnapshot, ModelCatalog,
    ModelCatalogEntry, ModelSelection, ProviderId, StartupPolicy, StartupTarget,
    StoredModelBinding,
};
#[cfg(test)]
use {
    catalog::{catalog_pair, selected_entry, selection_for},
    execution::{execute_external_connect_with_catalogs, execute_external_connect_with_discovery},
    plan::ExternalConnectPlan,
};

#[cfg(test)]
use crate::command::connect::input::ExternalConnectInput;

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
