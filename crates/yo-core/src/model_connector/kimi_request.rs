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
    K3 { effort: ReasoningEffort },
    K27Code,
    K26,
}

impl KimiWireProfile {
    pub(super) const fn private_replay(self) -> bool {
        !matches!(self, Self::K26)
    }
}

pub(super) fn admit_binding(
    complete: &CompleteModelBinding,
) -> Result<KimiWireProfile, ConnectorError> {
    match admit_new_complete_binding(complete)
        .map_err(configuration_failure)?
        .kimi_profile()
    {
        Some(AdmittedKimiProfile::K3 { effort }) => Ok(KimiWireProfile::K3 { effort }),
        Some(AdmittedKimiProfile::K27Code) => Ok(KimiWireProfile::K27Code),
        Some(AdmittedKimiProfile::K26) => Ok(KimiWireProfile::K26),
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
    if request.max_output_tokens()
        != match profile {
            KimiWireProfile::K3 { .. } => 131_072,
            KimiWireProfile::K27Code | KimiWireProfile::K26 => 32_768,
        }
    {
        return Err(configuration_failure(
            "Kimi request output cap does not match its complete profile",
        ));
    }
    let messages = replay::messages(request, profile)?;
    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "max_completion_tokens": request.max_output_tokens(),
    });
    match profile {
        KimiWireProfile::K3 { effort } => {
            if request.reasoning_effort() != Some(effort) {
                return Err(configuration_failure(
                    "Kimi K3 request reasoning effort differs from its complete profile",
                ));
            }
            body["reasoning_effort"] = Value::String(effort.as_str().to_owned());
        },
        KimiWireProfile::K27Code => {
            if request.reasoning_effort().is_some() {
                return Err(configuration_failure(
                    "Kimi K2.7 Code request must omit reasoning_effort",
                ));
            }
            body["stream_options"] = json!({"include_usage": true});
            body["thinking"] = json!({"type": "enabled", "keep": "all"});
        },
        KimiWireProfile::K26 => {
            if request.reasoning_effort().is_some() {
                return Err(configuration_failure(
                    "Kimi K2.6 request must omit reasoning_effort",
                ));
            }
            body["stream_options"] = json!({"include_usage": true});
            body["thinking"] = json!({"type": "disabled"});
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

fn configuration_failure(message: impl Into<String>) -> ConnectorError {
    ConnectorError::new(ConnectorFailureKind::Configuration, message)
}
