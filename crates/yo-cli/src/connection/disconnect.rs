use std::path::Path;

use yo_core::{
    ConnectionSnapshot, ExternalDisconnectCredentialAction, ManagedConnectionBinding, ModelCatalog,
    ModelCatalogEntry, ModelSelection, StartupTarget,
};

use super::{
    complete_binding_summary, display_target,
    input::{ExternalDisconnectInput, TtyConnectionInput},
    operation_repositories, selection_for_binding,
};
use crate::{AppError, command::DisconnectCommand, config};

pub(super) fn run_external_disconnect(
    config_path: &Path,
    command: DisconnectCommand,
) -> Result<String, AppError> {
    let mut input = TtyConnectionInput::new();
    execute_external_disconnect_with(config_path, command, &mut input)
}

fn execute_external_disconnect_with(
    config_path: &Path,
    command: DisconnectCommand,
    input: &mut impl ExternalDisconnectInput,
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
        .map_err(|error| AppError::single("capturing managed connections", error))?;
    let selection = select_managed_target(&snapshot, config.model_catalog(), &command, input)?;
    let plan = ExternalDisconnectPlan::prepare(&snapshot, config.model_catalog(), &selection)?;
    let prepared = session
        .prepare_external_disconnect(
            config.snapshot_digest(),
            snapshot.revision(),
            &selection,
            plan.credential_action,
        )
        .map_err(|error| AppError::single("preparing the external disconnect", error))?;

    if !command.yes && !input.confirm(&plan.summary)? {
        return Ok("disconnect cancelled; no state changed\n".to_owned());
    }
    config
        .verify_unchanged()
        .map_err(|error| AppError::single("guarding Yo configuration", error))?;
    session
        .commit_external_disconnect(prepared)
        .map_err(|error| AppError::single("publishing the external disconnect", error))?;

    Ok(format!(
        "disconnected: {}; credential: {}; default: {}\n",
        selection.canonical_reference(),
        action_label(plan.credential_action),
        display_target(plan.preference_after.as_ref())
    ))
}

fn select_managed_target(
    snapshot: &ConnectionSnapshot,
    manual: &ModelCatalog,
    command: &DisconnectCommand,
    input: &mut impl ExternalDisconnectInput,
) -> Result<ModelSelection, AppError> {
    let mut candidates = snapshot
        .managed_bindings()
        .iter()
        .map(ManagedConnectionBinding::selection)
        .filter(|selection| {
            command
                .provider
                .as_deref()
                .is_none_or(|provider| selection.provider().as_str() == provider)
                && command
                    .account
                    .as_deref()
                    .is_none_or(|account| selection.account().as_str() == account)
        })
        .collect::<Vec<_>>();
    candidates.sort();

    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 => Err(no_managed_target_error(manual, command)),
        _ if command.yes => Err(AppError::message(format!(
            "Provider {} Account {} has {} managed model targets; --yes never guesses which one to remove, so rerun interactively",
            command.provider.as_deref().unwrap_or("<missing>"),
            command.account.as_deref().unwrap_or("<missing>"),
            candidates.len()
        ))),
        _ => {
            let choices = candidates
                .iter()
                .map(ModelSelection::canonical_reference)
                .collect::<Vec<_>>();
            let selected = input.select_target(&choices)?;
            candidates
                .into_iter()
                .find(|candidate| candidate.canonical_reference() == selected.trim())
                .ok_or_else(|| {
                    AppError::message(format!(
                        "disconnect target {:?} is not one of the captured managed targets",
                        selected.trim()
                    ))
                })
        },
    }
}

fn no_managed_target_error(manual: &ModelCatalog, command: &DisconnectCommand) -> AppError {
    let manual_matches = manual.entries().iter().any(|entry| {
        let binding = entry.binding();
        command
            .provider
            .as_deref()
            .is_none_or(|provider| binding.provider_id().as_str() == provider)
            && command
                .account
                .as_deref()
                .is_none_or(|account| binding.account_id().as_str() == account)
    });
    if manual_matches {
        AppError::message(
            "the selected Provider and Account has only manual config.yaml bindings; edit config.yaml because yo disconnect removes only managed provenance",
        )
    } else {
        AppError::message("no Yo-managed model target matches the captured disconnect selection")
    }
}

struct ExternalDisconnectPlan {
    credential_action: ExternalDisconnectCredentialAction,
    preference_after: Option<StartupTarget>,
    summary: String,
}

impl ExternalDisconnectPlan {
    fn prepare(
        snapshot: &ConnectionSnapshot,
        manual: &ModelCatalog,
        selection: &ModelSelection,
    ) -> Result<Self, AppError> {
        let removed = snapshot
            .managed_bindings()
            .iter()
            .find(|binding| binding.selection() == *selection)
            .ok_or_else(|| AppError::message("the selected managed binding no longer exists"))?;
        let prospective = snapshot
            .compose_catalog_after_managed_remove(manual, selection)
            .map_err(|error| {
                AppError::single("composing the post-disconnect model catalog", error)
            })?;
        let mut remaining = prospective
            .entries()
            .iter()
            .filter(|entry| same_account(entry, selection))
            .map(binding_summary)
            .collect::<Vec<_>>();
        remaining.sort();
        let credential_action = if remaining.is_empty() {
            ExternalDisconnectCredentialAction::Remove
        } else {
            ExternalDisconnectCredentialAction::Preserve
        };
        let preference_after =
            if snapshot.preference() == Some(&StartupTarget::Model(selection.clone())) {
                None
            } else {
                snapshot.preference().cloned()
            };
        let provenance = removed_provenance(manual, removed);
        let remaining_summary = if remaining.is_empty() {
            "  - none".to_owned()
        } else {
            format!("  - {}", remaining.join("\n  - "))
        };
        let preference_before = display_target(snapshot.preference());
        let preference_after_label = display_target(preference_after.as_ref());
        let startup_risk = if snapshot.preference()
            == Some(&StartupTarget::Model(selection.clone()))
        {
            "the stored startup default is cleared, so a new startup needs another admitted target"
        } else {
            "the stored startup default is unchanged"
        };
        let exact_binding_remains = prospective
            .resolve_model(selection.provider(), selection.account(), selection.model())
            .ok()
            .and_then(ModelCatalogEntry::complete_binding)
            == Some(removed.complete());
        let continuation_risk = if exact_binding_remains {
            "existing Sessions bound to this complete model can still resolve through the equal manual binding; durable Session history is retained"
        } else {
            "existing Sessions bound to this removed complete model may no longer resume natively; reconnect the exact binding or restore equivalent manual configuration; durable Session history is retained"
        };
        let summary = format!(
            "Disconnect managed target {}\nRemoved provenance: {provenance}\nExact removed binding:\n  - {}\nPreference: {preference_before} -> {preference_after_label}\nCredential action: {}\nRemaining bindings using Provider {} / Account {}:\n{remaining_summary}\nStartup risk: {startup_risk}\nContinuation risk: {continuation_risk}",
            selection.canonical_reference(),
            complete_binding_summary(removed.complete()),
            action_label(credential_action),
            selection.provider(),
            selection.account(),
        );
        Ok(Self {
            credential_action,
            preference_after,
            summary,
        })
    }
}

fn removed_provenance(manual: &ModelCatalog, removed: &ManagedConnectionBinding) -> &'static str {
    manual
        .entries()
        .iter()
        .find(|entry| selection_for_binding(entry.binding()) == removed.selection())
        .map_or(
            "managed (no manual binding remains at this coordinate)",
            |entry| {
                if entry.complete_binding() == Some(removed.complete()) {
                    "managed part of an equal manual+managed binding (manual remains)"
                } else {
                    "managed binding (a distinct manual binding remains at this coordinate)"
                }
            },
        )
}

fn same_account(entry: &ModelCatalogEntry, selection: &ModelSelection) -> bool {
    let binding = entry.binding();
    binding.provider_id() == selection.provider() && binding.account_id() == selection.account()
}

fn binding_summary(entry: &ModelCatalogEntry) -> String {
    entry.complete_binding().map_or_else(
        || {
            format!(
                "{} [manual legacy profile]",
                selection_for_binding(entry.binding()).canonical_reference()
            )
        },
        complete_binding_summary,
    )
}

const fn action_label(action: ExternalDisconnectCredentialAction) -> &'static str {
    match action {
        ExternalDisconnectCredentialAction::Preserve => "preserve",
        ExternalDisconnectCredentialAction::Remove => "remove",
    }
}

#[cfg(test)]
mod tests;
