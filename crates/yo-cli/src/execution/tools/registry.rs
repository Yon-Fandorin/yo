use serde_json::{Value, json};
use yo_backend_managed::NativeModelBackend;
use yo_core::{
    ModelReplayContract, ModelReplayTool, TOOL_SCHEMA_DIALECT, ToolApprovalRequirement,
    ToolDefinition, ToolEffect, ToolExecutionError, ToolId, ToolRegistry,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LocalToolRegistryRevision {
    BasicFiles,
    BasicFilesV2,
    LegacyReadFile,
    NoTools,
    CommandTools,
    CommandToolsV2,
}

impl LocalToolRegistryRevision {
    pub(crate) const fn maximum_argument_bytes(self) -> usize {
        match self {
            Self::BasicFiles | Self::BasicFilesV2 | Self::CommandTools | Self::CommandToolsV2 => {
                101 * 1024 * 1024
            },
            Self::LegacyReadFile | Self::NoTools => 4 * 1024 * 1024,
        }
    }
}

pub(crate) fn registry(
    revision: LocalToolRegistryRevision,
) -> Result<ToolRegistry, ToolExecutionError> {
    match revision {
        LocalToolRegistryRevision::BasicFiles => basic_registry(),
        LocalToolRegistryRevision::BasicFilesV2 => {
            basic_registry_with_command(run_command_definition_v2()?)
        },
        LocalToolRegistryRevision::LegacyReadFile => legacy_registry(),
        LocalToolRegistryRevision::NoTools => Ok(ToolRegistry::default()),
        LocalToolRegistryRevision::CommandTools | LocalToolRegistryRevision::CommandToolsV2 => Err(
            ToolExecutionError::new("command tools require a frozen execution manifest"),
        ),
    }
}

pub(crate) fn revision_for_replay_contract(
    contract: Option<&ModelReplayContract>,
) -> Result<LocalToolRegistryRevision, ToolExecutionError> {
    let tools = contract
        .ok_or_else(|| ToolExecutionError::new("saved Session has no model replay contract"))?
        .tools();
    for revision in [
        LocalToolRegistryRevision::BasicFilesV2,
        LocalToolRegistryRevision::BasicFiles,
        LocalToolRegistryRevision::LegacyReadFile,
        LocalToolRegistryRevision::NoTools,
    ] {
        let trusted = registry(revision)?.freeze().replay_tools();
        if matches_saved_replay_tools(
            tools,
            &trusted,
            matches!(
                revision,
                LocalToolRegistryRevision::BasicFiles | LocalToolRegistryRevision::BasicFilesV2
            ),
        ) {
            return Ok(revision);
        }
    }
    Err(ToolExecutionError::new(
        "saved Session uses an unknown local tool registry",
    ))
}

/// 알려진 backend 상호작용 접미부만 정확한 로컬 도구 projection 뒤에 허용합니다.
/// 상호작용은 로컬 실행 도구나 configured-command manifest에 포함되지 않습니다.
pub(crate) fn matches_saved_replay_tools(
    recorded: &[ModelReplayTool],
    trusted_local: &[ModelReplayTool],
    allow_native_interactions: bool,
) -> bool {
    let Some(suffix) = recorded.strip_prefix(trusted_local) else {
        return false;
    };
    if suffix.is_empty() {
        return true;
    }
    if !allow_native_interactions || trusted_local.is_empty() {
        return false;
    }
    let [current_secret, historical_secret] = NativeModelBackend::known_secret_replay_tools();
    match suffix {
        [secret] => secret == &current_secret || secret == &historical_secret,
        [question, secret] => {
            question == &NativeModelBackend::known_ask_user_replay_tool()
                && secret == &current_secret
        },
        _ => false,
    }
}

fn basic_registry() -> Result<ToolRegistry, ToolExecutionError> {
    basic_registry_with_command(run_command_definition()?)
}

fn basic_registry_with_command(
    command: ToolDefinition,
) -> Result<ToolRegistry, ToolExecutionError> {
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
        command,
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
    command_definition(
        "Run one shell command in the current workspace after explicit user approval.",
        ToolApprovalRequirement::Required,
    )
}

fn run_command_definition_v2() -> Result<ToolDefinition, ToolExecutionError> {
    command_definition(
        "Run one shell command in the current workspace. Commands needing broader access or risky changes require approval.",
        ToolApprovalRequirement::Planned,
    )
}

fn command_definition(
    description: &str,
    approval: ToolApprovalRequirement,
) -> Result<ToolDefinition, ToolExecutionError> {
    definition(
        "run-command",
        "run_command",
        description,
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
        approval,
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
    use yo_backend_managed::NativeModelBackend;
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

    // 새 v2만 Planned를 선택하며 나머지 네 정의와 기록된 v1 projection은 정확히 보존한다.
    #[test]
    fn v2_registry_changes_only_the_explicit_command_definition() {
        let v1 = registry(LocalToolRegistryRevision::BasicFiles)
            .unwrap()
            .freeze();
        let v2 = registry(LocalToolRegistryRevision::BasicFilesV2)
            .unwrap()
            .freeze();
        assert_eq!(&v1.definitions()[..4], &v2.definitions()[..4]);
        assert_eq!(
            v1.definitions()[4].approval(),
            ToolApprovalRequirement::Required
        );
        assert_eq!(
            v2.definitions()[4].approval(),
            ToolApprovalRequirement::Planned
        );
        assert_eq!(
            v1.definitions()[4].input_schema(),
            v2.definitions()[4].input_schema()
        );
        for (frozen, revision) in [
            (v1, LocalToolRegistryRevision::BasicFiles),
            (v2, LocalToolRegistryRevision::BasicFilesV2),
        ] {
            assert_eq!(
                revision_for_replay_contract(Some(&ModelReplayContract::new(
                    "system",
                    frozen.replay_tools()
                )))
                .unwrap(),
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

    // Resume와 fork가 공유하는 startup admission은 로컬 도구 뒤의 정확한 native
    // 비밀 정의 한 개만 허용하고, 구형/no-tools registry에는 붙이지 않는다.
    #[test]
    fn saved_registry_admits_only_one_exact_final_native_secret_tool() {
        let basic = registry(LocalToolRegistryRevision::BasicFiles)
            .unwrap()
            .freeze()
            .replay_tools();
        for secret in NativeModelBackend::known_secret_replay_tools() {
            let mut recorded = basic.clone();
            recorded.push(secret.clone());
            let contract = ModelReplayContract::new("system", recorded.clone());
            assert_eq!(
                revision_for_replay_contract(Some(&contract)).unwrap(),
                LocalToolRegistryRevision::BasicFiles
            );

            let mut reordered = recorded.clone();
            let last = reordered.len() - 1;
            reordered.swap(0, last);
            assert!(
                revision_for_replay_contract(Some(&ModelReplayContract::new("system", reordered)))
                    .is_err()
            );
            let mut duplicate = recorded.clone();
            duplicate.push(secret.clone());
            assert!(
                revision_for_replay_contract(Some(&ModelReplayContract::new("system", duplicate)))
                    .is_err()
            );
            let mut altered = basic.clone();
            altered.push(ModelReplayTool::new(
                secret.name(),
                "altered secret request",
                secret.schema_version(),
                secret.parameters().clone(),
            ));
            assert!(
                revision_for_replay_contract(Some(&ModelReplayContract::new("system", altered)))
                    .is_err()
            );
            for revision in [
                LocalToolRegistryRevision::LegacyReadFile,
                LocalToolRegistryRevision::NoTools,
            ] {
                let mut disallowed = registry(revision).unwrap().freeze().replay_tools();
                disallowed.push(secret.clone());
                assert!(
                    revision_for_replay_contract(Some(&ModelReplayContract::new(
                        "system", disallowed
                    )))
                    .is_err()
                );
            }
        }
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
