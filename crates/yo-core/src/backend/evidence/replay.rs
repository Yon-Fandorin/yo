use std::fmt;

use serde_json::{Value, json};

use super::{valid_schema, valid_value};

const MAX_REPLAY_ITEMS: usize = 4_096;
const MAX_REPLAY_TEXT_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MAX_REPLAY_CONTRACT_BYTES: usize = 1024 * 1024;
pub(super) const MAX_REPLAY_DELTA_BYTES: usize = 16 * 1024 * 1024;
const MAX_REPLAY_PREFIX_BYTES: usize = 64 * 1024 * 1024;
const KIMI_ASSISTANT_SCHEMA: &str = "kimi.assistant-message/v1alpha1";
const MAX_KIMI_TOOL_CALLS: usize = 1_024;
const MAX_KIMI_CALL_ID_BYTES: usize = 4_096;
const MAX_KIMI_FUNCTION_NAME_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ModelReplayBudget {
    encoded_prefix_bytes: usize,
    prefix_items: usize,
}

impl ModelReplayBudget {
    pub(crate) fn accepts_item_lengths(&self, item_lengths: &[usize]) -> bool {
        self.encoded_len_with_item_lengths(item_lengths)
            .is_some_and(|bytes| bytes <= MAX_REPLAY_DELTA_BYTES)
    }

    pub(crate) fn encoded_len_with_item_lengths(&self, item_lengths: &[usize]) -> Option<usize> {
        let total_items = self.prefix_items.checked_add(item_lengths.len())?;
        if total_items > MAX_REPLAY_ITEMS {
            return None;
        }
        item_lengths
            .iter()
            .try_fold(
                (self.encoded_prefix_bytes, self.prefix_items),
                |(bytes, items), item_bytes| {
                    bytes
                        .checked_add(usize::from(items != 0))?
                        .checked_add(*item_bytes)
                        .map(|bytes| (bytes, items + 1))
                },
            )
            .map(|(bytes, _)| bytes)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct KimiReplayToolCallSize {
    pub(crate) id_json_bytes: usize,
    pub(crate) name_json_bytes: usize,
    pub(crate) arguments_json_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelReplayRole {
    System,
    Developer,
    User,
    Assistant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelReplayTool {
    name: String,
    description: String,
    schema_version: String,
    parameters: Value,
}

impl ModelReplayTool {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        schema_version: impl Into<String>,
        parameters: Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            schema_version: schema_version.into(),
            parameters,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    pub const fn parameters(&self) -> &Value {
        &self.parameters
    }

    pub(crate) fn is_valid(&self) -> bool {
        valid_schema(&self.name)
            && valid_value(&self.description)
            && valid_schema(&self.schema_version)
            && self.parameters.is_object()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelReplayContract {
    system_prompt: String,
    tools: Vec<ModelReplayTool>,
}

impl ModelReplayContract {
    pub fn new(system_prompt: impl Into<String>, tools: Vec<ModelReplayTool>) -> Self {
        Self {
            system_prompt: system_prompt.into(),
            tools,
        }
    }

    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    pub fn tools(&self) -> &[ModelReplayTool] {
        &self.tools
    }

    pub(crate) fn is_valid(&self) -> bool {
        let mut names = std::collections::HashSet::new();
        !self.system_prompt.is_empty()
            && self.tools.len() <= 1_024
            && self.tools.iter().all(ModelReplayTool::is_valid)
            && self.tools.iter().all(|tool| names.insert(tool.name()))
            && encoded_contract_len(self) <= MAX_REPLAY_CONTRACT_BYTES
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelReplayItem {
    Message {
        role: ModelReplayRole,
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

#[derive(Clone, Eq, PartialEq)]
pub struct KimiAssistantToolCall {
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
    pub fn new(
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

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn arguments(&self) -> &str {
        &self.arguments
    }

    fn is_valid(&self) -> bool {
        !self.id.is_empty()
            && self.id.len() <= MAX_KIMI_CALL_ID_BYTES
            && valid_kimi_function_name(&self.name)
            && self.arguments.len() <= 4 * 1024 * 1024
            && serde_json::from_str::<Value>(&self.arguments).is_ok()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct KimiAssistantMessage {
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
    pub fn new(
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

    pub fn reasoning_content(&self) -> &str {
        &self.reasoning_content
    }

    pub fn content(&self) -> Option<&str> {
        self.content.as_deref()
    }

    pub fn tool_calls(&self) -> &[KimiAssistantToolCall] {
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
            && (!self.tool_calls.is_empty() || self.content.is_some())
            && {
                let mut ids = std::collections::HashSet::new();
                self.tool_calls.iter().all(|call| ids.insert(call.id()))
            }
    }
}

impl ModelReplayItem {
    fn is_valid(&self) -> bool {
        match self {
            Self::Message {
                role,
                content,
                refusal,
            } => {
                content.len() <= MAX_REPLAY_TEXT_BYTES
                    && refusal.as_ref().is_none_or(|refusal| {
                        *role == ModelReplayRole::Assistant
                            && refusal.len() <= MAX_REPLAY_TEXT_BYTES
                    })
            },
            Self::FunctionCall {
                call_id,
                name,
                arguments,
            } => {
                valid_value(call_id)
                    && valid_schema(name)
                    && arguments.len() <= MAX_REPLAY_TEXT_BYTES
            },
            Self::FunctionCallOutput { call_id, output } => {
                valid_value(call_id) && output.len() <= MAX_REPLAY_TEXT_BYTES
            },
            Self::ProviderPrivateAssistant { schema, message } => {
                schema == KIMI_ASSISTANT_SCHEMA && message.is_valid()
            },
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModelReplayDelta {
    contract: Option<ModelReplayContract>,
    items: Vec<ModelReplayItem>,
}

impl ModelReplayDelta {
    pub(crate) const MAX_ENCODED_BYTES: usize = MAX_REPLAY_DELTA_BYTES;

    pub fn new(contract: Option<ModelReplayContract>, items: Vec<ModelReplayItem>) -> Self {
        Self { contract, items }
    }

    pub const fn contract(&self) -> Option<&ModelReplayContract> {
        self.contract.as_ref()
    }

    pub fn items(&self) -> &[ModelReplayItem] {
        &self.items
    }

    pub(crate) fn is_valid(&self) -> bool {
        self.contract
            .as_ref()
            .is_none_or(ModelReplayContract::is_valid)
            && !self.items.is_empty()
            && self.fits_capacity()
            && self.items.iter().all(ModelReplayItem::is_valid)
    }

    pub(crate) fn fits_capacity(&self) -> bool {
        Self::prospective_encoded_len(self.contract.as_ref(), self.items.iter())
            .is_some_and(|bytes| bytes <= MAX_REPLAY_DELTA_BYTES)
    }

    pub(crate) fn prospective_encoded_len<'a>(
        contract: Option<&ModelReplayContract>,
        items: impl Iterator<Item = &'a ModelReplayItem>,
    ) -> Option<usize> {
        let items = items.collect::<Vec<_>>();
        (items.len() <= MAX_REPLAY_ITEMS).then(|| encoded_prefix_len(contract, items.into_iter()))
    }

    pub(crate) fn replay_budget<'a>(
        contract: Option<&ModelReplayContract>,
        items: impl Iterator<Item = &'a ModelReplayItem>,
    ) -> Option<ModelReplayBudget> {
        let items = items.collect::<Vec<_>>();
        let encoded_prefix_bytes = (items.len() <= MAX_REPLAY_ITEMS)
            .then(|| encoded_prefix_len(contract, items.iter().copied()))?;
        (encoded_prefix_bytes <= MAX_REPLAY_DELTA_BYTES).then_some(ModelReplayBudget {
            encoded_prefix_bytes,
            prefix_items: items.len(),
        })
    }

    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        validate_replay_delta(self).map(|_| ())
    }
}

pub(crate) fn kimi_replay_round_item_lengths(
    private_replay: bool,
    content_seen: bool,
    content_json_bytes: usize,
    reasoning_json_bytes: usize,
    calls: &[KimiReplayToolCallSize],
) -> Option<Vec<usize>> {
    let message = encoded_item_len(&ModelReplayItem::Message {
        role: ModelReplayRole::Assistant,
        content: String::new(),
        refusal: None,
    })
    .checked_add(content_json_bytes)?;
    let empty_call = encoded_item_len(&ModelReplayItem::FunctionCall {
        call_id: String::new(),
        name: String::new(),
        arguments: String::new(),
    });
    let mut lengths = Vec::with_capacity(calls.len().checked_add(2)?);
    lengths.push(message);
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

    let empty_private_call = KimiAssistantToolCall::new("", "", "");
    let empty_private_call_bytes = encoded_kimi_private_call_len(&empty_private_call);
    let private_message = if calls.is_empty() {
        KimiAssistantMessage::new("", Some(String::new()), Vec::new())
    } else {
        KimiAssistantMessage::new("", None, vec![empty_private_call])
    };
    let mut private = encoded_item_len(&ModelReplayItem::ProviderPrivateAssistant {
        schema: KIMI_ASSISTANT_SCHEMA.to_owned(),
        message: private_message,
    })
    .checked_add(reasoning_json_bytes)?;
    if calls.is_empty() {
        private = private.checked_add(content_json_bytes)?;
    } else {
        if content_seen {
            private = private.checked_sub(2)?.checked_add(content_json_bytes)?;
        }
        for (index, call) in calls.iter().enumerate() {
            if index != 0 {
                private = private
                    .checked_add(1)?
                    .checked_add(empty_private_call_bytes)?;
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModelReplay {
    contract: Option<ModelReplayContract>,
    items: Vec<ModelReplayItem>,
    encoded_prefix_bytes: usize,
    known_calls: std::collections::BTreeSet<String>,
    answered_calls: std::collections::BTreeSet<String>,
}

impl ModelReplay {
    pub const fn contract(&self) -> Option<&ModelReplayContract> {
        self.contract.as_ref()
    }

    pub fn items(&self) -> &[ModelReplayItem] {
        &self.items
    }

    pub(crate) fn apply(&mut self, delta: &ModelReplayDelta) -> Result<(), &'static str> {
        let (delta_calls, delta_answers) = validate_replay_delta(delta)?;
        match (self.contract.is_some(), delta.contract.is_some()) {
            (false, false) => return Err("first model replay delta requires its contract"),
            (true, true) => return Err("model replay contract was declared more than once"),
            (false, true) | (true, false) => {},
        }
        if self.items.len().saturating_add(delta.items.len()) > MAX_REPLAY_ITEMS {
            return Err("model replay item limit exceeded");
        }
        if delta_calls
            .iter()
            .any(|call_id| self.known_calls.contains(call_id))
        {
            return Err("model replay contains a duplicate function call identity");
        }
        if delta_answers
            .iter()
            .any(|call_id| self.answered_calls.contains(call_id))
        {
            return Err("model replay contains a duplicate function call output");
        }
        let encoded_prefix_bytes = if self.items.is_empty() {
            encoded_prefix_len(delta.contract.as_ref(), delta.items.iter())
        } else {
            self.encoded_prefix_bytes
                .saturating_add(encoded_items_len(&delta.items))
                .saturating_add(delta.items.len())
        };
        if encoded_prefix_bytes > MAX_REPLAY_PREFIX_BYTES {
            return Err("model replay prefix byte limit exceeded");
        }
        if let Some(contract) = &delta.contract {
            self.contract = Some(contract.clone());
        }
        self.items.extend(delta.items.iter().cloned());
        self.encoded_prefix_bytes = encoded_prefix_bytes;
        self.known_calls.extend(delta_calls);
        self.answered_calls.extend(delta_answers);
        Ok(())
    }
}

fn validate_replay_delta(
    delta: &ModelReplayDelta,
) -> Result<
    (
        std::collections::BTreeSet<String>,
        std::collections::BTreeSet<String>,
    ),
    &'static str,
> {
    if !delta.is_valid() {
        return Err("model replay delta is invalid or exceeds its bounds");
    }
    let mut known_calls = std::collections::BTreeSet::new();
    let mut answered_calls = std::collections::BTreeSet::new();
    for (index, item) in delta.items.iter().enumerate() {
        match item {
            ModelReplayItem::FunctionCall {
                call_id, arguments, ..
            } => {
                if serde_json::from_str::<Value>(arguments).is_err() {
                    return Err("model replay function arguments are not valid JSON");
                }
                if !known_calls.insert(call_id.clone()) {
                    return Err("model replay contains a duplicate function call identity");
                }
            },
            ModelReplayItem::FunctionCallOutput { call_id, .. } => {
                if !known_calls.contains(call_id) {
                    return Err("model replay output has no matching function call");
                }
                if !answered_calls.insert(call_id.clone()) {
                    return Err("model replay contains a duplicate function call output");
                }
            },
            ModelReplayItem::Message { .. } => {},
            ModelReplayItem::ProviderPrivateAssistant { message, .. } => {
                validate_kimi_private_projection(&delta.items, index, message)?;
            },
        }
    }
    if known_calls.difference(&answered_calls).next().is_some() {
        return Err("model replay delta ends with an unmatched function call");
    }
    Ok((known_calls, answered_calls))
}

fn validate_kimi_private_projection(
    items: &[ModelReplayItem],
    private_index: usize,
    private: &KimiAssistantMessage,
) -> Result<(), &'static str> {
    let mut group_start = private_index;
    while group_start > 0 && matches!(items[group_start - 1], ModelReplayItem::FunctionCall { .. })
    {
        group_start -= 1;
    }
    let Some(ModelReplayItem::Message {
        role: ModelReplayRole::Assistant,
        content,
        refusal: None,
    }) = group_start
        .checked_sub(1)
        .and_then(|index| items.get(index))
    else {
        return Err("Kimi private assistant has no immediately preceding assistant projection");
    };
    if private.content().unwrap_or_default() != content
        || (private.content().is_none() && !content.is_empty())
    {
        return Err("Kimi private assistant content does not match its visible projection");
    }
    let generic_calls = &items[group_start..private_index];
    if generic_calls.len() != private.tool_calls().len() {
        return Err("Kimi private assistant tool calls do not match its visible projection");
    }
    for (generic, private) in generic_calls.iter().zip(private.tool_calls()) {
        let ModelReplayItem::FunctionCall {
            call_id,
            name,
            arguments,
        } = generic
        else {
            return Err("Kimi private assistant projection is malformed");
        };
        if call_id != private.id() || name != private.name() || arguments != private.arguments() {
            return Err("Kimi private assistant tool call differs from its visible projection");
        }
    }
    Ok(())
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

fn encoded_contract_len(contract: &ModelReplayContract) -> usize {
    serde_json::to_vec(&contract_value(contract))
        .expect("a replay contract is always JSON serializable")
        .len()
}

fn encoded_prefix_len<'a>(
    contract: Option<&ModelReplayContract>,
    items: impl Iterator<Item = &'a ModelReplayItem>,
) -> usize {
    let value = json!({
        "contract": contract.map(contract_value),
        "items": items.map(item_value).collect::<Vec<_>>(),
    });
    serde_json::to_vec(&value)
        .expect("a replay prefix is always JSON serializable")
        .len()
}

fn encoded_items_len(items: &[ModelReplayItem]) -> usize {
    items.iter().map(encoded_item_len).sum()
}

fn encoded_item_len(item: &ModelReplayItem) -> usize {
    serde_json::to_vec(&item_value(item))
        .expect("a replay item is always JSON serializable")
        .len()
}

fn encoded_kimi_private_call_len(call: &KimiAssistantToolCall) -> usize {
    serde_json::to_vec(&json!({
        "id": call.id(),
        "type": "function",
        "function": {
            "name": call.name(),
            "arguments": call.arguments(),
        },
    }))
    .expect("a Kimi private call is always JSON serializable")
    .len()
}

fn contract_value(contract: &ModelReplayContract) -> Value {
    json!({
        "system_prompt": contract.system_prompt,
        "tools": contract.tools.iter().map(|tool| json!({
            "name": tool.name,
            "description": tool.description,
            "schema_version": tool.schema_version,
            "parameters": tool.parameters,
        })).collect::<Vec<_>>(),
    })
}

fn item_value(item: &ModelReplayItem) -> Value {
    match item {
        ModelReplayItem::Message {
            role,
            content,
            refusal,
        } => {
            let mut value = json!({
                "kind": "message",
                "role": match role {
                    ModelReplayRole::System => "system",
                    ModelReplayRole::Developer => "developer",
                    ModelReplayRole::User => "user",
                    ModelReplayRole::Assistant => "assistant",
                },
                "content": content,
            });
            if let Some(refusal) = refusal {
                value["refusal"] = Value::String(refusal.clone());
            }
            value
        },
        ModelReplayItem::FunctionCall {
            call_id,
            name,
            arguments,
        } => json!({
            "kind": "function_call",
            "call_id": call_id,
            "name": name,
            "arguments": arguments,
        }),
        ModelReplayItem::FunctionCallOutput { call_id, output } => json!({
            "kind": "function_call_output",
            "call_id": call_id,
            "output": output,
        }),
        ModelReplayItem::ProviderPrivateAssistant { schema, message } => {
            let mut private = json!({
                "role": "assistant",
                "reasoning_content": message.reasoning_content(),
                "content": message.content(),
            });
            if !message.tool_calls().is_empty() {
                private["tool_calls"] = Value::Array(
                    message
                        .tool_calls()
                        .iter()
                        .map(|call| {
                            json!({
                                "id": call.id(),
                                "type": "function",
                                "function": {
                                    "name": call.name(),
                                    "arguments": call.arguments(),
                                },
                            })
                        })
                        .collect(),
                );
            }
            json!({
                "kind": "provider_private_assistant",
                "schema": schema,
                "message": private,
            })
        },
    }
}
