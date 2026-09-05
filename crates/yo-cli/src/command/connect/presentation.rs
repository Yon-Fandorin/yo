use std::{collections::BTreeSet, num::NonZeroU16};

use yo_core::CredentialMutationAction;

use crate::{
    connection::presentation::{
        BindingDetails, ConfirmationView, PlanAction, PlanCounts, PresentationError,
        SuccessPresentation, group_profiles, plural, push_change, push_model_list_field,
        push_plan_summary, push_section_heading, push_title, render_success, trim_trailing_newline,
    },
    presentation::PresentationStyle,
};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StoredConnectionChange {
    Create,
    Update,
    Keep,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ImportPreview {
    account: String,
    added: Vec<String>,
    changed: Vec<String>,
    removed: Vec<String>,
    definition_changed: bool,
    account_transition: String,
    account_changed: bool,
    seed_transition: String,
    seed_changed: bool,
    resume_risk: Vec<String>,
    definition: String,
    credential_action: CredentialMutationAction,
    default_after: String,
    default_changed: bool,
    bindings: Vec<BindingDetails>,
    verbose: bool,
}

impl ImportPreview {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        account: String,
        added: Vec<String>,
        changed: Vec<String>,
        removed: Vec<String>,
        definition_changed: bool,
        account_transition: String,
        account_changed: bool,
        seed_transition: String,
        seed_changed: bool,
        resume_risk: Vec<String>,
        definition: String,
        credential_action: CredentialMutationAction,
        default_after: String,
        default_changed: bool,
        bindings: Vec<BindingDetails>,
        verbose: bool,
    ) -> Self {
        Self {
            account,
            added,
            changed,
            removed,
            definition_changed,
            account_transition,
            account_changed,
            seed_transition,
            seed_changed,
            resume_risk,
            definition,
            credential_action,
            default_after,
            default_changed,
            bindings,
            verbose,
        }
    }

    fn render(
        &self,
        width: NonZeroU16,
        style: PresentationStyle,
    ) -> Result<String, PresentationError> {
        let width = usize::from(width.get());
        let mut output = String::new();
        push_title(&mut output, "IMPORT", &self.account, width, style)?;
        output.push('\n');
        push_section_heading(
            &mut output,
            "Yo will replace this account definition:",
            width,
            style,
        )?;
        let mut counts = PlanCounts::default();
        let definition_action = if self.definition_changed {
            PlanAction::Change
        } else {
            PlanAction::Keep
        };
        push_change(
            &mut output,
            definition_action,
            "Definition",
            &self.definition,
            width,
            style,
        )?;
        counts.record(definition_action);
        let account_action = if self.account_changed {
            PlanAction::Change
        } else {
            PlanAction::Keep
        };
        push_change(
            &mut output,
            account_action,
            "Account metadata",
            &self.account_transition,
            width,
            style,
        )?;
        counts.record(account_action);
        let seed_action = if self.seed_changed {
            PlanAction::Change
        } else {
            PlanAction::Keep
        };
        push_change(
            &mut output,
            seed_action,
            "Catalog seed",
            &self.seed_transition,
            width,
            style,
        )?;
        counts.record(seed_action);
        for (action, label, models) in [
            (PlanAction::Add, "Add models", &self.added),
            (PlanAction::Change, "Change models", &self.changed),
            (PlanAction::Remove, "Remove models", &self.removed),
        ] {
            if !models.is_empty() {
                let values = models.iter().map(String::as_str).collect::<Vec<_>>();
                push_model_list_field(&mut output, label, &values, width, style)?;
                counts.record(action);
            }
        }
        if self.resume_risk.is_empty() {
            push_change(
                &mut output,
                PlanAction::Keep,
                "Saved Sessions",
                "No stored complete binding is changed or removed",
                width,
                style,
            )?;
            counts.record(PlanAction::Keep);
        } else {
            push_change(
                &mut output,
                PlanAction::Attention,
                "Saved Sessions",
                &format!(
                    "May not resume until each changed or removed exact binding is restored; history is kept: {}",
                    self.resume_risk.join(", ")
                ),
                width,
                style,
            )?;
        }
        let (credential_action, credential_detail) = match self.credential_action {
            CredentialMutationAction::Add => (PlanAction::Add, format!("Save {}", self.account)),
            CredentialMutationAction::Replace => {
                (PlanAction::Change, format!("Replace {}", self.account))
            },
            CredentialMutationAction::Remove => {
                return Err(PresentationError::InvalidPlan(
                    "definition import cannot remove a credential",
                ));
            },
        };
        push_change(
            &mut output,
            credential_action,
            "API key",
            &credential_detail,
            width,
            style,
        )?;
        counts.record(credential_action);
        let default_action = if self.default_changed {
            PlanAction::Change
        } else {
            PlanAction::Keep
        };
        push_change(
            &mut output,
            default_action,
            "Default model",
            &self.default_after,
            width,
            style,
        )?;
        counts.record(default_action);
        if self
            .bindings
            .iter()
            .any(|binding| binding.profile.replay == yo_core::KIMI_PRIVATE_REPLAY_PROFILE)
        {
            push_change(
                &mut output,
                PlanAction::Add,
                "Private replay",
                "Retain bounded Kimi assistant state unencrypted in local current-user Session records",
                width,
                style,
            )?;
            counts.record(PlanAction::Add);
        }
        if self.verbose && !self.bindings.is_empty() {
            output.push('\n');
            for (index, group) in group_profiles(&self.bindings).iter().enumerate() {
                if index > 0 {
                    output.push('\n');
                }
                push_section_heading(
                    &mut output,
                    &format!("Imported profile {}", index + 1),
                    width,
                    style,
                )?;
                push_model_list_field(
                    &mut output,
                    &format!("Models ({})", group.models.len()),
                    &group.models,
                    width,
                    style,
                )?;
                group.profile.render(&mut output, width, style)?;
            }
        }
        output.push('\n');
        push_plan_summary(&mut output, &counts, width, style)?;
        trim_trailing_newline(&mut output);
        Ok(output)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ConnectPreview {
    target: String,
    account: String,
    default_after: String,
    stored_change: StoredConnectionChange,
    credential_action: CredentialMutationAction,
    default_changed: bool,
    verbose: bool,
    bindings: Vec<BindingDetails>,
}

impl ConnectPreview {
    pub(super) fn new(
        target: String,
        account: String,
        default_after: String,
        stored_change: StoredConnectionChange,
        credential_action: CredentialMutationAction,
        default_changed: bool,
        bindings: Vec<BindingDetails>,
    ) -> Self {
        Self {
            target,
            account,
            default_after,
            stored_change,
            credential_action,
            default_changed,
            verbose: false,
            bindings,
        }
    }

    pub(super) const fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    fn render(
        &self,
        width: NonZeroU16,
        style: PresentationStyle,
    ) -> Result<String, PresentationError> {
        let width = usize::from(width.get());
        let mut output = String::new();
        push_title(&mut output, "CONNECT", &self.target, width, style)?;
        output.push('\n');
        push_section_heading(&mut output, "Yo will make these changes:", width, style)?;
        let mut counts = PlanCounts::default();
        let (stored_action, stored_detail) = match self.stored_change {
            StoredConnectionChange::Create => (PlanAction::Add, format!("Create {}", self.target)),
            StoredConnectionChange::Update => {
                (PlanAction::Change, format!("Update {}", self.target))
            },
            StoredConnectionChange::Keep => (PlanAction::Keep, format!("Keep {}", self.target)),
        };
        push_change(
            &mut output,
            stored_action,
            "Stored connection",
            &stored_detail,
            width,
            style,
        )?;
        counts.record(stored_action);
        let registered_models = self
            .bindings
            .iter()
            .map(|binding| binding.model.as_str())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let registration_detail = format!(
            "register {} {}",
            registered_models.len(),
            plural(registered_models.len(), "model", "models")
        );
        let (credential_action, credential_detail) = match self.credential_action {
            CredentialMutationAction::Add => (
                PlanAction::Add,
                format!("Save {} · {registration_detail}", self.account),
            ),
            CredentialMutationAction::Replace => (
                PlanAction::Change,
                format!("Replace {} · {registration_detail}", self.account),
            ),
            CredentialMutationAction::Remove => {
                return Err(PresentationError::InvalidPlan(
                    "connect cannot prepare credential removal",
                ));
            },
        };
        push_change(
            &mut output,
            credential_action,
            "API key",
            &credential_detail,
            width,
            style,
        )?;
        push_model_list_field(&mut output, "Models", &registered_models, width, style)?;
        counts.record(credential_action);
        if self
            .bindings
            .iter()
            .any(|binding| binding.profile.replay == yo_core::KIMI_PRIVATE_REPLAY_PROFILE)
        {
            push_change(
                &mut output,
                PlanAction::Add,
                "Private replay",
                "Retain bounded Kimi assistant state unencrypted in local current-user Session records",
                width,
                style,
            )?;
            counts.record(PlanAction::Add);
        }
        let default_action = if self.default_changed {
            PlanAction::Change
        } else {
            PlanAction::Keep
        };
        push_change(
            &mut output,
            default_action,
            "Default model",
            &self.default_after,
            width,
            style,
        )?;
        counts.record(default_action);
        if self.verbose {
            output.push('\n');
            let groups = group_profiles(&self.bindings);
            let multiple = groups.len() > 1;
            for (index, group) in groups.iter().enumerate() {
                if index > 0 {
                    output.push('\n');
                }
                let heading = if multiple {
                    format!("Connection profile {} of {}", index + 1, groups.len())
                } else {
                    "Connection profile".to_owned()
                };
                push_section_heading(&mut output, &heading, width, style)?;
                push_model_list_field(
                    &mut output,
                    &format!("Models ({})", group.models.len()),
                    &group.models,
                    width,
                    style,
                )?;
                group.profile.render(&mut output, width, style)?;
            }
        }
        output.push('\n');
        push_plan_summary(&mut output, &counts, width, style)?;
        trim_trailing_newline(&mut output);
        Ok(output)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Confirmation {
    Connect(Box<ConnectPreview>),
    Import(Box<ImportPreview>),
}

impl Confirmation {
    #[cfg(test)]
    pub(super) fn render(&self, width: NonZeroU16) -> Result<String, PresentationError> {
        self.render_styled(width, PresentationStyle::Plain)
    }
}

impl ConfirmationView for Confirmation {
    fn render_styled(
        &self,
        width: NonZeroU16,
        style: PresentationStyle,
    ) -> Result<String, PresentationError> {
        match self {
            Self::Connect(preview) => preview.render(width, style),
            Self::Import(preview) => preview.render(width, style),
        }
    }

    fn prompt(&self) -> &'static str {
        match self {
            Self::Connect(_) => "Apply this connection plan? [y/N] ",
            Self::Import(_) => "Import this connection definition? [y/N] ",
        }
    }
}

pub(super) fn connect_success(
    target: &str,
    registered: usize,
    default: &str,
) -> Result<String, PresentationError> {
    connect_success_with(
        SuccessPresentation::for_stdout(),
        target,
        registered,
        default,
    )
}

fn connect_success_with(
    presentation: SuccessPresentation,
    target: &str,
    registered: usize,
    default: &str,
) -> Result<String, PresentationError> {
    render_success(
        presentation,
        "Connected",
        12,
        &[
            ("Model", target.to_owned()),
            (
                "Registered",
                format!(
                    "{registered} model {}",
                    plural(registered, "profile", "profiles")
                ),
            ),
            ("Default", default.to_owned()),
        ],
    )
}

pub(super) fn import_success(
    account: &str,
    registered: usize,
    default: &str,
) -> Result<String, PresentationError> {
    import_success_with(
        SuccessPresentation::for_stdout(),
        account,
        registered,
        default,
    )
}

fn import_success_with(
    presentation: SuccessPresentation,
    account: &str,
    registered: usize,
    default: &str,
) -> Result<String, PresentationError> {
    render_success(
        presentation,
        "Imported",
        12,
        &[
            ("Account", account.to_owned()),
            (
                "Registered",
                format!(
                    "{registered} model {}",
                    plural(registered, "profile", "profiles")
                ),
            ),
            ("Default", default.to_owned()),
        ],
    )
}
