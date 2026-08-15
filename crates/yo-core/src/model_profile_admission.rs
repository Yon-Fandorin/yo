use crate::{EffectiveModelProfile, ReasoningEffort};

const LOCAL_TOOLS_PROFILE: &str = "local-tools/v1";
const NO_TOOLS_PROFILE: &str = "no-tools/v1";
const SEMANTIC_TERMINAL_PROFILE: &str = "semantic-terminal/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AdmittedToolPolicy {
    LocalTools,
    NoTools,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AdmittedModelProfile {
    reasoning_effort: Option<ReasoningEffort>,
    tool_policy: AdmittedToolPolicy,
}

impl AdmittedModelProfile {
    pub(crate) const fn reasoning_effort(self) -> Option<ReasoningEffort> {
        self.reasoning_effort
    }

    pub(crate) const fn tool_policy(self) -> AdmittedToolPolicy {
        self.tool_policy
    }
}

/// Admits the resolved profile fields currently implemented by the native model path.
pub(crate) fn admit_explicit_model_profile(
    profile: &EffectiveModelProfile,
) -> Result<AdmittedModelProfile, String> {
    if !profile.optional_request_parameters().is_empty_mapping() {
        return Err(
            "optional_request_parameters is not yet supported by the native model loop".to_owned(),
        );
    }
    let tool_policy = match profile.tool_capability_policy().as_str() {
        LOCAL_TOOLS_PROFILE => AdmittedToolPolicy::LocalTools,
        NO_TOOLS_PROFILE => AdmittedToolPolicy::NoTools,
        value => {
            return Err(format!(
                "unsupported tool_capability_policy {value:?}; expected {LOCAL_TOOLS_PROFILE} or {NO_TOOLS_PROFILE}"
            ));
        },
    };
    if profile.verification_profile().as_str() != SEMANTIC_TERMINAL_PROFILE {
        return Err(format!(
            "unsupported verification_profile {:?}; expected {SEMANTIC_TERMINAL_PROFILE}",
            profile.verification_profile().as_str()
        ));
    }

    let value = profile.reasoning_parameters().to_json_value();
    let serde_json::Value::Object(parameters) = value else {
        return Err(
            "reasoning_parameters must be a mapping supported by the native model loop".to_owned(),
        );
    };
    let reasoning_effort = if parameters.is_empty() {
        None
    } else {
        if parameters.len() != 1 {
            return Err("reasoning_parameters supports only the effort field".to_owned());
        }
        match parameters.get("effort").and_then(|value| value.as_str()) {
            Some("none") => Some(ReasoningEffort::None),
            Some("minimal") => Some(ReasoningEffort::Minimal),
            Some("medium") => Some(ReasoningEffort::Medium),
            Some("high") => Some(ReasoningEffort::High),
            _ => {
                return Err(
                    "reasoning_parameters.effort must be none, minimal, medium, or high".to_owned(),
                );
            },
        }
    };
    Ok(AdmittedModelProfile {
        reasoning_effort,
        tool_policy,
    })
}
