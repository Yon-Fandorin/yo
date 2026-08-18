use std::{collections::HashSet, fmt};

use serde_json::{Value, json};

use super::{ConnectorError, ConnectorFailureKind};
use crate::{KimiAssistantMessage, SessionId};

const MAX_WIRE_ID_BYTES: usize = 256;
const MAX_TOOL_DESCRIPTION_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponsesInputRole {
    System,
    Developer,
    User,
    Assistant,
}

impl ResponsesInputRole {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Developer => "developer",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResponsesInputItem {
    Message {
        role: ResponsesInputRole,
        content: String,
        refusal: Option<String>,
    },
    FunctionCall {
        call_id: String,
        name: String,
        arguments: String,
    },
    FunctionCallOutput {
        call_id: String,
        output: String,
    },
    ProviderPrivateAssistant {
        schema: String,
        message: KimiAssistantMessage,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct FunctionTool {
    name: String,
    description: String,
    parameters: Value,
}

impl FunctionTool {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: Value,
    ) -> Result<Self, ConnectorError> {
        let name = name.into();
        validate_wire_id("function tool name", &name)?;
        let description = description.into();
        if description.is_empty()
            || description.len() > MAX_TOOL_DESCRIPTION_BYTES
            || description.chars().any(char::is_control)
        {
            return Err(ConnectorError::new(
                ConnectorFailureKind::Configuration,
                format!(
                    "function tool description must contain 1 to {MAX_TOOL_DESCRIPTION_BYTES} bytes without control characters"
                ),
            ));
        }
        if !parameters.is_object() {
            return Err(ConnectorError::new(
                ConnectorFailureKind::Configuration,
                "function tool parameters must be a JSON object schema",
            ));
        }
        Ok(Self {
            name,
            description,
            parameters,
        })
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn description(&self) -> &str {
        &self.description
    }

    pub(super) const fn parameters(&self) -> &Value {
        &self.parameters
    }
}

/// Request-local exposure of the current frozen tool registry.
///
/// Historical function-call input items are independent replay data and remain
/// valid when current exposure is disabled.
#[derive(Clone, Debug, PartialEq)]
pub enum RequestToolExposure {
    Enabled(Vec<FunctionTool>),
    Disabled,
}

impl RequestToolExposure {
    #[must_use]
    pub fn enabled(tools: Vec<FunctionTool>) -> Self {
        Self::Enabled(tools)
    }

    #[must_use]
    pub const fn disabled() -> Self {
        Self::Disabled
    }

    fn tools(&self) -> Option<&[FunctionTool]> {
        match self {
            Self::Enabled(tools) => Some(tools),
            Self::Disabled => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReasoningEffort {
    None,
    Low,
    Minimal,
    Medium,
    High,
    Max,
}

impl ReasoningEffort {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Low => "low",
            Self::Minimal => "minimal",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
        }
    }
}

/// Provider-neutral request context used only by reviewed Connector wire variants.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct ModelCacheAffinityHint(String);

impl ModelCacheAffinityHint {
    pub(crate) fn for_session(session_id: SessionId) -> Self {
        Self(session_id.to_string())
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ModelCacheAffinityHint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ModelCacheAffinityHint([redacted])")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResponsesRequest {
    input: Vec<ResponsesInputItem>,
    tool_exposure: RequestToolExposure,
    max_output_tokens: Option<u64>,
    reasoning_effort: Option<ReasoningEffort>,
    replay_budget: Option<crate::ModelReplayBudget>,
    cache_affinity_hint: Option<ModelCacheAffinityHint>,
}

impl ResponsesRequest {
    pub fn new(
        input: Vec<ResponsesInputItem>,
        tool_exposure: RequestToolExposure,
        max_output_tokens: impl Into<Option<u64>>,
        reasoning_effort: Option<ReasoningEffort>,
    ) -> Result<Self, ConnectorError> {
        let max_output_tokens = max_output_tokens.into();
        if input.is_empty() {
            return Err(ConnectorError::new(
                ConnectorFailureKind::Configuration,
                "Responses input must contain at least one item",
            ));
        }
        if max_output_tokens == Some(0) {
            return Err(ConnectorError::new(
                ConnectorFailureKind::Configuration,
                "Responses max_output_tokens must be positive",
            ));
        }
        if let Some(tools) = tool_exposure.tools() {
            if tools.is_empty() {
                return Err(ConnectorError::new(
                    ConnectorFailureKind::Configuration,
                    "enabled tool exposure requires at least one function tool",
                ));
            }
            let mut names = HashSet::new();
            for tool in tools {
                if !names.insert(tool.name()) {
                    return Err(ConnectorError::new(
                        ConnectorFailureKind::Configuration,
                        format!("duplicate function tool name {:?}", tool.name()),
                    ));
                }
            }
        }
        for item in &input {
            match item {
                ResponsesInputItem::Message { role, refusal, .. } => {
                    if refusal.is_some() && *role != ResponsesInputRole::Assistant {
                        return Err(ConnectorError::new(
                            ConnectorFailureKind::Configuration,
                            "visible refusal is valid only on an assistant message",
                        ));
                    }
                },
                ResponsesInputItem::FunctionCall { call_id, name, .. } => {
                    validate_wire_id("function call_id", call_id)?;
                    validate_wire_id("function name", name)?;
                },
                ResponsesInputItem::FunctionCallOutput { call_id, .. } => {
                    validate_wire_id("function call_id", call_id)?;
                },
                ResponsesInputItem::ProviderPrivateAssistant { schema, .. }
                    if schema != "kimi.assistant-message/v1alpha1" =>
                {
                    return Err(ConnectorError::new(
                        ConnectorFailureKind::Configuration,
                        "provider-private assistant replay schema is unsupported",
                    ));
                },
                ResponsesInputItem::ProviderPrivateAssistant { .. } => {},
            }
        }
        Ok(Self {
            input,
            tool_exposure,
            max_output_tokens,
            reasoning_effort,
            replay_budget: None,
            cache_affinity_hint: None,
        })
    }

    pub(crate) fn with_replay_budget(mut self, replay_budget: crate::ModelReplayBudget) -> Self {
        self.replay_budget = Some(replay_budget);
        self
    }

    pub(crate) fn with_cache_affinity_hint(mut self, hint: ModelCacheAffinityHint) -> Self {
        self.cache_affinity_hint = Some(hint);
        self
    }

    pub(crate) fn input(&self) -> &[ResponsesInputItem] {
        &self.input
    }

    pub(super) fn contains_provider_private_input(&self) -> bool {
        self.input
            .iter()
            .any(|item| matches!(item, ResponsesInputItem::ProviderPrivateAssistant { .. }))
    }

    pub(super) fn tools(&self) -> Option<&[FunctionTool]> {
        self.tool_exposure.tools()
    }

    pub(super) const fn max_output_tokens(&self) -> Option<u64> {
        self.max_output_tokens
    }

    pub(super) const fn reasoning_effort(&self) -> Option<ReasoningEffort> {
        self.reasoning_effort
    }

    pub(super) const fn replay_budget(&self) -> Option<crate::ModelReplayBudget> {
        self.replay_budget
    }

    pub(crate) fn cache_affinity_hint(&self) -> Option<&str> {
        self.cache_affinity_hint
            .as_ref()
            .map(ModelCacheAffinityHint::as_str)
    }

    pub(crate) fn tokenization_payload(&self, model: &str) -> Value {
        self.wire_body(model)
    }

    pub(super) fn wire_body(&self, model: &str) -> Value {
        let input = self
            .input
            .iter()
            .map(|item| match item {
                ResponsesInputItem::Message {
                    role,
                    content,
                    refusal,
                } => {
                    let mut visible = content.clone();
                    if let Some(refusal) = refusal {
                        visible.push_str(refusal);
                    }
                    json!({
                        "role": role.as_str(),
                        "content": visible,
                    })
                },
                ResponsesInputItem::FunctionCall {
                    call_id,
                    name,
                    arguments,
                } => json!({
                    "type": "function_call",
                    "call_id": call_id,
                    "name": name,
                    "arguments": arguments,
                }),
                ResponsesInputItem::FunctionCallOutput { call_id, output } => json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": output,
                }),
                ResponsesInputItem::ProviderPrivateAssistant { schema, .. } => json!({
                    "type": "provider_private_assistant",
                    "schema": schema,
                }),
            })
            .collect::<Vec<_>>();
        let mut body = json!({
            "model": model,
            "input": input,
            "stream": true,
        });
        if let Some(max_output_tokens) = self.max_output_tokens {
            body["max_output_tokens"] = Value::from(max_output_tokens);
        }
        if let Some(tools) = self.tools() {
            body["tools"] = Value::Array(
                tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "type": "function",
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters,
                        })
                    })
                    .collect(),
            );
            body["tool_choice"] = Value::String("auto".to_owned());
        }
        if let Some(effort) = self.reasoning_effort {
            body["reasoning"] = json!({ "effort": effort.as_str() });
        }
        body
    }
}

fn validate_wire_id(label: &'static str, value: &str) -> Result<(), ConnectorError> {
    if value.is_empty() || value.len() > MAX_WIRE_ID_BYTES || value.chars().any(char::is_control) {
        return Err(ConnectorError::new(
            ConnectorFailureKind::Configuration,
            format!(
                "{label} must contain 1 to {MAX_WIRE_ID_BYTES} bytes without control characters"
            ),
        ));
    }
    Ok(())
}
