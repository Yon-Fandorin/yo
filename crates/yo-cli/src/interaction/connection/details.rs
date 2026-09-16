use yo_core::CompleteModelBinding;

use super::{
    error::PresentationError,
    layout::{display_model_item, escape_remote_text, push_detail_field},
};
use crate::interaction::PresentationStyle;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct BindingDetails {
    pub(crate) model: String,
    pub(crate) profile: ProfileDetails,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct ProfileDetails {
    endpoint: String,
    protocol: String,
    connector: String,
    tokenizer: String,
    input_limit: u64,
    output_limit: Option<u64>,
    reasoning: String,
    request_options: String,
    tools: String,
    pub(crate) replay: String,
    image_input: Option<String>,
    image_accounting: Option<String>,
}

impl BindingDetails {
    pub(crate) fn escape_remote_model(&mut self, model_id: &str) {
        if self.model == model_id {
            self.model = escape_remote_text(&display_model_item(model_id));
        }
    }

    pub(crate) fn render(
        &self,
        output: &mut String,
        width: usize,
        style: PresentationStyle,
    ) -> Result<(), PresentationError> {
        push_detail_field(output, "Model", &self.model, width, style)?;
        self.profile.render(output, width, style)
    }
}

impl ProfileDetails {
    pub(crate) fn has_image_input(&self) -> bool {
        self.image_input.is_some()
    }

    pub(crate) fn render_image_disclosure(
        &self,
        output: &mut String,
        width: usize,
        style: PresentationStyle,
    ) -> Result<(), PresentationError> {
        let Some(image_input) = &self.image_input else {
            return Ok(());
        };
        push_detail_field(
            output,
            "Image input",
            "Enabled: normalized PNG snapshots",
            width,
            style,
        )?;
        push_detail_field(
            output,
            "Accounting",
            &format!(
                "Advisory estimate ({}), not measured provider usage",
                self.image_accounting
                    .as_deref()
                    .expect("admitted image accounting policy")
            ),
            width,
            style,
        )?;
        if image_input == yo_core::OPENROUTER_FREE_IMAGE_INPUT_PROFILE {
            push_detail_field(output, "Image endpoint", &self.endpoint, width, style)?;
            push_detail_field(
                output,
                "Free route",
                "NVIDIA only; no fallbacks; required parameters enforced; prompt, completion, request and image price caps are all 0 on every request",
                width,
                style,
            )?;
        }
        if image_input == yo_core::QWENCLOUD_GENERAL_IMAGE_INPUT_PROFILE {
            push_detail_field(output, "Image endpoint", &self.endpoint, width, style)?;
            push_detail_field(
                output,
                "Thinking",
                "Disabled; semantic replay only",
                width,
                style,
            )?;
            push_detail_field(
                output,
                "Metering",
                "General API is metered; free quota availability is determined by QwenCloud",
                width,
                style,
            )?;
        }
        Ok(())
    }

    pub(crate) fn render(
        &self,
        output: &mut String,
        width: usize,
        style: PresentationStyle,
    ) -> Result<(), PresentationError> {
        push_detail_field(output, "Endpoint", &self.endpoint, width, style)?;
        push_detail_field(output, "Protocol", &self.protocol, width, style)?;
        push_detail_field(output, "Connector", &self.connector, width, style)?;
        push_detail_field(
            output,
            "Limits",
            &format!(
                "{} input · {} max output tokens",
                readable_number(self.input_limit),
                self.output_limit
                    .map(readable_number)
                    .unwrap_or_else(|| "unknown".to_owned())
            ),
            width,
            style,
        )?;
        push_detail_field(output, "Tokenizer", &self.tokenizer, width, style)?;
        push_detail_field(output, "Tools", &self.tools, width, style)?;
        push_detail_field(output, "Replay", &self.replay, width, style)?;
        push_detail_field(output, "Reasoning", &self.reasoning, width, style)?;
        push_detail_field(
            output,
            "Request options",
            &self.request_options,
            width,
            style,
        )?;
        self.render_image_disclosure(output, width, style)
    }
}

pub(super) struct ProfileGroup<'a> {
    pub(crate) profile: &'a ProfileDetails,
    pub(crate) models: Vec<&'a str>,
}

pub(crate) fn group_profiles(bindings: &[BindingDetails]) -> Vec<ProfileGroup<'_>> {
    let mut groups: Vec<ProfileGroup<'_>> = Vec::new();
    for binding in bindings {
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group.profile == &binding.profile)
        {
            group.models.push(&binding.model);
        } else {
            groups.push(ProfileGroup {
                profile: &binding.profile,
                models: vec![&binding.model],
            });
        }
    }
    groups
}

impl From<&CompleteModelBinding> for BindingDetails {
    fn from(complete: &CompleteModelBinding) -> Self {
        let binding = complete.binding();
        let profile = complete.profile();
        Self {
            model: binding.model_id().to_string(),
            profile: ProfileDetails {
                endpoint: binding.endpoint().to_string(),
                protocol: binding.api_dialect().to_string(),
                connector: binding.connector_id().to_string(),
                tokenizer: profile.context().tokenizer_profile().to_owned(),
                input_limit: profile.context().input_token_limit(),
                output_limit: profile.context().max_output_tokens(),
                reasoning: profile.reasoning_parameters().to_json_value().to_string(),
                request_options: profile
                    .optional_request_parameters()
                    .to_json_value()
                    .to_string(),
                tools: profile.tool_capability_policy().to_string(),
                replay: profile.replay_profile().to_string(),
                image_input: profile.image_input_profile().map(ToString::to_string),
                image_accounting: profile
                    .image_accounting_policy()
                    .map(|policy| policy.to_string()),
            },
        }
    }
}

fn readable_number(value: u64) -> String {
    let digits = value.to_string();
    let mut output = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(character);
    }
    output
}
