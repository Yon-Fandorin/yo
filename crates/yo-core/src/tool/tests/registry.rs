use serde_json::json;

use super::{
    super::{
        TOOL_SCHEMA_DIALECT, ToolApprovalRequirement, ToolEffect, ToolRegistry,
        ToolValidationFailure,
    },
    support::{definition, definition_with_metadata},
};
use crate::{FunctionTool, ModelReplayTool};

// 도구 식별자와 모델 노출 이름은 각각 유일해야 하므로 중복 레지스트리를 거부한다.
#[test]
fn registry_rejects_duplicate_id_and_wire_name() {
    let duplicate_id = ToolRegistry::new([
        definition("read-one", "read_one"),
        definition("read-one", "read_two"),
    ]);
    assert!(duplicate_id.is_err());

    let duplicate_name = ToolRegistry::new([
        definition("read-one", "read_path"),
        definition("read-two", "read_path"),
    ]);
    assert!(duplicate_name.is_err());
}

// 알 수 없는 도구와 잘못된 JSON·스키마·크기 초과 인자는 실행 전에 유형화된 오류로 차단한다.
#[test]
fn frozen_registry_validates_the_exact_arguments_before_execution() {
    let registry = ToolRegistry::new([definition("read-one", "read_path")])
        .unwrap()
        .freeze();

    assert_eq!(
        registry
            .validate_call("call-1", "missing", "{}", 100)
            .unwrap_err()
            .kind(),
        ToolValidationFailure::UnknownTool
    );
    assert_eq!(
        registry
            .validate_call("call-1", "read_path", "{", 100)
            .unwrap_err()
            .kind(),
        ToolValidationFailure::InvalidJson
    );
    assert_eq!(
        registry
            .validate_call("call-1", "read_path", "{}", 100)
            .unwrap_err()
            .kind(),
        ToolValidationFailure::SchemaMismatch
    );
    assert_eq!(
        registry
            .validate_call("call-1", "read_path", r#"{"path":"a"}"#, 2)
            .unwrap_err()
            .kind(),
        ToolValidationFailure::ArgumentLimit
    );
    assert_eq!(
        ToolValidationFailure::SchemaMismatch.code(),
        "yo.tool.validation.schema-mismatch/v1"
    );
}

// 한 요청에 고정된 레지스트리는 커넥터와 리플레이에 같은 순서와 스키마를 투영한다.
#[test]
fn frozen_registry_projection_is_stable() {
    let registry = ToolRegistry::new([
        definition("read-one", "read_one"),
        definition("read-two", "read_two"),
    ])
    .unwrap()
    .freeze();

    let connector = registry.function_tools().unwrap();
    let replay = registry.replay_tools();
    assert_eq!(connector[0].name(), replay[0].name());
    assert_eq!(connector[1].name(), replay[1].name());
    assert_eq!(replay[0].schema_version(), TOOL_SCHEMA_DIALECT);
}

// connector와 replay projection은 description·schema·version을 모두 보존하고 registry insertion
// order를 유지한다.
#[test]
fn frozen_registry_projections_are_field_complete_and_ordered() {
    let first_schema = json!({
        "type": "object",
        "properties": {"path": {"type": "string"}},
        "required": ["path"],
        "additionalProperties": false
    });
    let second_schema = json!({
        "type": "object",
        "properties": {"count": {"type": "integer"}},
        "required": ["count"],
        "additionalProperties": false
    });
    let third_schema = json!({
        "type": "object",
        "properties": {"enabled": {"type": "boolean"}},
        "required": ["enabled"],
        "additionalProperties": false
    });
    let registry = ToolRegistry::new([
        definition_with_metadata(
            "z-id",
            "z_wire",
            "Zulu description",
            TOOL_SCHEMA_DIALECT,
            first_schema.clone(),
            ToolEffect::ReadOnly,
            ToolApprovalRequirement::Automatic,
        )
        .unwrap(),
        definition_with_metadata(
            "a-id",
            "a_wire",
            "Alpha description",
            TOOL_SCHEMA_DIALECT,
            second_schema.clone(),
            ToolEffect::WorkspaceWrite,
            ToolApprovalRequirement::Required,
        )
        .unwrap(),
        definition_with_metadata(
            "m-id",
            "m_wire",
            "Mike description",
            TOOL_SCHEMA_DIALECT,
            third_schema.clone(),
            ToolEffect::Process,
            ToolApprovalRequirement::Automatic,
        )
        .unwrap(),
    ])
    .unwrap()
    .freeze();

    let connector = registry.function_tools().unwrap();
    let replay = registry.replay_tools();
    assert_eq!(
        connector,
        vec![
            FunctionTool::new("z_wire", "Zulu description", first_schema.clone()).unwrap(),
            FunctionTool::new("a_wire", "Alpha description", second_schema.clone()).unwrap(),
            FunctionTool::new("m_wire", "Mike description", third_schema.clone()).unwrap(),
        ]
    );
    assert_eq!(
        replay,
        vec![
            ModelReplayTool::new(
                "z_wire",
                "Zulu description",
                TOOL_SCHEMA_DIALECT,
                first_schema,
            ),
            ModelReplayTool::new(
                "a_wire",
                "Alpha description",
                TOOL_SCHEMA_DIALECT,
                second_schema,
            ),
            ModelReplayTool::new(
                "m_wire",
                "Mike description",
                TOOL_SCHEMA_DIALECT,
                third_schema,
            ),
        ]
    );
}

// 현재 정의된 모든 ToolValidationFailure variant는 변경 없이 고유한 stable v1 code를 제공한다.
#[test]
fn current_tool_validation_failure_codes_are_stable_and_distinct() {
    let expected = [
        (
            ToolValidationFailure::InvalidIdentity,
            "yo.tool.validation.invalid-identity/v1",
        ),
        (
            ToolValidationFailure::ArgumentLimit,
            "yo.tool.validation.argument-limit/v1",
        ),
        (
            ToolValidationFailure::InvalidJson,
            "yo.tool.validation.invalid-json/v1",
        ),
        (
            ToolValidationFailure::SchemaMismatch,
            "yo.tool.validation.schema-mismatch/v1",
        ),
        (
            ToolValidationFailure::UnknownTool,
            "yo.tool.validation.unknown-tool/v1",
        ),
        (
            ToolValidationFailure::DuplicateIdentity,
            "yo.tool.validation.duplicate-identity/v1",
        ),
        (
            ToolValidationFailure::Unavailable,
            "yo.tool.validation.unavailable/v1",
        ),
        (
            ToolValidationFailure::ApprovalMismatch,
            "yo.tool.validation.approval-mismatch/v1",
        ),
        (
            ToolValidationFailure::SemanticAdmission,
            "yo.tool.validation.semantic-admission/v1",
        ),
    ];

    for (failure, code) in expected {
        assert_eq!(failure.code(), code);
    }
    let unique_codes = expected
        .iter()
        .map(|(failure, _)| failure.code())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(unique_codes.len(), expected.len());
}
