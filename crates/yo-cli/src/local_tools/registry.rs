use serde_json::{Value, json};
use yo_core::{
    TOOL_SCHEMA_DIALECT, ToolApprovalRequirement, ToolDefinition, ToolEffect, ToolExecutionError,
    ToolId, ToolRegistry,
};

pub(crate) fn registry() -> Result<ToolRegistry, ToolExecutionError> {
    ToolRegistry::new([
        definition(
            "read-file",
            "read_file",
            "Read one UTF-8 file inside the current workspace.",
            path_schema(),
            ToolEffect::ReadOnly,
            ToolApprovalRequirement::Automatic,
        )?,
        definition(
            "list-files",
            "list_files",
            "List files recursively below one directory inside the current workspace.",
            path_schema(),
            ToolEffect::ReadOnly,
            ToolApprovalRequirement::Automatic,
        )?,
        definition(
            "run-command",
            "run_command",
            "Run one shell command in the current workspace after explicit user approval.",
            json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command to run from the workspace root"
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
            ToolEffect::Process,
            ToolApprovalRequirement::Required,
        )?,
    ])
    .map_err(|error| ToolExecutionError::new(error.to_string()))
}

fn definition(
    id: &str,
    wire_name: &str,
    description: &str,
    schema: Value,
    effect: ToolEffect,
    approval: ToolApprovalRequirement,
) -> Result<ToolDefinition, ToolExecutionError> {
    ToolDefinition::new(
        ToolId::new(id).map_err(|error| ToolExecutionError::new(error.to_string()))?,
        wire_name,
        description,
        TOOL_SCHEMA_DIALECT,
        schema,
        effect,
        approval,
    )
    .map_err(|error| ToolExecutionError::new(error.to_string()))
}

fn path_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Workspace-relative path"
            }
        },
        "required": ["path"],
        "additionalProperties": false
    })
}
