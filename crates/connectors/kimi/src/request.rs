use serde_json::{Value, json};
use yo_core::{
    ApiDialect, CompleteModelBinding, ConnectorError, ConnectorFailureKind, ConnectorId,
    FunctionTool, KIMI_PRIVATE_REPLAY_PROFILE, ModelConnectorInputItem, ModelConnectorInputRole,
    ModelConnectorRequest, ReasoningEffort, SEMANTIC_REPLAY_PROFILE,
};

mod replay;
mod tools;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct KimiWireProfile {
    pub(super) kind: KimiWireKind,
    local_tools: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum KimiWireKind {
    PlatformK3 { effort: ReasoningEffort },
    PlatformK27Code,
    PlatformK26,
    CodeK3 { effort: ReasoningEffort },
    CodeK27,
}

impl KimiWireProfile {
    pub(super) const fn private_replay(self) -> bool {
        !matches!(self.kind, KimiWireKind::PlatformK26)
    }
}

pub(super) fn admit_binding(
    complete: &CompleteModelBinding,
) -> Result<KimiWireProfile, ConnectorError> {
    let binding = complete.binding();
    let profile = complete.profile();
    if binding.provider_id().as_str() != "kimi"
        || binding.connector_id().as_str() != ConnectorId::KIMI_CHAT_COMPLETIONS
        || binding.api_dialect() != ApiDialect::KimiChatCompletions
        || profile.api_dialect() != ApiDialect::KimiChatCompletions
        || profile.context().tokenizer_profile() != "utf8-bytes/v1"
        || !matches!(
            profile.tool_capability_policy().as_str(),
            "local-tools/v1" | "no-tools/v1"
        )
    {
        return Err(configuration_failure(
            "complete binding is outside the closed Kimi connector envelope",
        ));
    }

    let input = profile.context().input_token_limit();
    let output = profile.context().max_output_tokens();
    let reasoning = profile.reasoning_parameters().to_json_value();
    let optional = profile.optional_request_parameters().to_json_value();
    let replay = profile.replay_profile().as_str();
    let endpoint = binding.endpoint().as_str();
    let model = binding.model_id().as_str();
    let kind = match (endpoint, model) {
        ("https://api.moonshot.ai/v1", "kimi-k3")
            if (131_073..=1_048_576).contains(&input)
                && output == Some(131_072)
                && optional == json!({})
                && replay == KIMI_PRIVATE_REPLAY_PROFILE =>
        {
            Ok(KimiWireKind::PlatformK3 {
                effort: kimi_effort(&reasoning)?,
            })
        },
        ("https://api.moonshot.ai/v1", "kimi-k2.7-code" | "kimi-k2.7-code-highspeed")
            if (32_769..=262_144).contains(&input)
                && output == Some(32_768)
                && reasoning == json!({})
                && optional == json!({"thinking": {"type": "enabled", "keep": "all"}})
                && replay == KIMI_PRIVATE_REPLAY_PROFILE =>
        {
            Ok(KimiWireKind::PlatformK27Code)
        },
        ("https://api.moonshot.ai/v1", "kimi-k2.6")
            if (32_769..=262_144).contains(&input)
                && output == Some(32_768)
                && reasoning == json!({})
                && optional == json!({"thinking": {"type": "disabled"}})
                && replay == SEMANTIC_REPLAY_PROFILE =>
        {
            Ok(KimiWireKind::PlatformK26)
        },
        ("https://api.kimi.com/coding/v1", "k3")
            if (262_144..=1_048_576).contains(&input)
                && output == Some(131_072)
                && optional == json!({"thinking": {"type": "enabled", "keep": "all"}})
                && replay == KIMI_PRIVATE_REPLAY_PROFILE =>
        {
            Ok(KimiWireKind::CodeK3 {
                effort: kimi_effort(&reasoning)?,
            })
        },
        ("https://api.kimi.com/coding/v1", "k3-256k")
            if input == 262_144
                && output == Some(131_072)
                && optional == json!({"thinking": {"type": "enabled", "keep": "all"}})
                && replay == KIMI_PRIVATE_REPLAY_PROFILE =>
        {
            Ok(KimiWireKind::CodeK3 {
                effort: kimi_effort(&reasoning)?,
            })
        },
        ("https://api.kimi.com/coding/v1", "kimi-for-coding" | "kimi-for-coding-highspeed")
            if input == 262_144
                && output == Some(32_768)
                && reasoning == json!({})
                && optional == json!({"thinking": {"type": "enabled", "keep": "all"}})
                && replay == KIMI_PRIVATE_REPLAY_PROFILE =>
        {
            Ok(KimiWireKind::CodeK27)
        },
        _ => Err(configuration_failure(
            "complete binding is not an admitted Kimi model profile",
        )),
    }?;
    Ok(KimiWireProfile {
        kind,
        local_tools: profile.tool_capability_policy().as_str() == "local-tools/v1",
    })
}

fn kimi_effort(reasoning: &Value) -> Result<ReasoningEffort, ConnectorError> {
    reasoning
        .as_object()
        .filter(|mapping| mapping.len() == 1)
        .and_then(|mapping| mapping.get("effort"))
        .and_then(Value::as_str)
        .and_then(|effort| match effort {
            "low" => Some(ReasoningEffort::Low),
            "high" => Some(ReasoningEffort::High),
            "max" => Some(ReasoningEffort::Max),
            _ => None,
        })
        .ok_or_else(|| configuration_failure("Kimi K3 requires exact low, high, or max reasoning"))
}

pub(super) fn wire_body(
    request: &ModelConnectorRequest,
    model: &str,
    profile: KimiWireProfile,
) -> Result<Value, ConnectorError> {
    let hard_max = match profile.kind {
        KimiWireKind::PlatformK3 { .. } | KimiWireKind::CodeK3 { .. } => 131_072,
        KimiWireKind::PlatformK27Code | KimiWireKind::PlatformK26 | KimiWireKind::CodeK27 => 32_768,
    };
    let Some(request_cap) = request.max_output_tokens() else {
        return Err(configuration_failure(
            "Kimi requests require a known positive output cap",
        ));
    };
    if request_cap > hard_max {
        return Err(configuration_failure(
            "Kimi request output cap exceeds its complete profile hard maximum",
        ));
    }
    let messages = replay::messages(request, profile)?;
    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "max_completion_tokens": request_cap,
    });
    match profile.kind {
        KimiWireKind::PlatformK3 { effort } => {
            if request.reasoning_effort() != Some(effort) {
                return Err(configuration_failure(
                    "Kimi K3 request reasoning effort differs from its complete profile",
                ));
            }
            body["reasoning_effort"] = Value::String(effort.as_str().to_owned());
        },
        KimiWireKind::PlatformK27Code => {
            if request.reasoning_effort().is_some() {
                return Err(configuration_failure(
                    "Kimi K2.7 Code request must omit reasoning_effort",
                ));
            }
            body["stream_options"] = json!({"include_usage": true});
            body["thinking"] = json!({"type": "enabled", "keep": "all"});
        },
        KimiWireKind::PlatformK26 => {
            if request.reasoning_effort().is_some() {
                return Err(configuration_failure(
                    "Kimi K2.6 request must omit reasoning_effort",
                ));
            }
            body["stream_options"] = json!({"include_usage": true});
            body["thinking"] = json!({"type": "disabled"});
        },
        KimiWireKind::CodeK3 { effort } => {
            if request.reasoning_effort() != Some(effort) {
                return Err(configuration_failure(
                    "Kimi Code K3 request reasoning effort differs from its complete profile",
                ));
            }
            body["reasoning_effort"] = Value::String(effort.as_str().to_owned());
            body["thinking"] = json!({"type": "enabled", "keep": "all"});
            body["prompt_cache_key"] = Value::String(required_cache_hint(request)?.to_owned());
        },
        KimiWireKind::CodeK27 => {
            if request.reasoning_effort().is_some() {
                return Err(configuration_failure(
                    "Kimi Code K2.7 request must omit reasoning_effort",
                ));
            }
            body["stream_options"] = json!({"include_usage": true});
            body["thinking"] = json!({"type": "enabled", "keep": "all"});
            body["prompt_cache_key"] = Value::String(required_cache_hint(request)?.to_owned());
        },
    }
    if request.tools().is_some() && !profile.local_tools {
        return Err(configuration_failure(
            "Kimi no-tools profile forbids request-local tool exposure",
        ));
    }
    if let Some(tools) = request.tools() {
        body["tools"] = Value::Array(
            tools
                .iter()
                .map(tools::strict_tool)
                .collect::<Result<Vec<_>, _>>()?,
        );
        body["tool_choice"] = Value::String("auto".to_owned());
    }
    Ok(body)
}

fn required_cache_hint(request: &ModelConnectorRequest) -> Result<&str, ConnectorError> {
    request.cache_affinity_hint().ok_or_else(|| {
        configuration_failure("Kimi Code requests require one typed cache-affinity hint")
    })
}

fn configuration_failure(message: impl Into<String>) -> ConnectorError {
    ConnectorError::new(ConnectorFailureKind::Configuration, message)
}
