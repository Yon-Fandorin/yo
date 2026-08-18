use serde_json::{Value, json};

use super::{
    ConnectorError, ConnectorFailureKind, FunctionTool, ReasoningEffort, ResponsesInputItem,
    ResponsesInputRole, ResponsesRequest,
};
use crate::{
    CompleteModelBinding,
    model_profile_admission::{AdmittedKimiProfile, admit_new_complete_binding},
};

mod replay;
mod tools;

const KIMI_ASSISTANT_SCHEMA: &str = "kimi.assistant-message/v1alpha1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum KimiWireProfile {
    PlatformK3 { effort: ReasoningEffort },
    PlatformK27Code,
    PlatformK26,
    CodeK3 { effort: ReasoningEffort },
    CodeK27,
}

impl KimiWireProfile {
    pub(super) const fn private_replay(self) -> bool {
        !matches!(self, Self::PlatformK26)
    }
}

pub(super) fn admit_binding(
    complete: &CompleteModelBinding,
) -> Result<KimiWireProfile, ConnectorError> {
    match admit_new_complete_binding(complete)
        .map_err(configuration_failure)?
        .kimi_profile()
    {
        Some(AdmittedKimiProfile::PlatformK3 { effort }) => {
            Ok(KimiWireProfile::PlatformK3 { effort })
        },
        Some(AdmittedKimiProfile::PlatformK27Code) => Ok(KimiWireProfile::PlatformK27Code),
        Some(AdmittedKimiProfile::PlatformK26) => Ok(KimiWireProfile::PlatformK26),
        Some(AdmittedKimiProfile::CodeK3 { effort }) => Ok(KimiWireProfile::CodeK3 { effort }),
        Some(AdmittedKimiProfile::CodeK27) => Ok(KimiWireProfile::CodeK27),
        None => Err(configuration_failure(
            "complete binding is outside the closed Kimi connector envelope",
        )),
    }
}

pub(super) fn wire_body(
    request: &ResponsesRequest,
    model: &str,
    profile: KimiWireProfile,
) -> Result<Value, ConnectorError> {
    let hard_max = match profile {
        KimiWireProfile::PlatformK3 { .. } | KimiWireProfile::CodeK3 { .. } => 131_072,
        KimiWireProfile::PlatformK27Code
        | KimiWireProfile::PlatformK26
        | KimiWireProfile::CodeK27 => 32_768,
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
    match profile {
        KimiWireProfile::PlatformK3 { effort } => {
            if request.reasoning_effort() != Some(effort) {
                return Err(configuration_failure(
                    "Kimi K3 request reasoning effort differs from its complete profile",
                ));
            }
            body["reasoning_effort"] = Value::String(effort.as_str().to_owned());
        },
        KimiWireProfile::PlatformK27Code => {
            if request.reasoning_effort().is_some() {
                return Err(configuration_failure(
                    "Kimi K2.7 Code request must omit reasoning_effort",
                ));
            }
            body["stream_options"] = json!({"include_usage": true});
            body["thinking"] = json!({"type": "enabled", "keep": "all"});
        },
        KimiWireProfile::PlatformK26 => {
            if request.reasoning_effort().is_some() {
                return Err(configuration_failure(
                    "Kimi K2.6 request must omit reasoning_effort",
                ));
            }
            body["stream_options"] = json!({"include_usage": true});
            body["thinking"] = json!({"type": "disabled"});
        },
        KimiWireProfile::CodeK3 { effort } => {
            if request.reasoning_effort() != Some(effort) {
                return Err(configuration_failure(
                    "Kimi Code K3 request reasoning effort differs from its complete profile",
                ));
            }
            body["reasoning_effort"] = Value::String(effort.as_str().to_owned());
            body["thinking"] = json!({"type": "enabled", "keep": "all"});
            body["prompt_cache_key"] = Value::String(required_cache_hint(request)?.to_owned());
        },
        KimiWireProfile::CodeK27 => {
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

fn required_cache_hint(request: &ResponsesRequest) -> Result<&str, ConnectorError> {
    request.cache_affinity_hint().ok_or_else(|| {
        configuration_failure("Kimi Code requests require one typed cache-affinity hint")
    })
}

fn configuration_failure(message: impl Into<String>) -> ConnectorError {
    ConnectorError::new(ConnectorFailureKind::Configuration, message)
}
