use std::path::Path;

use yo_core::{
    ConnectionSnapshot, ExternalDisconnectCredentialAction, ManagedConnectionBinding, ModelCatalog,
    ModelCatalogEntry, ModelSelection, StartupPolicy, StartupSelectionSources, StartupTarget,
    resolve_startup_target,
};

use super::{
    complete_binding_details, display_target,
    input::{ExternalDisconnectInput, TtyConnectionInput},
    operation_repositories,
    presentation::{
        Confirmation, DisconnectEffect, DisconnectImpact, DisconnectPreview, RemainingBinding,
        disconnect_success,
    },
    selection_for_binding,
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
    let startup_policy = StartupPolicy::initial();
    let plan = ExternalDisconnectPlan::prepare(
        &snapshot,
        config.model_catalog(),
        &selection,
        &startup_policy,
        config.startup_target().cloned(),
    )?;
    let prepared = session
        .prepare_external_disconnect(
            config.snapshot_digest(),
            snapshot.revision(),
            &selection,
            plan.credential_action,
        )
        .map_err(|error| AppError::single("preparing the external disconnect", error))?;

    if !command.yes && !input.confirm(&plan.preview)? {
        return Ok("Disconnect cancelled; nothing changed.\n".to_owned());
    }
    config
        .verify_unchanged()
        .map_err(|error| AppError::single("guarding Yo configuration", error))?;
    session
        .commit_external_disconnect(prepared)
        .map_err(|error| AppError::single("publishing the external disconnect", error))?;

    Ok(disconnect_success(
        &selection.canonical_reference(),
        action_label(plan.credential_action),
        &display_target(plan.preference_after.as_ref()),
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
    preview: Confirmation,
}

impl ExternalDisconnectPlan {
    fn prepare(
        snapshot: &ConnectionSnapshot,
        manual: &ModelCatalog,
        selection: &ModelSelection,
        startup_policy: &StartupPolicy,
        operator_target: Option<StartupTarget>,
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
        let preference_before = display_target(snapshot.preference());
        let preference_after_label = display_target(preference_after.as_ref());
        let default_changed = preference_before != preference_after_label;
        let default_change = if !default_changed {
            format!("Keep {preference_after_label}")
        } else {
            format!("{preference_before}  →  {preference_after_label}")
        };
        let startup_after = resolve_startup_target(
            &prospective,
            startup_policy,
            StartupSelectionSources {
                invocation: None,
                stored_preference: preference_after.clone(),
                operator_target,
            },
        )
        .map_err(|error| AppError::single("resolving startup behavior after disconnect", error))?;
        let (new_sessions, new_sessions_ready) = match startup_after.as_ref() {
            Some(target) if !default_changed => {
                (format!("Keep using {}", display_target(Some(target))), true)
            },
            Some(target) => (format!("Use {}", display_target(Some(target))), true),
            None => (
                "No startup target remains; configure or select one before starting a new session"
                    .to_owned(),
                false,
            ),
        };
        let exact_binding_remains = prospective
            .resolve_model(selection.provider(), selection.account(), selection.model())
            .ok()
            .and_then(ModelCatalogEntry::complete_binding)
            == Some(removed.complete());
        let saved_sessions = if exact_binding_remains {
            "Can resume through the equal manual configuration; history is kept"
        } else {
            "May not resume until this exact model is restored; history is kept"
        };
        let api_key = match credential_action {
            ExternalDisconnectCredentialAction::Preserve => format!(
                "Keep it because another configured model still uses {}:{}",
                selection.provider(),
                selection.account()
            ),
            ExternalDisconnectCredentialAction::Remove => format!(
                "Remove it because no configured model still uses {}:{}",
                selection.provider(),
                selection.account()
            ),
        };
        let preview = Confirmation::Disconnect(Box::new(DisconnectPreview::new(
            selection.canonical_reference(),
            provenance.to_owned(),
            complete_binding_details(removed.complete()),
            DisconnectImpact::new(
                if default_changed {
                    DisconnectEffect::change(default_change)
                } else {
                    DisconnectEffect::keep(default_change)
                },
                if credential_action == ExternalDisconnectCredentialAction::Remove {
                    DisconnectEffect::remove(api_key)
                } else {
                    DisconnectEffect::keep(api_key)
                },
                if new_sessions_ready {
                    DisconnectEffect::ready(new_sessions)
                } else {
                    DisconnectEffect::attention(new_sessions)
                },
                if exact_binding_remains {
                    DisconnectEffect::ready(saved_sessions.to_owned())
                } else {
                    DisconnectEffect::attention(saved_sessions.to_owned())
                },
            ),
            remaining,
        )));
        Ok(Self {
            credential_action,
            preference_after,
            preview,
        })
    }
}

fn removed_provenance(manual: &ModelCatalog, removed: &ManagedConnectionBinding) -> &'static str {
    manual
        .entries()
        .iter()
        .find(|entry| selection_for_binding(entry.binding()) == removed.selection())
        .map_or(
            "Managed connection only; no manual configuration remains for this model",
            |entry| {
                if entry.complete_binding() == Some(removed.complete()) {
                    "Managed copy removed; equal manual configuration remains"
                } else {
                    "Managed connection removed; different manual configuration remains"
                }
            },
        )
}

fn same_account(entry: &ModelCatalogEntry, selection: &ModelSelection) -> bool {
    let binding = entry.binding();
    binding.provider_id() == selection.provider() && binding.account_id() == selection.account()
}

fn binding_summary(entry: &ModelCatalogEntry) -> RemainingBinding {
    entry.complete_binding().map_or_else(
        || RemainingBinding::legacy(selection_for_binding(entry.binding())),
        |complete| RemainingBinding::complete(selection_for_binding(complete.binding())),
    )
}

const fn action_label(action: ExternalDisconnectCredentialAction) -> &'static str {
    match action {
        ExternalDisconnectCredentialAction::Preserve => "Kept",
        ExternalDisconnectCredentialAction::Remove => "Removed",
    }
}

#[cfg(test)]
mod tests;
