use std::{
    collections::BTreeSet,
    io::{Error as IoError, Result as IoResult, Write},
};

use serde::Serialize;
use serde_json::{Value, json};

use super::{
    budget::MAX_REPLAY_PREFIX_BYTES,
    contract::{ModelReplayContract, encoded_contract_len},
    item::{ModelReplayItem, ModelReplayRole, is_valid_item},
    private::ProviderPrivateReplayEnvelope,
};
use crate::ModelInputPart;

pub(super) fn replay_delta_is_valid(
    contract: Option<&ModelReplayContract>,
    items: &[ModelReplayItem],
    fits_capacity: bool,
) -> bool {
    contract.is_none_or(ModelReplayContract::is_valid)
        && !items.is_empty()
        && fits_capacity
        && items.iter().all(is_valid_item)
}

pub(super) fn validate_replay_delta(
    contract: Option<&ModelReplayContract>,
    items: &[ModelReplayItem],
    fits_capacity: bool,
) -> Result<(BTreeSet<String>, BTreeSet<String>), &'static str> {
    if !replay_delta_is_valid(contract, items, fits_capacity) {
        return Err("model replay delta is invalid or exceeds its bounds");
    }
    validate_replay_items(items)
}

pub(super) fn validate_replay_items(
    items: &[ModelReplayItem],
) -> Result<(BTreeSet<String>, BTreeSet<String>), &'static str> {
    let mut known_calls = BTreeSet::new();
    let mut answered_calls = BTreeSet::new();
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
            ModelReplayItem::Message { .. } | ModelReplayItem::MultimodalUser { .. } => {},
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
            ModelReplayItem::Message { .. }
            | ModelReplayItem::MultimodalUser { .. }
            | ModelReplayItem::FunctionCallOutput { .. } => {
                index += 1;
            },
        }
    }
    if assistant_groups == 0 {
        return Err("provider-private exact replay requires a provider-private assistant item");
    }
    Ok(())
}

pub(super) fn encoded_prefix_len<'a>(
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
    if let ModelReplayItem::MultimodalUser { parts } = item {
        #[derive(Serialize)]
        struct MultimodalUser<'a> {
            kind: &'static str,
            parts: &'a [ModelInputPart],
        }
        struct EncodedBudget(usize);
        impl Write for EncodedBudget {
            fn write(&mut self, bytes: &[u8]) -> IoResult<usize> {
                self.0 = self
                    .0
                    .checked_add(bytes.len())
                    .filter(|length| *length <= MAX_REPLAY_PREFIX_BYTES)
                    .ok_or_else(|| {
                        IoError::other("multimodal replay exceeds the prefix byte budget")
                    })?;
                Ok(bytes.len())
            }
            fn flush(&mut self) -> IoResult<()> {
                Ok(())
            }
        }
        let mut budget = EncodedBudget(0);
        return if serde_json::to_writer(
            &mut budget,
            &MultimodalUser {
                kind: "multimodal_user",
                parts,
            },
        )
        .is_ok()
        {
            budget.0
        } else {
            MAX_REPLAY_PREFIX_BYTES + 1
        };
    }
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

fn item_value(item: &ModelReplayItem) -> Value {
    match item {
        ModelReplayItem::MultimodalUser { parts } => {
            json!({"kind": "multimodal_user", "parts": parts})
        },
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
