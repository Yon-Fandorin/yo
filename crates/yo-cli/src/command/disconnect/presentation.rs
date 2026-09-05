use std::num::NonZeroU16;

use yo_core::ModelSelection;

use crate::interaction::{
    PresentationStyle,
    connection::{
        BindingDetails, ConfirmationView, PlanAction, PlanCounts, PresentationError,
        SuccessPresentation, display_model_item, push_bullet, push_change, push_detail_field,
        push_plan_summary, push_section_heading, push_title, render_success, trim_trailing_newline,
    },
};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct RemainingBinding {
    model: String,
}

impl RemainingBinding {
    pub(super) fn new(selection: ModelSelection) -> Self {
        Self {
            model: selection.model().to_string(),
        }
    }

    pub(super) fn model(&self) -> &str {
        &self.model
    }

    fn render(
        &self,
        output: &mut String,
        width: usize,
        style: PresentationStyle,
    ) -> Result<(), PresentationError> {
        push_bullet(output, &display_model_item(&self.model), width, style)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DisconnectPreview {
    target: String,
    stored_source: String,
    removed: BindingDetails,
    impact: DisconnectImpact,
    remaining: Vec<RemainingBinding>,
    verbose: bool,
}

impl DisconnectPreview {
    pub(super) fn new(
        target: String,
        stored_source: String,
        removed: BindingDetails,
        impact: DisconnectImpact,
        remaining: Vec<RemainingBinding>,
        verbose: bool,
    ) -> Self {
        Self {
            target,
            stored_source,
            removed,
            impact,
            remaining,
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
        push_title(&mut output, "DISCONNECT", &self.target, width, style)?;
        output.push('\n');
        push_section_heading(&mut output, "Yo will make these changes:", width, style)?;
        let mut counts = PlanCounts::default();
        push_change(
            &mut output,
            PlanAction::Remove,
            "Stored connection",
            &format!("Remove {}", self.target),
            width,
            style,
        )?;
        counts.record(PlanAction::Remove);
        push_change(
            &mut output,
            self.impact.default.action,
            "Default model",
            &self.impact.default.detail,
            width,
            style,
        )?;
        counts.record(self.impact.default.action);
        push_change(
            &mut output,
            self.impact.api_key.action,
            "API key",
            &self.impact.api_key.detail,
            width,
            style,
        )?;
        counts.record(self.impact.api_key.action);
        push_change(
            &mut output,
            self.impact.new_sessions.action,
            "New sessions",
            &self.impact.new_sessions.detail,
            width,
            style,
        )?;
        push_change(
            &mut output,
            self.impact.saved_sessions.action,
            "Saved sessions",
            &self.impact.saved_sessions.detail,
            width,
            style,
        )?;
        if self.verbose {
            output.push('\n');
            push_section_heading(&mut output, "Connection being removed", width, style)?;
            push_detail_field(&mut output, "Source", &self.stored_source, width, style)?;
            self.removed.render(&mut output, width, style)?;
            output.push('\n');
            push_section_heading(
                &mut output,
                &format!(
                    "Still available for this account ({})",
                    self.remaining.len()
                ),
                width,
                style,
            )?;
            if self.remaining.is_empty() {
                push_bullet(&mut output, "None", width, style)?;
            } else {
                for (index, binding) in self.remaining.iter().enumerate() {
                    if index > 0 {
                        output.push('\n');
                    }
                    binding.render(&mut output, width, style)?;
                }
            }
        }
        output.push('\n');
        push_plan_summary(&mut output, &counts, width, style)?;
        trim_trailing_newline(&mut output);
        Ok(output)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DisconnectImpact {
    default: DisconnectEffect,
    api_key: DisconnectEffect,
    new_sessions: DisconnectEffect,
    saved_sessions: DisconnectEffect,
}

impl DisconnectImpact {
    pub(super) fn new(
        default: DisconnectEffect,
        api_key: DisconnectEffect,
        new_sessions: DisconnectEffect,
        saved_sessions: DisconnectEffect,
    ) -> Self {
        Self {
            default,
            api_key,
            new_sessions,
            saved_sessions,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DisconnectEffect {
    detail: String,
    action: PlanAction,
}

impl DisconnectEffect {
    pub(super) fn change(detail: String) -> Self {
        Self {
            detail,
            action: PlanAction::Change,
        }
    }

    pub(super) fn remove(detail: String) -> Self {
        Self {
            detail,
            action: PlanAction::Remove,
        }
    }

    pub(super) fn keep(detail: String) -> Self {
        Self {
            detail,
            action: PlanAction::Keep,
        }
    }

    pub(super) fn ready(detail: String) -> Self {
        Self {
            detail,
            action: PlanAction::Success,
        }
    }

    pub(super) fn attention(detail: String) -> Self {
        Self {
            detail,
            action: PlanAction::Attention,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Confirmation {
    Disconnect(Box<DisconnectPreview>),
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
            Self::Disconnect(preview) => preview.render(width, style),
        }
    }

    fn prompt(&self) -> &'static str {
        "Apply this disconnect plan? [y/N] "
    }
}

pub(super) fn disconnect_success(
    target: &str,
    api_key: &str,
    default: &str,
) -> Result<String, PresentationError> {
    disconnect_success_with(SuccessPresentation::for_stdout(), target, api_key, default)
}

fn disconnect_success_with(
    presentation: SuccessPresentation,
    target: &str,
    api_key: &str,
    default: &str,
) -> Result<String, PresentationError> {
    render_success(
        presentation,
        "Disconnected",
        9,
        &[
            ("Model", target.to_owned()),
            ("API key", api_key.to_owned()),
            ("Default", default.to_owned()),
        ],
    )
}
