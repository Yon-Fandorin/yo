use std::collections::HashSet;

use serde_json::{Map, Value, json};

use super::{ConnectorError, FunctionTool, configuration_failure};
use crate::{TOOL_SCHEMA_DIALECT, ToolApprovalRequirement, ToolDefinition, ToolEffect, ToolId};

pub(super) fn strict_tool(tool: &FunctionTool) -> Result<Value, ConnectorError> {
    if !valid_function_name(tool.name())
        || ToolDefinition::new(
            ToolId::new("kimi_schema_probe").expect("the fixed tool ID is valid"),
            tool.name(),
            tool.description(),
            TOOL_SCHEMA_DIALECT,
            tool.parameters().clone(),
            ToolEffect::ReadOnly,
            ToolApprovalRequirement::Automatic,
        )
        .is_err()
        || !valid_mfjs_schema(tool.parameters(), 0)
    {
        return Err(configuration_failure(
            "Kimi strict tool is outside the admitted name or MFJS schema subset",
        ));
    }
    Ok(json!({
        "type": "function",
        "function": {
            "name": tool.name(),
            "description": tool.description(),
            "parameters": tool.parameters(),
            "strict": true,
        },
    }))
}

fn valid_function_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    (3..=64).contains(&bytes.len())
        && bytes
            .first()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn valid_mfjs_schema(value: &Value, depth: usize) -> bool {
    if depth > 64 {
        return false;
    }
    let Some(object) = value.as_object() else {
        return false;
    };
    const ALLOWED: [&str; 7] = [
        "type",
        "description",
        "properties",
        "required",
        "additionalProperties",
        "items",
        "enum",
    ];
    if object.keys().any(|key| !ALLOWED.contains(&key.as_str())) {
        return false;
    }
    let Some(kind) = object.get("type").and_then(Value::as_str) else {
        return false;
    };
    if !matches!(
        kind,
        "object" | "array" | "string" | "integer" | "number" | "boolean"
    ) {
        return false;
    }
    if object.get("description").is_some_and(|value| {
        !value
            .as_str()
            .is_some_and(|text| !text.is_empty() && text.len() <= 4 * 1024)
    }) {
        return false;
    }
    if let Some(properties) = object.get("properties") {
        let Some(properties) = properties.as_object() else {
            return false;
        };
        if kind != "object"
            || properties
                .values()
                .any(|child| !valid_mfjs_schema(child, depth + 1))
        {
            return false;
        }
    }
    if let Some(required) = object.get("required") {
        let Some(required) = required.as_array() else {
            return false;
        };
        let empty_properties = Map::new();
        let properties = object
            .get("properties")
            .and_then(Value::as_object)
            .unwrap_or(&empty_properties);
        let mut names = HashSet::new();
        if required.iter().any(|name| {
            !name
                .as_str()
                .is_some_and(|name| properties.contains_key(name) && names.insert(name))
        }) {
            return false;
        }
    }
    if object
        .get("additionalProperties")
        .is_some_and(|value| value != &Value::Bool(false))
    {
        return false;
    }
    if let Some(items) = object.get("items")
        && (kind != "array" || !valid_mfjs_schema(items, depth + 1))
    {
        return false;
    }
    if let Some(values) = object.get("enum") {
        let Some(values) = values.as_array().filter(|values| !values.is_empty()) else {
            return false;
        };
        let variant = enum_variant(&values[0]);
        if variant == 0 || values.iter().any(|value| enum_variant(value) != variant) {
            return false;
        }
    }
    true
}

fn enum_variant(value: &Value) -> u8 {
    match value {
        Value::String(_) => 1,
        Value::Number(number) if number.as_i64().is_some() || number.as_u64().is_some() => 2,
        Value::Number(number) if number.as_f64().is_some_and(f64::is_finite) => 3,
        _ => 0,
    }
}
