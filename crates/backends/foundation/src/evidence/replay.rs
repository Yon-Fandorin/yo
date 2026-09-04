use std::{collections::HashSet, fmt};

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error as _, MapAccess, SeqAccess, Visitor},
    ser::{SerializeMap, SerializeSeq},
};
use serde_json::{Value, json};

use super::{valid_schema, valid_value};

const MAX_REPLAY_ITEMS: usize = 4_096;
pub(super) const MAX_REPLAY_TEXT_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MAX_REPLAY_CONTRACT_BYTES: usize = 1024 * 1024;
pub(super) const MAX_REPLAY_DELTA_BYTES: usize = 16 * 1024 * 1024;
const MAX_REPLAY_PREFIX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelReplayBudget {
    encoded_prefix_bytes: usize,
    prefix_items: usize,
}

impl ModelReplayBudget {
    pub fn accepts_item_lengths(&self, item_lengths: &[usize]) -> bool {
        self.encoded_len_with_item_lengths(item_lengths)
            .is_some_and(|bytes| bytes <= MAX_REPLAY_DELTA_BYTES)
    }

    pub fn encoded_len_with_item_lengths(&self, item_lengths: &[usize]) -> Option<usize> {
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

    #[doc(hidden)]
    pub fn is_valid(&self) -> bool {
        let mut names = HashSet::new();
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
        envelope: ProviderPrivateReplayEnvelope,
    },
}

#[derive(Clone, Eq, PartialEq)]
pub struct ProviderPrivateReplayEnvelope {
    schema: String,
    payload: Vec<u8>,
}

#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
pub enum ProviderPrivateReplayPayload {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    Object(Vec<(String, Self)>),
}

impl fmt::Debug for ProviderPrivateReplayEnvelope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderPrivateReplayEnvelope")
            .field("schema", &self.schema)
            .field("payload_bytes", &self.payload.len())
            .finish()
    }
}

impl ProviderPrivateReplayEnvelope {
    pub fn new(schema: impl Into<String>, payload: Vec<u8>) -> Result<Self, &'static str> {
        let envelope = Self {
            schema: schema.into(),
            payload,
        };
        if !envelope.is_valid() {
            return Err("provider-private replay envelope is invalid or exceeds its bounds");
        }
        Ok(envelope)
    }

    pub fn schema(&self) -> &str {
        &self.schema
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    #[doc(hidden)]
    pub fn ordered_payload(&self) -> ProviderPrivateReplayPayload {
        serde_json::from_slice(&self.payload)
            .expect("a validated provider-private payload remains ordered canonical JSON")
    }

    fn is_valid(&self) -> bool {
        valid_versioned_schema(&self.schema)
            && !self.payload.is_empty()
            && self.payload.len() <= MAX_REPLAY_TEXT_BYTES
            && serde_json::from_slice::<ProviderPrivateReplayPayload>(&self.payload).is_ok_and(
                |value| {
                    matches!(value, ProviderPrivateReplayPayload::Object(_))
                        && serde_json::to_vec(&value)
                            .is_ok_and(|canonical| canonical == self.payload)
                },
            )
    }
}

impl Serialize for ProviderPrivateReplayPayload {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Null => serializer.serialize_unit(),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Number(value) => value.serialize(serializer),
            Self::String(value) => serializer.serialize_str(value),
            Self::Array(values) => {
                let mut sequence = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    sequence.serialize_element(value)?;
                }
                sequence.end()
            },
            Self::Object(fields) => {
                let mut mapping = serializer.serialize_map(Some(fields.len()))?;
                for (key, value) in fields {
                    mapping.serialize_entry(key, value)?;
                }
                mapping.end()
            },
        }
    }
}

impl<'de> Deserialize<'de> for ProviderPrivateReplayPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PayloadVisitor;

        impl<'de> Visitor<'de> for PayloadVisitor {
            type Value = ProviderPrivateReplayPayload;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("JSON without duplicate object members")
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(ProviderPrivateReplayPayload::Null)
            }

            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(ProviderPrivateReplayPayload::Null)
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
                Ok(ProviderPrivateReplayPayload::Bool(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
                Ok(ProviderPrivateReplayPayload::Number(value.into()))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
                Ok(ProviderPrivateReplayPayload::Number(value.into()))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                serde_json::Number::from_f64(value)
                    .map(ProviderPrivateReplayPayload::Number)
                    .ok_or_else(|| E::custom("provider-private JSON number is not finite"))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
                Ok(ProviderPrivateReplayPayload::String(value.to_owned()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
                Ok(ProviderPrivateReplayPayload::String(value))
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element()? {
                    values.push(value);
                }
                Ok(ProviderPrivateReplayPayload::Array(values))
            }

            fn visit_map<A>(self, mut mapping: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut keys = HashSet::new();
                let mut fields = Vec::new();
                while let Some((key, value)) = mapping.next_entry::<String, _>()? {
                    if !keys.insert(key.clone()) {
                        return Err(A::Error::custom("duplicate provider-private JSON member"));
                    }
                    fields.push((key, value));
                }
                Ok(ProviderPrivateReplayPayload::Object(fields))
            }
        }

        deserializer.deserialize_any(PayloadVisitor)
    }
}

fn valid_versioned_schema(value: &str) -> bool {
    let Some((name, version)) = value.rsplit_once("/v") else {
        return false;
    };
    valid_schema(value)
        && !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        && version.as_bytes().first().is_some_and(u8::is_ascii_digit)
        && version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
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
            Self::ProviderPrivateAssistant { envelope } => envelope.is_valid(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModelReplayDelta {
    contract: Option<ModelReplayContract>,
    items: Vec<ModelReplayItem>,
}

impl ModelReplayDelta {
    pub const MAX_ENCODED_BYTES: usize = MAX_REPLAY_DELTA_BYTES;

    pub fn new(contract: Option<ModelReplayContract>, items: Vec<ModelReplayItem>) -> Self {
        Self { contract, items }
    }

    pub const fn contract(&self) -> Option<&ModelReplayContract> {
        self.contract.as_ref()
    }

    pub fn items(&self) -> &[ModelReplayItem] {
        &self.items
    }

    #[doc(hidden)]
    pub fn is_valid(&self) -> bool {
        self.contract
            .as_ref()
            .is_none_or(ModelReplayContract::is_valid)
            && !self.items.is_empty()
            && self.fits_capacity()
            && self.items.iter().all(ModelReplayItem::is_valid)
    }

    #[doc(hidden)]
    pub fn fits_capacity(&self) -> bool {
        Self::prospective_encoded_len(self.contract.as_ref(), self.items.iter())
            .is_some_and(|bytes| bytes <= MAX_REPLAY_DELTA_BYTES)
    }

    #[doc(hidden)]
    pub fn prospective_encoded_len<'a>(
        contract: Option<&ModelReplayContract>,
        items: impl Iterator<Item = &'a ModelReplayItem>,
    ) -> Option<usize> {
        let items = items.collect::<Vec<_>>();
        (items.len() <= MAX_REPLAY_ITEMS).then(|| encoded_prefix_len(contract, items.into_iter()))
    }

    pub fn replay_budget<'a>(
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

    #[doc(hidden)]
    pub fn validate(&self) -> Result<(), &'static str> {
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

    /// Builds one replay root recovered from a durable context checkpoint.
    ///
    /// Unlike an ordinary request-local delta, a checkpoint root may use the
    /// complete 64-MiB replay-prefix budget. Callers still have to preserve
    /// correlated semantic groups before flattening them into `items`.
    #[doc(hidden)]
    pub fn from_checkpoint(
        contract: ModelReplayContract,
        items: Vec<ModelReplayItem>,
    ) -> Result<Self, &'static str> {
        if !contract.is_valid() || items.is_empty() || items.len() > MAX_REPLAY_ITEMS {
            return Err("context checkpoint replay root is invalid or exceeds its item bound");
        }
        if items.iter().any(|item| !item.is_valid()) {
            return Err("context checkpoint replay root contains an invalid item");
        }
        let (known_calls, answered_calls) = validate_replay_items(&items)?;
        let encoded_prefix_bytes = encoded_prefix_len(Some(&contract), items.iter());
        if encoded_prefix_bytes > MAX_REPLAY_PREFIX_BYTES {
            return Err("context checkpoint replay root exceeds its byte bound");
        }
        Ok(Self {
            contract: Some(contract),
            items,
            encoded_prefix_bytes,
            known_calls,
            answered_calls,
        })
    }

    #[doc(hidden)]
    pub fn apply(&mut self, delta: &ModelReplayDelta) -> Result<(), &'static str> {
        self.apply_inner(delta, false)
    }

    /// Applies the first replay delta produced by a replacement binding.
    ///
    /// The replacement starts from an already reconstructed replay seed, but
    /// its first completed request must establish the new binding's exact
    /// system/tool contract instead of inheriting the source contract as its
    /// own chain declaration.
    #[doc(hidden)]
    pub fn apply_binding_replacement(
        &mut self,
        delta: &ModelReplayDelta,
    ) -> Result<(), &'static str> {
        self.apply_inner(delta, true)
    }

    fn apply_inner(
        &mut self,
        delta: &ModelReplayDelta,
        replace_contract: bool,
    ) -> Result<(), &'static str> {
        let (delta_calls, delta_answers) = validate_replay_delta(delta)?;
        if replace_contract {
            if self.contract.is_none() || delta.contract.is_none() {
                return Err("replacement binding first replay delta requires its new contract");
            }
        } else {
            match (self.contract.is_some(), delta.contract.is_some()) {
                (false, false) => return Err("first model replay delta requires its contract"),
                (true, true) => return Err("model replay contract was declared more than once"),
                (false, true) | (true, false) => {},
            }
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
        let resulting_contract = if replace_contract {
            delta.contract.as_ref()
        } else {
            delta.contract.as_ref().or(self.contract.as_ref())
        };
        let encoded_prefix_bytes = encoded_prefix_len(
            resulting_contract,
            self.items.iter().chain(delta.items.iter()),
        );
        if encoded_prefix_bytes > MAX_REPLAY_PREFIX_BYTES {
            return Err("model replay prefix byte limit exceeded");
        }
        if replace_contract || self.contract.is_none() {
            let contract = delta
                .contract
                .as_ref()
                .expect("the selected replay-contract transition requires a contract");
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
    validate_replay_items(&delta.items)
}

fn validate_replay_items(
    items: &[ModelReplayItem],
) -> Result<
    (
        std::collections::BTreeSet<String>,
        std::collections::BTreeSet<String>,
    ),
    &'static str,
> {
    let mut known_calls = std::collections::BTreeSet::new();
    let mut answered_calls = std::collections::BTreeSet::new();
    for (index, item) in items.iter().enumerate() {
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
            ModelReplayItem::ProviderPrivateAssistant { .. } => {
                validate_provider_private_adjacency(items, index)?;
            },
        }
    }
    if known_calls.difference(&answered_calls).next().is_some() {
        return Err("model replay delta ends with an unmatched function call");
    }
    Ok((known_calls, answered_calls))
}

fn validate_provider_private_adjacency(
    items: &[ModelReplayItem],
    private_index: usize,
) -> Result<(), &'static str> {
    let mut group_start = private_index;
    while group_start > 0 && matches!(items[group_start - 1], ModelReplayItem::FunctionCall { .. })
    {
        group_start -= 1;
    }
    match group_start
        .checked_sub(1)
        .and_then(|index| items.get(index))
    {
        Some(ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            refusal: None,
            ..
        }) => Ok(()),
        _ => Err(
            "provider-private assistant requires one immediately preceding visible assistant group",
        ),
    }
}

#[doc(hidden)]
pub fn validate_provider_private_replay_sequence(
    items: &[ModelReplayItem],
    expected_schema: &str,
) -> Result<(), &'static str> {
    let mut index = 0;
    let mut assistant_groups = 0;
    while index < items.len() {
        match &items[index] {
            ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                refusal,
                ..
            } => {
                assistant_groups += 1;
                if refusal.is_some() {
                    return Err(
                        "provider-private replay cannot follow a refused assistant message",
                    );
                }
                index += 1;
                while matches!(items.get(index), Some(ModelReplayItem::FunctionCall { .. })) {
                    index += 1;
                }
                match items.get(index) {
                    Some(ModelReplayItem::ProviderPrivateAssistant { envelope })
                        if envelope.schema() == expected_schema =>
                    {
                        index += 1;
                    },
                    Some(ModelReplayItem::ProviderPrivateAssistant { .. }) => {
                        return Err(
                            "provider-private replay schema differs from its replay profile",
                        );
                    },
                    _ => {
                        return Err(
                            "provider-private replay requires every assistant group to end with its private item",
                        );
                    },
                }
            },
            ModelReplayItem::FunctionCall { .. } => {
                return Err("provider-private replay function call is outside an assistant group");
            },
            ModelReplayItem::ProviderPrivateAssistant { .. } => {
                return Err("provider-private replay item has no preceding assistant group");
            },
            ModelReplayItem::Message { .. } | ModelReplayItem::FunctionCallOutput { .. } => {
                index += 1;
            },
        }
    }
    if assistant_groups == 0 {
        return Err("provider-private exact replay requires a provider-private assistant item");
    }
    Ok(())
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
    let items = items.map(encoded_item_len).collect::<Vec<_>>();
    let empty = serde_json::to_vec(&json!({"contract": null, "items": []}))
        .expect("an empty replay prefix is JSON serializable")
        .len();
    let contract_len = contract.map_or(4, encoded_contract_len);
    empty - 4 + contract_len + items.iter().sum::<usize>() + items.len().saturating_sub(1)
}

fn encoded_item_len(item: &ModelReplayItem) -> usize {
    if let ModelReplayItem::ProviderPrivateAssistant { envelope } = item {
        let empty = ProviderPrivateReplayEnvelope::new(envelope.schema(), b"{}".to_vec())
            .expect("a validated provider-private schema accepts an empty object");
        return serde_json::to_vec(&item_value(&ModelReplayItem::ProviderPrivateAssistant {
            envelope: empty,
        }))
        .expect("a provider-private replay wrapper is JSON serializable")
        .len()
            - 2
            + envelope.payload().len();
    }
    serde_json::to_vec(&item_value(item))
        .expect("a replay item is always JSON serializable")
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
        ModelReplayItem::ProviderPrivateAssistant { envelope } => {
            let private: Value = serde_json::from_slice(envelope.payload())
                .expect("a provider-private replay envelope always contains JSON");
            json!({
                "kind": "provider_private_assistant",
                "schema": envelope.schema(),
                "message": private,
            })
        },
    }
}
