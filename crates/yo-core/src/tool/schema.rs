use std::collections::HashSet;

use serde_json::Value;

use super::errors::ToolRegistryError;

pub(super) const MAX_ID_BYTES: usize = 128;
pub(super) const MAX_DESCRIPTION_BYTES: usize = 4 * 1024;
pub(super) const MAX_SCHEMA_BYTES: usize = 64 * 1024;

/// Closed JSON-schema subset accepted for local tool argument admission.
pub const TOOL_SCHEMA_DIALECT: &str = "yo.tool-schema/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolEffect {
    ReadOnly,
    WorkspaceWrite,
    Process,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolApprovalRequirement {
    Automatic,
    Required,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ToolId(String);

impl ToolId {
    pub fn new(value: impl Into<String>) -> Result<Self, ToolRegistryError> {
        let value = value.into();
        validate_wire_name(&value, "ToolId")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolDefinition {
    id: ToolId,
    wire_name: String,
    description: String,
    schema_version: String,
    input_schema: Value,
    effect: ToolEffect,
    approval: ToolApprovalRequirement,
}

impl ToolDefinition {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: ToolId,
        wire_name: impl Into<String>,
        description: impl Into<String>,
        schema_version: impl Into<String>,
        input_schema: Value,
        effect: ToolEffect,
        approval: ToolApprovalRequirement,
    ) -> Result<Self, ToolRegistryError> {
        let wire_name = wire_name.into();
        validate_wire_name(&wire_name, "tool wire name")?;
        let description = description.into();
        if description.is_empty()
            || description.len() > MAX_DESCRIPTION_BYTES
            || description.chars().any(char::is_control)
        {
            return Err(ToolRegistryError::new(
                "tool description must be non-empty, bounded, and free of control characters",
            ));
        }
        let schema_version = schema_version.into();
        if schema_version != TOOL_SCHEMA_DIALECT {
            return Err(ToolRegistryError::new(format!(
                "unsupported tool schema version; expected {TOOL_SCHEMA_DIALECT}"
            )));
        }
        let encoded_schema = serde_json::to_vec(&input_schema)
            .map_err(|_| ToolRegistryError::new("tool input schema cannot be encoded"))?;
        if encoded_schema.len() > MAX_SCHEMA_BYTES {
            return Err(ToolRegistryError::new(
                "tool input schema exceeds its byte limit",
            ));
        }
        validate_schema_definition(&input_schema)?;
        Ok(Self {
            id,
            wire_name,
            description,
            schema_version,
            input_schema,
            effect,
            approval,
        })
    }

    pub const fn id(&self) -> &ToolId {
        &self.id
    }

    pub fn wire_name(&self) -> &str {
        &self.wire_name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    pub const fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    pub const fn effect(&self) -> ToolEffect {
        self.effect
    }

    pub const fn approval(&self) -> ToolApprovalRequirement {
        self.approval
    }
}

fn validate_wire_name(value: &str, label: &str) -> Result<(), ToolRegistryError> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        Err(ToolRegistryError::new(format!(
            "{label} must be a bounded ASCII identifier"
        )))
    } else {
        Ok(())
    }
}

pub(super) fn normalize_json(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.clone(), normalize_json(value)))
                    .collect(),
            )
        },
        Value::Array(values) => Value::Array(values.iter().map(normalize_json).collect()),
        _ => value.clone(),
    }
}

fn validate_schema_definition(schema: &Value) -> Result<(), ToolRegistryError> {
    let object = schema
        .as_object()
        .ok_or_else(|| ToolRegistryError::new("tool input schema must be an object"))?;
    if object.get("type").and_then(Value::as_str) != Some("object") {
        return Err(ToolRegistryError::new(
            "tool input schema root type must be object",
        ));
    }
    validate_schema_node(schema, 0)
}

fn validate_schema_node(schema: &Value, depth: usize) -> Result<(), ToolRegistryError> {
    if depth > 16 {
        return Err(ToolRegistryError::new(
            "tool input schema is nested too deeply",
        ));
    }
    let object = schema
        .as_object()
        .ok_or_else(|| ToolRegistryError::new("schema node must be an object"))?;
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "type"
                | "description"
                | "properties"
                | "required"
                | "additionalProperties"
                | "items"
                | "enum"
        ) {
            return Err(ToolRegistryError::new(format!(
                "unsupported tool schema keyword `{key}`"
            )));
        }
    }
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolRegistryError::new("every schema node requires a string type"))?;
    if !matches!(
        kind,
        "object" | "array" | "string" | "number" | "integer" | "boolean" | "null"
    ) {
        return Err(ToolRegistryError::new("unsupported tool schema type"));
    }
    if let Some(description) = object.get("description") {
        let description = description
            .as_str()
            .ok_or_else(|| ToolRegistryError::new("schema description must be a string"))?;
        if description.is_empty()
            || description.len() > MAX_DESCRIPTION_BYTES
            || description.chars().any(char::is_control)
        {
            return Err(ToolRegistryError::new(
                "schema description must be non-empty, bounded, and free of control characters",
            ));
        }
    }
    match kind {
        "object" => {
            let properties = match object.get("properties") {
                Some(properties) => properties
                    .as_object()
                    .ok_or_else(|| ToolRegistryError::new("schema properties must be an object"))?
                    .clone(),
                None => serde_json::Map::new(),
            };
            for (name, child) in &properties {
                if name.is_empty()
                    || name.len() > MAX_ID_BYTES
                    || name.chars().any(char::is_control)
                {
                    return Err(ToolRegistryError::new(
                        "schema property names must be non-empty, bounded, and free of control characters",
                    ));
                }
                validate_schema_node(child, depth + 1)?;
            }
            let mut required_names = HashSet::new();
            if let Some(required) = object.get("required") {
                let required = required
                    .as_array()
                    .ok_or_else(|| ToolRegistryError::new("schema required must be an array"))?;
                for name in required {
                    let name = name.as_str().ok_or_else(|| {
                        ToolRegistryError::new("schema required entries must be strings")
                    })?;
                    if !required_names.insert(name) {
                        return Err(ToolRegistryError::new(
                            "schema required entries must be unique",
                        ));
                    }
                    if !properties.contains_key(name) {
                        return Err(ToolRegistryError::new(
                            "schema required entries must name declared properties",
                        ));
                    }
                }
            }
            if object.get("additionalProperties") != Some(&Value::Bool(false)) {
                return Err(ToolRegistryError::new(
                    "object schemas must set additionalProperties to false",
                ));
            }
            if object.contains_key("items") {
                return Err(ToolRegistryError::new(
                    "object schemas cannot declare array items",
                ));
            }
        },
        "array" => {
            if object.contains_key("properties")
                || object.contains_key("required")
                || object.contains_key("additionalProperties")
            {
                return Err(ToolRegistryError::new(
                    "array schemas cannot declare object-only keywords",
                ));
            }
            let items = object
                .get("items")
                .ok_or_else(|| ToolRegistryError::new("array schemas require items"))?;
            validate_schema_node(items, depth + 1)?;
        },
        _ => {
            if object.contains_key("properties")
                || object.contains_key("required")
                || object.contains_key("additionalProperties")
                || object.contains_key("items")
            {
                return Err(ToolRegistryError::new(
                    "scalar schemas cannot declare object or array keywords",
                ));
            }
        },
    }
    if let Some(values) = object.get("enum") {
        let values = values
            .as_array()
            .ok_or_else(|| ToolRegistryError::new("schema enum must be an array"))?;
        if values.is_empty() {
            return Err(ToolRegistryError::new("schema enum must not be empty"));
        }
        for (index, value) in values.iter().enumerate() {
            if !instance_matches_type(kind, value) {
                return Err(ToolRegistryError::new(
                    "schema enum values must match the node type",
                ));
            }
            if values[..index].contains(value) {
                return Err(ToolRegistryError::new("schema enum values must be unique"));
            }
        }
    }
    Ok(())
}

fn instance_matches_type(kind: &str, instance: &Value) -> bool {
    match kind {
        "object" => instance.is_object(),
        "array" => instance.is_array(),
        "string" => instance.is_string(),
        "number" => instance.is_number(),
        "integer" => instance.as_i64().is_some() || instance.as_u64().is_some(),
        "boolean" => instance.is_boolean(),
        "null" => instance.is_null(),
        _ => false,
    }
}

pub(super) fn validate_instance(
    schema: &Value,
    instance: &Value,
    path: &str,
    depth: usize,
) -> Result<(), String> {
    if depth > 16 {
        return Err(format!("{path} exceeds the schema nesting limit"));
    }
    let object = schema
        .as_object()
        .ok_or_else(|| format!("{path} schema is not an object"))?;
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{path} schema has no type"))?;
    let type_matches = instance_matches_type(kind, instance);
    if !type_matches {
        return Err(format!("{path} does not match schema type `{kind}`"));
    }
    if let Some(values) = object.get("enum").and_then(Value::as_array)
        && !values.contains(instance)
    {
        return Err(format!("{path} is not one of the admitted enum values"));
    }
    if let Some(instance) = instance.as_object() {
        let properties = object
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if let Some(required) = object.get("required").and_then(Value::as_array) {
            for name in required.iter().filter_map(Value::as_str) {
                if !instance.contains_key(name) {
                    return Err(format!("{path}.{name} is required"));
                }
            }
        }
        if object.get("additionalProperties") == Some(&Value::Bool(false)) {
            for name in instance.keys() {
                if !properties.contains_key(name) {
                    return Err(format!("{path}.{name} is not admitted by the schema"));
                }
            }
        }
        for (name, child_schema) in properties {
            if let Some(child) = instance.get(&name) {
                validate_instance(&child_schema, child, &format!("{path}.{name}"), depth + 1)?;
            }
        }
    }
    if let (Some(items), Some(instance)) = (object.get("items"), instance.as_array()) {
        for (index, child) in instance.iter().enumerate() {
            validate_instance(items, child, &format!("{path}[{index}]"), depth + 1)?;
        }
    }
    Ok(())
}
