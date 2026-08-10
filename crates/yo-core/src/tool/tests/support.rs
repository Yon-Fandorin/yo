use serde_json::{Value, json};

use super::super::{
    TOOL_SCHEMA_DIALECT, ToolApprovalRequirement, ToolDefinition, ToolEffect, ToolId,
};

pub(super) fn definition_with_schema(
    id: &str,
    name: &str,
    schema: Value,
) -> Result<ToolDefinition, super::super::ToolRegistryError> {
    ToolDefinition::new(
        ToolId::new(id).unwrap(),
        name,
        "tool fixture",
        TOOL_SCHEMA_DIALECT,
        schema,
        ToolEffect::ReadOnly,
        ToolApprovalRequirement::Required,
    )
}

pub(super) fn definition_with_metadata(
    id: &str,
    name: &str,
    description: &str,
    schema_version: &str,
    schema: Value,
    effect: ToolEffect,
    approval: ToolApprovalRequirement,
) -> Result<ToolDefinition, super::super::ToolRegistryError> {
    ToolDefinition::new(
        ToolId::new(id).unwrap(),
        name,
        description,
        schema_version,
        schema,
        effect,
        approval,
    )
}

pub(super) fn basic_schema() -> Value {
    json!({
        "type": "object",
        "properties": {"path": {"type": "string"}},
        "required": ["path"],
        "additionalProperties": false
    })
}

pub(super) fn definition(id: &str, name: &str) -> ToolDefinition {
    ToolDefinition::new(
        ToolId::new(id).unwrap(),
        name,
        "reads one path",
        TOOL_SCHEMA_DIALECT,
        basic_schema(),
        ToolEffect::ReadOnly,
        ToolApprovalRequirement::Required,
    )
    .unwrap()
}
