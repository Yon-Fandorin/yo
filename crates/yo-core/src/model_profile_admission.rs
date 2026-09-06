use crate::{
    AdmittedCompleteBinding, AdmittedModelProfile, AdmittedReplayProfile, AdmittedToolPolicy,
    ApiDialect, CompleteModelBinding, ConnectorId, EffectiveModelProfile, ReasoningEffort,
    SEMANTIC_REPLAY_PROFILE,
};

const LOCAL_TOOLS_PROFILE: &str = "local-tools/v1";
const NO_TOOLS_PROFILE: &str = "no-tools/v1";

/// Admits the resolved profile fields currently implemented by the native model path.
pub(crate) fn admit_explicit_model_profile(
    profile: &EffectiveModelProfile,
) -> Result<AdmittedModelProfile, String> {
    let optional = profile.optional_request_parameters().to_json_value();
    let supported_optional = optional
        .as_object()
        .is_some_and(|mapping| mapping.is_empty());
    if !supported_optional {
        return Err(
            "optional_request_parameters is not supported by the native model loop".to_owned(),
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
    Ok(AdmittedModelProfile::new(reasoning_effort, tool_policy))
}

/// Admits the generic native profile implemented by the OpenAI-dialect connectors.
///
/// Kimi requires its connector-owned admission implementation; this function never
/// interprets or falls back for a Kimi Provider, Connector, or dialect.
pub fn admit_standard_complete_binding(
    complete: &CompleteModelBinding,
) -> Result<AdmittedCompleteBinding, String> {
    let binding = complete.binding();
    let profile = complete.profile();
    if binding.provider_id().as_str() == "kimi"
        || binding.connector_id().as_str() == ConnectorId::KIMI_CHAT_COMPLETIONS
        || binding.api_dialect() == ApiDialect::KimiChatCompletions
        || profile.api_dialect() == ApiDialect::KimiChatCompletions
    {
        return Err("Kimi complete bindings require connector-owned admission".to_owned());
    }
    let admitted_profile = admit_explicit_model_profile(profile)?;
    if profile.replay_profile().as_str() != SEMANTIC_REPLAY_PROFILE {
        return Err("non-Kimi complete bindings require exact semantic-only/v1 replay".to_owned());
    }
    Ok(AdmittedCompleteBinding::new(
        admitted_profile,
        AdmittedReplayProfile::SemanticOnly,
    ))
}
