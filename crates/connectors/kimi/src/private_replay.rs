use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use yo_connector_transport::configuration_failure;
use yo_core::{ConnectorError, ModelReplayItem, ModelReplayRole, ProviderPrivateReplayEnvelope};

pub(crate) const KIMI_ASSISTANT_SCHEMA: &str = "kimi.assistant-message/v1alpha1";
const MAX_REPLAY_TEXT_BYTES: usize = 16 * 1024 * 1024;
const MAX_KIMI_TOOL_CALLS: usize = 1_024;
const MAX_KIMI_CALL_ID_BYTES: usize = 4_096;
const MAX_KIMI_FUNCTION_NAME_BYTES: usize = 64;
const MAX_KIMI_FUNCTION_ARGUMENT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct KimiReplayToolCallSize {
    pub id_json_bytes: usize,
    pub name_json_bytes: usize,
    pub arguments_json_bytes: usize,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct KimiAssistantToolCall {
    id: String,
    name: String,
    arguments: String,
}

impl fmt::Debug for KimiAssistantToolCall {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KimiAssistantToolCall")
            .field("id_bytes", &self.id.len())
            .field("name_bytes", &self.name.len())
            .field("argument_bytes", &self.arguments.len())
            .finish()
    }
}

impl KimiAssistantToolCall {
    pub(crate) fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments: arguments.into(),
        }
    }

    pub(crate) fn id(&self) -> &str {
        &self.id
    }
    pub(crate) fn name(&self) -> &str {
        &self.name
    }
    pub(crate) fn arguments(&self) -> &str {
        &self.arguments
    }

    fn is_valid(&self) -> bool {
        !self.id.is_empty()
            && self.id.len() <= MAX_KIMI_CALL_ID_BYTES
            && valid_kimi_function_name(&self.name)
            && self.arguments.len() <= MAX_KIMI_FUNCTION_ARGUMENT_BYTES
            && serde_json::from_str::<Value>(&self.arguments).is_ok()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct KimiAssistantMessage {
    reasoning_content: String,
    content: Option<String>,
    tool_calls: Vec<KimiAssistantToolCall>,
}

impl fmt::Debug for KimiAssistantMessage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KimiAssistantMessage")
            .field("reasoning_bytes", &self.reasoning_content.len())
            .field("content_bytes", &self.content.as_ref().map(String::len))
            .field("tool_call_count", &self.tool_calls.len())
            .finish()
    }
}

impl KimiAssistantMessage {
    pub(crate) fn new(
        reasoning_content: impl Into<String>,
        content: Option<String>,
        tool_calls: Vec<KimiAssistantToolCall>,
    ) -> Self {
        Self {
            reasoning_content: reasoning_content.into(),
            content,
            tool_calls,
        }
    }

    pub(crate) fn reasoning_content(&self) -> &str {
        &self.reasoning_content
    }
    pub(crate) fn content(&self) -> Option<&str> {
        self.content.as_deref()
    }
    pub(crate) fn tool_calls(&self) -> &[KimiAssistantToolCall] {
        &self.tool_calls
    }

    pub(crate) fn is_valid(&self) -> bool {
        self.reasoning_content.len() <= MAX_REPLAY_TEXT_BYTES
            && self
                .content
                .as_ref()
                .is_none_or(|content| content.len() <= MAX_REPLAY_TEXT_BYTES)
            && self.tool_calls.len() <= MAX_KIMI_TOOL_CALLS
            && self.tool_calls.iter().all(KimiAssistantToolCall::is_valid)
            && self
                .tool_calls
                .iter()
                .try_fold(0usize, |total, call| {
                    total.checked_add(call.arguments().len())
                })
                .is_some_and(|total| total <= MAX_KIMI_FUNCTION_ARGUMENT_BYTES)
            && (!self.tool_calls.is_empty() || self.content.is_some())
            && {
                let mut ids = std::collections::HashSet::new();
                self.tool_calls.iter().all(|call| ids.insert(call.id()))
            }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireKimiAssistantMessage {
    role: String,
    reasoning_content: String,
    content: RequiredNullableString,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_tool_calls",
        skip_serializing_if = "Option::is_none"
    )]
    tool_calls: Option<Vec<WireKimiAssistantToolCall>>,
}

#[derive(Deserialize, Serialize)]
#[serde(transparent)]
struct RequiredNullableString(Option<String>);

fn deserialize_optional_tool_calls<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<WireKimiAssistantToolCall>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Vec::<WireKimiAssistantToolCall>::deserialize(deserializer).map(Some)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireKimiAssistantToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: WireKimiAssistantFunction,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireKimiAssistantFunction {
    name: String,
    arguments: String,
}

pub(crate) fn encode_envelope(
    message: &KimiAssistantMessage,
) -> Result<ProviderPrivateReplayEnvelope, ConnectorError> {
    if !message.is_valid() {
        return Err(configuration_failure("Kimi private assistant is invalid"));
    }
    let wire = wire_message(message);
    let payload = serde_json::to_vec(&wire)
        .map_err(|_| configuration_failure("Kimi private assistant cannot be encoded"))?;
    ProviderPrivateReplayEnvelope::new(KIMI_ASSISTANT_SCHEMA, payload)
        .map_err(configuration_failure)
}

pub(crate) fn decode_envelope(
    envelope: &ProviderPrivateReplayEnvelope,
) -> Result<KimiAssistantMessage, ConnectorError> {
    if envelope.schema() != KIMI_ASSISTANT_SCHEMA {
        return Err(configuration_failure(
            "Kimi private assistant schema is unsupported",
        ));
    }
    let wire: WireKimiAssistantMessage = serde_json::from_slice(envelope.payload())
        .map_err(|_| configuration_failure("Kimi private assistant payload is malformed"))?;
    if wire.role != "assistant" || wire.tool_calls.as_ref().is_some_and(Vec::is_empty) {
        return Err(configuration_failure(
            "Kimi private assistant has an invalid closed shape",
        ));
    }
    let calls = wire
        .tool_calls
        .unwrap_or_default()
        .into_iter()
        .map(|call| {
            if call.kind != "function" {
                return Err(configuration_failure(
                    "Kimi private assistant tool type is unsupported",
                ));
            }
            Ok(KimiAssistantToolCall::new(
                call.id,
                call.function.name,
                call.function.arguments,
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let message = KimiAssistantMessage::new(wire.reasoning_content, wire.content.0, calls);
    if !message.is_valid() {
        return Err(configuration_failure(
            "Kimi private assistant payload is invalid",
        ));
    }
    Ok(message)
}

pub(crate) fn visible_projection(message: &KimiAssistantMessage) -> Vec<ModelReplayItem> {
    let mut projection = Vec::with_capacity(message.tool_calls().len() + 1);
    projection.push(ModelReplayItem::Message {
        role: ModelReplayRole::Assistant,
        content: message.content().unwrap_or_default().to_owned(),
        refusal: None,
    });
    projection.extend(
        message
            .tool_calls()
            .iter()
            .map(|call| ModelReplayItem::FunctionCall {
                call_id: call.id().to_owned(),
                name: call.name().to_owned(),
                arguments: call.arguments().to_owned(),
            }),
    );
    projection
}

fn wire_message(message: &KimiAssistantMessage) -> WireKimiAssistantMessage {
    WireKimiAssistantMessage {
        role: "assistant".to_owned(),
        reasoning_content: message.reasoning_content().to_owned(),
        content: RequiredNullableString(message.content().map(str::to_owned)),
        tool_calls: (!message.tool_calls().is_empty()).then(|| {
            message
                .tool_calls()
                .iter()
                .map(|call| WireKimiAssistantToolCall {
                    id: call.id().to_owned(),
                    kind: "function".to_owned(),
                    function: WireKimiAssistantFunction {
                        name: call.name().to_owned(),
                        arguments: call.arguments().to_owned(),
                    },
                })
                .collect()
        }),
    }
}

pub(crate) fn private_message_value(message: &KimiAssistantMessage) -> Value {
    serde_json::to_value(wire_message(message))
        .expect("a validated Kimi private assistant is JSON serializable")
}

pub(crate) fn kimi_replay_round_item_lengths(
    private_replay: bool,
    content_seen: bool,
    content_json_bytes: usize,
    reasoning_json_bytes: usize,
    calls: &[KimiReplayToolCallSize],
) -> Option<Vec<usize>> {
    let empty_message = serde_json::to_vec(&json!({
        "kind": "message", "role": "assistant", "content": "",
    }))
    .ok()?
    .len();
    let mut lengths = Vec::with_capacity(calls.len().checked_add(2)?);
    lengths.push(empty_message.checked_add(content_json_bytes)?);
    let empty_call = serde_json::to_vec(&json!({
        "kind": "function_call", "call_id": "", "name": "", "arguments": "",
    }))
    .ok()?
    .len();
    for call in calls {
        lengths.push(
            empty_call
                .checked_add(call.id_json_bytes)?
                .checked_add(call.name_json_bytes)?
                .checked_add(call.arguments_json_bytes)?,
        );
    }
    if !private_replay {
        return Some(lengths);
    }

    let empty_private_call = serde_json::to_vec(&json!({
        "id": "", "type": "function", "function": {"name": "", "arguments": ""},
    }))
    .ok()?
    .len();
    let empty_private = if calls.is_empty() {
        json!({"role":"assistant", "reasoning_content":"", "content":""})
    } else {
        json!({"role":"assistant", "reasoning_content":"", "content":null,
            "tool_calls":[{"id":"", "type":"function", "function":{"name":"", "arguments":""}}]})
    };
    let mut private = serde_json::to_vec(&json!({
        "kind": "provider_private_assistant", "schema": KIMI_ASSISTANT_SCHEMA,
        "message": empty_private,
    }))
    .ok()?
    .len()
    .checked_add(reasoning_json_bytes)?;
    if calls.is_empty() {
        private = private.checked_add(content_json_bytes)?;
    } else {
        if content_seen {
            private = private.checked_sub(2)?.checked_add(content_json_bytes)?;
        }
        for (index, call) in calls.iter().enumerate() {
            if index != 0 {
                private = private.checked_add(1)?.checked_add(empty_private_call)?;
            }
            private = private
                .checked_add(call.id_json_bytes)?
                .checked_add(call.name_json_bytes)?
                .checked_add(call.arguments_json_bytes)?;
        }
    }
    lengths.push(private);
    Some(lengths)
}

fn valid_kimi_function_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    (3..=MAX_KIMI_FUNCTION_NAME_BYTES).contains(&bytes.len())
        && bytes
            .first()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}
