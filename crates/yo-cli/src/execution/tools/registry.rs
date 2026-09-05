use serde_json::{Value, json};
use yo_core::{
    ModelReplayContract, TOOL_SCHEMA_DIALECT, ToolApprovalRequirement, ToolDefinition, ToolEffect,
    ToolExecutionError, ToolId, ToolRegistry,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LocalToolRegistryRevision {
    BasicFiles,
    LegacyReadFile,
    NoTools,
}

impl LocalToolRegistryRevision {
    pub(crate) const fn maximum_argument_bytes(self) -> usize {
        match self {
            Self::BasicFiles => 101 * 1024 * 1024,
            Self::LegacyReadFile | Self::NoTools => 4 * 1024 * 1024,
        }
    }
}

pub(crate) fn registry(
    revision: LocalToolRegistryRevision,
) -> Result<ToolRegistry, ToolExecutionError> {
    match revision {
        LocalToolRegistryRevision::BasicFiles => basic_registry(),
        LocalToolRegistryRevision::LegacyReadFile => legacy_registry(),
        LocalToolRegistryRevision::NoTools => Ok(ToolRegistry::default()),
    }
}

pub(crate) fn revision_for_replay_contract(
    contract: Option<&ModelReplayContract>,
) -> Result<LocalToolRegistryRevision, ToolExecutionError> {
    let tools = contract
        .ok_or_else(|| ToolExecutionError::new("saved Session has no model replay contract"))?
        .tools();
    for revision in [
        LocalToolRegistryRevision::BasicFiles,
        LocalToolRegistryRevision::LegacyReadFile,
        LocalToolRegistryRevision::NoTools,
    ] {
        let trusted = registry(revision)?.freeze().replay_tools();
        if tools == trusted.as_slice() {
            return Ok(revision);
        }
    }
    Err(ToolExecutionError::new(
        "saved Session uses an unknown local tool registry",
    ))
}

fn basic_registry() -> Result<ToolRegistry, ToolExecutionError> {
    ToolRegistry::new([
        definition(
            "list-files",
            "list_files",
            "List immediate children of one directory inside the current workspace.",
            path_schema("Workspace-relative directory path."),
            ToolEffect::ReadOnly,
            ToolApprovalRequirement::Automatic,
        )?,
        definition(
            "read-files",
            "read_files",
            "Read 1–8 ordered UTF-8 file windows from the workspace; batch related files in one call. Each result is content or a per-file error. Continue unread lines from next_offset.",
            read_files_schema(),
            ToolEffect::ReadOnly,
            ToolApprovalRequirement::Automatic,
        )?,
        definition(
            "edit-file",
            "edit_file",
            "Atomically replace 1–256 unique, non-overlapping exact text matches in one UTF-8 workspace file.",
            edit_file_schema(),
            ToolEffect::WorkspaceWrite,
            ToolApprovalRequirement::Automatic,
        )?,
        definition(
            "write-file",
            "write_file",
            "Atomically create or replace one complete UTF-8 file under an existing workspace directory.",
            write_file_schema(),
            ToolEffect::WorkspaceWrite,
            ToolApprovalRequirement::Automatic,
        )?,
        run_command_definition()?,
    ])
    .map_err(|error| ToolExecutionError::new(error.to_string()))
}

fn legacy_registry() -> Result<ToolRegistry, ToolExecutionError> {
    ToolRegistry::new([
        definition(
            "read-file",
            "read_file",
            "Read one UTF-8 file inside the current workspace.",
            legacy_path_schema(),
            ToolEffect::ReadOnly,
            ToolApprovalRequirement::Automatic,
        )?,
        definition(
            "list-files",
            "list_files",
            "List immediate children of one directory inside the current workspace.",
            legacy_path_schema(),
            ToolEffect::ReadOnly,
            ToolApprovalRequirement::Automatic,
        )?,
        run_command_definition()?,
    ])
    .map_err(|error| ToolExecutionError::new(error.to_string()))
}

fn run_command_definition() -> Result<ToolDefinition, ToolExecutionError> {
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
    )
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

fn legacy_path_schema() -> Value {
    path_schema("Workspace-relative path")
}

fn path_schema(description: &str) -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": description
            }
        },
        "required": ["path"],
        "additionalProperties": false
    })
}

fn read_files_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "files": {
                "type": "array",
                "description": "Ordered file windows to read together.",
                "items": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Workspace-relative file path."
                        },
                        "offset": {
                            "type": "integer",
                            "description": "First logical line, 1-based; default 1."
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum logical lines, 1–400; default 400."
                        }
                    },
                    "required": ["path"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["files"],
        "additionalProperties": false
    })
}

fn edit_file_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Workspace-relative file path."
            },
            "edits": {
                "type": "array",
                "description": "Ordered exact replacements matched against the original file.",
                "items": {
                    "type": "object",
                    "properties": {
                        "oldText": {
                            "type": "string",
                            "description": "Non-empty text that must occur exactly once."
                        },
                        "newText": {
                            "type": "string",
                            "description": "Replacement text; may be empty."
                        }
                    },
                    "required": ["oldText", "newText"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["path", "edits"],
        "additionalProperties": false
    })
}

fn write_file_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Workspace-relative file path."
            },
            "content": {
                "type": "string",
                "description": "Complete UTF-8 file content."
            }
        },
        "required": ["path", "content"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use yo_core::{ModelReplayContract, ModelReplayTool, ToolApprovalRequirement, ToolEffect};

    use super::{LocalToolRegistryRevision, registry, revision_for_replay_contract};

    // 새 Session과 구형 Session의 wire projection을 정확히 구분해, 재시작이 구형 세션에
    // write 도구를 섞거나 신규 세션에 두 경쟁 read schema를 노출하지 않습니다.
    #[test]
    fn registry_revisions_are_closed_and_resume_by_exact_projection() {
        let basic = registry(LocalToolRegistryRevision::BasicFiles)
            .unwrap()
            .freeze();
        let legacy = registry(LocalToolRegistryRevision::LegacyReadFile)
            .unwrap()
            .freeze();
        assert_eq!(
            basic
                .definitions()
                .iter()
                .map(|definition| definition.wire_name())
                .collect::<Vec<_>>(),
            [
                "list_files",
                "read_files",
                "edit_file",
                "write_file",
                "run_command"
            ]
        );
        assert_eq!(
            legacy
                .definitions()
                .iter()
                .map(|definition| definition.wire_name())
                .collect::<Vec<_>>(),
            ["read_file", "list_files", "run_command"]
        );
        let shallow_description =
            "List immediate children of one directory inside the current workspace.";
        assert_eq!(basic.definitions()[0].description(), shallow_description);
        assert_eq!(legacy.definitions()[1].description(), shallow_description);
        let empty = registry(LocalToolRegistryRevision::NoTools)
            .unwrap()
            .freeze();
        for (revision, frozen) in [
            (LocalToolRegistryRevision::BasicFiles, basic),
            (LocalToolRegistryRevision::LegacyReadFile, legacy),
            (LocalToolRegistryRevision::NoTools, empty),
        ] {
            let contract = ModelReplayContract::new("system", frozen.replay_tools());
            assert_eq!(
                revision_for_replay_contract(Some(&contract)).unwrap(),
                revision
            );
        }
    }

    // 저장된 projection은 이름만 비슷한 값이나 혼합 schema를 신뢰하지 않고 read-only
    // 시작 경계로 보내기 위해 exact manifest 비교에서 거절합니다.
    #[test]
    fn resume_rejects_missing_or_unknown_registry_projection() {
        assert!(revision_for_replay_contract(None).is_err());
        let unknown = ModelReplayContract::new(
            "system",
            vec![ModelReplayTool::new(
                "read_files",
                "different",
                "yo.tool-schema/v1",
                json!({"type":"object"}),
            )],
        );
        assert!(revision_for_replay_contract(Some(&unknown)).is_err());

        let retired = registry(LocalToolRegistryRevision::LegacyReadFile)
            .unwrap()
            .freeze()
            .replay_tools()
            .into_iter()
            .map(|tool| {
                ModelReplayTool::new(
                    tool.name(),
                    if tool.name() == "list_files" {
                        "List files recursively below one directory inside the current workspace."
                    } else {
                        tool.description()
                    },
                    tool.schema_version(),
                    tool.parameters().clone(),
                )
            })
            .collect();
        assert!(
            revision_for_replay_contract(Some(&ModelReplayContract::new("system", retired)))
                .is_err()
        );
    }

    // basic manifest의 description/schema/effect/approval을 각각 직접 관찰해 이름만 맞는
    // 잘못된 registry가 exact durable projection test를 자기 자신과 비교해 통과하지 않습니다.
    #[test]
    fn basic_registry_manifest_matches_the_frozen_projection() {
        let frozen = registry(LocalToolRegistryRevision::BasicFiles)
            .unwrap()
            .freeze();
        let read = &frozen.definitions()[1];
        assert_eq!(read.id().as_str(), "read-files");
        assert_eq!(
            read.description(),
            "Read 1–8 ordered UTF-8 file windows from the workspace; batch related files in one call. Each result is content or a per-file error. Continue unread lines from next_offset."
        );
        assert_eq!(read.schema_version(), "yo.tool-schema/v1");
        assert_eq!(read.effect(), ToolEffect::ReadOnly);
        assert_eq!(read.approval(), ToolApprovalRequirement::Automatic);
        assert_eq!(
            read.input_schema(),
            &json!({
                "type":"object",
                "properties":{"files":{
                    "type":"array",
                    "description":"Ordered file windows to read together.",
                    "items":{
                        "type":"object",
                        "properties":{
                            "path":{"type":"string","description":"Workspace-relative file path."},
                            "offset":{"type":"integer","description":"First logical line, 1-based; default 1."},
                            "limit":{"type":"integer","description":"Maximum logical lines, 1–400; default 400."}
                        },
                        "required":["path"],
                        "additionalProperties":false
                    }
                }},
                "required":["files"],
                "additionalProperties":false
            })
        );
        let write = &frozen.definitions()[3];
        assert_eq!(write.effect(), ToolEffect::WorkspaceWrite);
        assert_eq!(write.approval(), ToolApprovalRequirement::Automatic);
        let command = &frozen.definitions()[4];
        assert_eq!(command.effect(), ToolEffect::Process);
        assert_eq!(command.approval(), ToolApprovalRequirement::Required);
    }
}
