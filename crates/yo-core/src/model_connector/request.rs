use std::collections::HashSet;

use serde_json::{Value, json};

use super::{ConnectorError, ConnectorFailureKind};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReasoningEffort {
    None,
    Minimal,
    Medium,
    High,
}

impl ReasoningEffort {
    const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResponsesRequest {
    input: Vec<ResponsesInputItem>,
    tools: Vec<FunctionTool>,
    max_output_tokens: u64,
    reasoning_effort: Option<ReasoningEffort>,
}

impl ResponsesRequest {
    pub fn new(
        input: Vec<ResponsesInputItem>,
        tools: Vec<FunctionTool>,
        max_output_tokens: u64,
        reasoning_effort: Option<ReasoningEffort>,
    ) -> Result<Self, ConnectorError> {
        if input.is_empty() {
            return Err(ConnectorError::new(
                ConnectorFailureKind::Configuration,
                "Responses input must contain at least one item",
            ));
        }
        if max_output_tokens == 0 {
            return Err(ConnectorError::new(
                ConnectorFailureKind::Configuration,
                "Responses max_output_tokens must be positive",
            ));
        }
        let mut names = HashSet::new();
        for tool in &tools {
            if !names.insert(tool.name()) {
                return Err(ConnectorError::new(
                    ConnectorFailureKind::Configuration,
                    format!("duplicate function tool name {:?}", tool.name()),
                ));
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
            }
        }
        Ok(Self {
            input,
            tools,
            max_output_tokens,
            reasoning_effort,
        })
    }

    pub(crate) fn input(&self) -> &[ResponsesInputItem] {
        &self.input
    }

    pub(super) fn tools(&self) -> &[FunctionTool] {
        &self.tools
    }

    pub(super) const fn max_output_tokens(&self) -> u64 {
        self.max_output_tokens
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
            })
            .collect::<Vec<_>>();
        let tools = self
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                })
            })
            .collect::<Vec<_>>();
        let mut body = json!({
            "model": model,
            "input": input,
            "tools": tools,
            "tool_choice": "auto",
            "stream": true,
            "max_output_tokens": self.max_output_tokens,
        });
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
