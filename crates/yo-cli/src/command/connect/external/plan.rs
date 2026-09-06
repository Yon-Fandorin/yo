use yo_core::{
    CompleteModelBinding, ConnectionAccount, ConnectionSnapshot, ModelCatalog, ModelCatalogEntry,
    ModelSelection, PreparedConnectionMutation, StartupPolicy, StartupSelectionSources,
    StartupTarget, StoredModelBinding, resolve_startup_target,
};

use super::super::presentation::StoredConnectionChange;
#[cfg(test)]
use super::super::presentation::{Confirmation, ConnectPreview};
use crate::{
    AppError,
    state::connection::{complete_binding_details, display_target},
};

pub(super) struct ExternalConnectPlan {
    pub(super) connection: PreparedConnectionMutation,
    pub(super) bindings: Vec<CompleteModelBinding>,
    pub(super) binding_count: usize,
    pub(super) preference: Option<StartupTarget>,
    pub(super) target: String,
    pub(super) account: String,
    pub(super) default_after: String,
    pub(super) stored_change: StoredConnectionChange,
    pub(super) default_changed: bool,
    pub(super) binding_details: Vec<crate::interaction::connection::BindingDetails>,
}

impl ExternalConnectPlan {
    pub(super) fn escape_remote_model(&mut self, model_id: &str) {
        for details in &mut self.binding_details {
            details.escape_remote_model(model_id);
        }
    }

    pub(super) fn prepare(
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
    pub(super) fn preview(
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
