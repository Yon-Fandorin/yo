use crate::{EffectiveModelProfile, ReasoningEffort};

const LOCAL_TOOLS_PROFILE: &str = "local-tools/v1";
const SEMANTIC_TERMINAL_PROFILE: &str = "semantic-terminal/v1";

/// Admits the resolved profile fields currently implemented by the native model path.
pub(crate) fn admit_explicit_model_profile(
    profile: &EffectiveModelProfile,
) -> Result<Option<ReasoningEffort>, String> {
    if !profile.optional_request_parameters().is_empty_mapping() {
        return Err(
            "optional_request_parameters is not yet supported by the native model loop".to_owned(),
        );
    }
    if profile.tool_capability_policy().as_str() != LOCAL_TOOLS_PROFILE {
        return Err(format!(
            "unsupported tool_capability_policy {:?}; expected {LOCAL_TOOLS_PROFILE}",
            profile.tool_capability_policy().as_str()
        ));
    }
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
    if parameters.is_empty() {
        return Ok(None);
    }
    if parameters.len() != 1 {
        return Err("reasoning_parameters supports only the effort field".to_owned());
    }
    match parameters.get("effort").and_then(|value| value.as_str()) {
        Some("none") => Ok(Some(ReasoningEffort::None)),
        Some("minimal") => Ok(Some(ReasoningEffort::Minimal)),
        Some("medium") => Ok(Some(ReasoningEffort::Medium)),
        Some("high") => Ok(Some(ReasoningEffort::High)),
        _ => Err("reasoning_parameters.effort must be none, minimal, medium, or high".to_owned()),
    }
}
