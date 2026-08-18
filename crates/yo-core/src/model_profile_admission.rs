use crate::{
    ApiDialect, CompleteModelBinding, ConnectorId, EffectiveModelProfile,
    KIMI_PRIVATE_REPLAY_PROFILE, ReasoningEffort, SEMANTIC_REPLAY_PROFILE,
};

const LOCAL_TOOLS_PROFILE: &str = "local-tools/v1";
const NO_TOOLS_PROFILE: &str = "no-tools/v1";

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AdmittedKimiProfile {
    PlatformK3 { effort: ReasoningEffort },
    PlatformK27Code,
    PlatformK26,
    CodeK3 { effort: ReasoningEffort },
    CodeK27,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AdmittedReplayProfile {
    SemanticOnly,
    KimiPrivateLocalPlaintext,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AdmittedCompleteBinding {
    profile: AdmittedModelProfile,
    replay_profile: AdmittedReplayProfile,
    kimi_profile: Option<AdmittedKimiProfile>,
}

impl AdmittedCompleteBinding {
    pub(crate) const fn profile(self) -> AdmittedModelProfile {
        self.profile
    }

    pub(crate) const fn replay_profile(self) -> AdmittedReplayProfile {
        self.replay_profile
    }

    pub(crate) const fn kimi_profile(self) -> Option<AdmittedKimiProfile> {
        self.kimi_profile
    }
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
    let optional = profile.optional_request_parameters().to_json_value();
    let kimi = profile.api_dialect() == ApiDialect::KimiChatCompletions;
    let supported_optional = optional.as_object().is_some_and(|mapping| {
        mapping.is_empty()
            || kimi
                && (optional == serde_json::json!({"thinking": {"type": "enabled", "keep": "all"}})
                    || optional == serde_json::json!({"thinking": {"type": "disabled"}}))
    });
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
            Some("none") if !kimi => Some(ReasoningEffort::None),
            Some("low") if kimi => Some(ReasoningEffort::Low),
            Some("minimal") if !kimi => Some(ReasoningEffort::Minimal),
            Some("medium") if !kimi => Some(ReasoningEffort::Medium),
            Some("high") => Some(ReasoningEffort::High),
            Some("max") if kimi => Some(ReasoningEffort::Max),
            _ => {
                return Err(if kimi {
                    "Kimi reasoning_parameters.effort must be low, high, or max"
                } else {
                    "reasoning_parameters.effort must be none, minimal, medium, or high"
                }
                .to_owned());
            },
        }
    };
    Ok(AdmittedModelProfile {
        reasoning_effort,
        tool_policy,
    })
}

/// Admits one complete binding before preview, verification, persistence, or runtime use.
pub(crate) fn admit_new_complete_binding(
    complete: &CompleteModelBinding,
) -> Result<AdmittedCompleteBinding, String> {
    let binding = complete.binding();
    let profile = complete.profile();
    let admitted_profile = admit_explicit_model_profile(profile)?;
    let kimi_surface = binding.provider_id().as_str() == "kimi"
        || binding.connector_id().as_str() == ConnectorId::KIMI_CHAT_COMPLETIONS
        || binding.api_dialect() == ApiDialect::KimiChatCompletions
        || profile.api_dialect() == ApiDialect::KimiChatCompletions;
    if !kimi_surface {
        if profile.replay_profile().as_str() != SEMANTIC_REPLAY_PROFILE {
            return Err(
                "non-Kimi complete bindings require exact semantic-only/v1 replay".to_owned(),
            );
        }
        return Ok(AdmittedCompleteBinding {
            profile: admitted_profile,
            replay_profile: AdmittedReplayProfile::SemanticOnly,
            kimi_profile: None,
        });
    }

    if binding.provider_id().as_str() != "kimi"
        || binding.connector_id().as_str() != ConnectorId::KIMI_CHAT_COMPLETIONS
        || binding.api_dialect() != ApiDialect::KimiChatCompletions
        || profile.api_dialect() != ApiDialect::KimiChatCompletions
        || profile.context().tokenizer_profile() != "utf8-bytes/v1"
        || !matches!(
            profile.tool_capability_policy().as_str(),
            LOCAL_TOOLS_PROFILE | NO_TOOLS_PROFILE
        )
    {
        return Err("complete binding is outside the closed Kimi connector envelope".to_owned());
    }

    let input = profile.context().input_token_limit();
    let output = profile.context().max_output_tokens();
    let reasoning = profile.reasoning_parameters().to_json_value();
    let optional = profile.optional_request_parameters().to_json_value();
    let replay = profile.replay_profile().as_str();
    let endpoint = binding.endpoint().as_str();
    let model = binding.model_id().as_str();
    let kimi_profile = match (endpoint, model) {
        ("https://api.moonshot.ai/v1", "kimi-k3")
            if (131_073..=1_048_576).contains(&input)
                && output == Some(131_072)
                && optional == serde_json::json!({})
                && replay == KIMI_PRIVATE_REPLAY_PROFILE =>
        {
            let effort = reasoning
                .as_object()
                .filter(|mapping| mapping.len() == 1)
                .and_then(|mapping| mapping.get("effort"))
                .and_then(serde_json::Value::as_str)
                .and_then(|effort| match effort {
                    "low" => Some(ReasoningEffort::Low),
                    "high" => Some(ReasoningEffort::High),
                    "max" => Some(ReasoningEffort::Max),
                    _ => None,
                })
                .ok_or_else(|| "Kimi K3 requires exact low, high, or max reasoning".to_owned())?;
            AdmittedKimiProfile::PlatformK3 { effort }
        },
        ("https://api.moonshot.ai/v1", "kimi-k2.7-code" | "kimi-k2.7-code-highspeed")
            if (32_769..=262_144).contains(&input)
                && output == Some(32_768)
                && reasoning == serde_json::json!({})
                && optional
                    == serde_json::json!({"thinking": {"type": "enabled", "keep": "all"}})
                && replay == KIMI_PRIVATE_REPLAY_PROFILE =>
        {
            AdmittedKimiProfile::PlatformK27Code
        },
        ("https://api.moonshot.ai/v1", "kimi-k2.6")
            if (32_769..=262_144).contains(&input)
                && output == Some(32_768)
                && reasoning == serde_json::json!({})
                && optional == serde_json::json!({"thinking": {"type": "disabled"}})
                && replay == SEMANTIC_REPLAY_PROFILE =>
        {
            AdmittedKimiProfile::PlatformK26
        },
        ("https://api.kimi.com/coding/v1", "k3")
            if (262_144..=1_048_576).contains(&input)
                && output == Some(131_072)
                && optional
                    == serde_json::json!({"thinking": {"type": "enabled", "keep": "all"}})
                && replay == KIMI_PRIVATE_REPLAY_PROFILE =>
        {
            AdmittedKimiProfile::CodeK3 {
                effort: kimi_effort(&reasoning)?,
            }
        },
        ("https://api.kimi.com/coding/v1", "k3-256k")
            if input == 262_144
                && output == Some(131_072)
                && optional
                    == serde_json::json!({"thinking": {"type": "enabled", "keep": "all"}})
                && replay == KIMI_PRIVATE_REPLAY_PROFILE =>
        {
            AdmittedKimiProfile::CodeK3 {
                effort: kimi_effort(&reasoning)?,
            }
        },
        ("https://api.kimi.com/coding/v1", "kimi-for-coding" | "kimi-for-coding-highspeed")
            if input == 262_144
                && output == Some(32_768)
                && reasoning == serde_json::json!({})
                && optional
                    == serde_json::json!({"thinking": {"type": "enabled", "keep": "all"}})
                && replay == KIMI_PRIVATE_REPLAY_PROFILE =>
        {
            AdmittedKimiProfile::CodeK27
        },
        _ => {
            return Err("complete binding is not an admitted Kimi model profile".to_owned());
        },
    };
    let replay_profile = match kimi_profile {
        AdmittedKimiProfile::PlatformK26 => AdmittedReplayProfile::SemanticOnly,
        AdmittedKimiProfile::PlatformK3 { .. }
        | AdmittedKimiProfile::PlatformK27Code
        | AdmittedKimiProfile::CodeK3 { .. }
        | AdmittedKimiProfile::CodeK27 => AdmittedReplayProfile::KimiPrivateLocalPlaintext,
    };
    Ok(AdmittedCompleteBinding {
        profile: admitted_profile,
        replay_profile,
        kimi_profile: Some(kimi_profile),
    })
}

fn kimi_effort(reasoning: &serde_json::Value) -> Result<ReasoningEffort, String> {
    reasoning
        .as_object()
        .filter(|mapping| mapping.len() == 1)
        .and_then(|mapping| mapping.get("effort"))
        .and_then(serde_json::Value::as_str)
        .and_then(|effort| match effort {
            "low" => Some(ReasoningEffort::Low),
            "high" => Some(ReasoningEffort::High),
            "max" => Some(ReasoningEffort::Max),
            _ => None,
        })
        .ok_or_else(|| "Kimi K3 requires exact low, high, or max reasoning".to_owned())
}
