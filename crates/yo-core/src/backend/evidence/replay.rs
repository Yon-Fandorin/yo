use serde_json::{Value, json};

use super::{valid_schema, valid_value};

const MAX_REPLAY_ITEMS: usize = 4_096;
const MAX_REPLAY_TEXT_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MAX_REPLAY_CONTRACT_BYTES: usize = 1024 * 1024;
pub(super) const MAX_REPLAY_DELTA_BYTES: usize = 16 * 1024 * 1024;
const MAX_REPLAY_PREFIX_BYTES: usize = 64 * 1024 * 1024;

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
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModelReplayDelta {
    contract: Option<ModelReplayContract>,
    items: Vec<ModelReplayItem>,
}

impl ModelReplayDelta {
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
            && self.items.len() <= MAX_REPLAY_ITEMS
            && self.items.iter().all(ModelReplayItem::is_valid)
            && encoded_delta_len(self) <= MAX_REPLAY_DELTA_BYTES
    }

    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        validate_replay_delta(self).map(|_| ())
    }
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
    for item in &delta.items {
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
        }
    }
    if known_calls.difference(&answered_calls).next().is_some() {
        return Err("model replay delta ends with an unmatched function call");
    }
    Ok((known_calls, answered_calls))
}

fn encoded_contract_len(contract: &ModelReplayContract) -> usize {
    serde_json::to_vec(&contract_value(contract))
        .expect("a replay contract is always JSON serializable")
        .len()
}

fn encoded_delta_len(delta: &ModelReplayDelta) -> usize {
    let value = json!({
        "contract": delta.contract.as_ref().map(contract_value),
        "items": delta.items.iter().map(item_value).collect::<Vec<_>>(),
    });
    serde_json::to_vec(&value)
        .expect("a replay delta is always JSON serializable")
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
    items
        .iter()
        .map(|item| {
            serde_json::to_vec(&item_value(item))
                .expect("a replay item is always JSON serializable")
                .len()
        })
        .sum()
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
    }
}
