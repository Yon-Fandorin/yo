use std::path::Path;

use yo_core::{
    ConnectionSnapshot, ExternalDisconnectCredentialAction, ModelSelection, StartupPolicy,
    StartupSelectionSources, StartupTarget, StoredModelBinding, resolve_startup_target,
};

use super::{
    complete_binding_details, display_target,
    input::{ExternalDisconnectInput, TtyConnectionInput},
    operation_repositories,
    presentation::{
        Confirmation, DisconnectEffect, DisconnectImpact, DisconnectPreview, RemainingBinding,
        disconnect_success, display_model_item, escape_remote_text,
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
        .map_err(|error| AppError::single("capturing stored connections", error))?;
    let selection = select_stored_target(&snapshot, &command, input)?;
    let startup_policy = StartupPolicy::initial();
    let plan =
        ExternalDisconnectPlan::prepare(&snapshot, &selection, &startup_policy, command.verbose)?;
    let prepared = session
        .prepare_external_disconnect(snapshot.revision(), &selection, plan.credential_action)
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

    let target = escape_remote_text(&selection.canonical_reference());
    Ok(disconnect_success(
        &target,
        action_label(plan.credential_action),
        &display_target(plan.preference_after.as_ref()),
    ))
}

fn select_stored_target(
    snapshot: &ConnectionSnapshot,
    command: &DisconnectCommand,
    input: &mut impl ExternalDisconnectInput,
) -> Result<ModelSelection, AppError> {
    let mut candidates = snapshot
        .models()
        .iter()
        .map(StoredModelBinding::selection)
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
        0 => Err(AppError::message(
            "no stored model target matches the captured disconnect selection",
        )),
        _ if command.yes => Err(AppError::message(format!(
            "Provider {} Account {} has {} stored model targets; --yes never guesses which one to remove, so rerun interactively",
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
                        "disconnect target {:?} is not one of the captured stored targets",
                        selected.trim()
                    ))
                })
        },
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
        selection: &ModelSelection,
        startup_policy: &StartupPolicy,
        verbose: bool,
    ) -> Result<Self, AppError> {
        let removed = snapshot
            .models()
            .iter()
            .find(|binding| binding.selection() == *selection)
            .ok_or_else(|| AppError::message("the selected stored model no longer exists"))?;
        let prospective = snapshot
            .catalog_after_model_remove(selection)
            .map_err(|error| {
                AppError::single("composing the post-disconnect model catalog", error)
            })?;
        let mut remaining = snapshot
            .models()
            .iter()
            .filter(|binding| binding.selection() != *selection && same_account(binding, selection))
            .map(binding_summary)
            .collect::<Vec<_>>();
        remaining.sort();
        let catalog_seed_remains = snapshot.catalog_seeds().iter().any(|seed| {
            seed.provider() == selection.provider() && seed.account() == selection.account()
        });
        let credential_action = if remaining.is_empty() && !catalog_seed_remains {
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
                operator_target: None,
            },
        )
        .map_err(|error| AppError::single("resolving startup behavior after disconnect", error))?;
        let (new_sessions, new_sessions_ready) = match startup_after.as_ref() {
            Some(target) if !default_changed => {
                (format!("Keep using {}", display_target(Some(target))), true)
            },
            Some(target) => (format!("Use {}", display_target(Some(target))), true),
            None => ("No startup target remains".to_owned(), false),
        };
        let saved_sessions = "Unavailable until this exact model is restored; history stays";
        let api_key = match credential_action {
            ExternalDisconnectCredentialAction::Preserve if catalog_seed_remains => {
                "Keep — still used by the stored catalog definition".to_owned()
            },
            ExternalDisconnectCredentialAction::Preserve => {
                format!(
                    "Keep — still used by {}",
                    remaining
                        .iter()
                        .map(RemainingBinding::model)
                        .map(display_model_item)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            },
            ExternalDisconnectCredentialAction::Remove => format!(
                "Remove — no configured model uses {}:{}",
                selection.provider(),
                selection.account()
            ),
        };
        let preview = Confirmation::Disconnect(Box::new(DisconnectPreview::new(
            selection.canonical_reference(),
            "Stored connection".to_owned(),
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
                DisconnectEffect::attention(saved_sessions.to_owned()),
            ),
            remaining,
            verbose,
        )));
        Ok(Self {
            credential_action,
            preference_after,
            preview,
        })
    }
}

fn same_account(binding: &StoredModelBinding, selection: &ModelSelection) -> bool {
    let stored = binding.complete().binding();
    stored.provider_id() == selection.provider() && stored.account_id() == selection.account()
}

fn binding_summary(binding: &StoredModelBinding) -> RemainingBinding {
    RemainingBinding::new(selection_for_binding(binding.complete().binding()))
}

const fn action_label(action: ExternalDisconnectCredentialAction) -> &'static str {
    match action {
        ExternalDisconnectCredentialAction::Preserve => "Kept",
        ExternalDisconnectCredentialAction::Remove => "Removed",
    }
}

#[cfg(test)]
mod tests;
