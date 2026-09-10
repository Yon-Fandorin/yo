use serde_json::json;

use super::{
    super::{
        TOOL_SCHEMA_DIALECT, ToolApprovalRequirement, ToolEffect, ToolRegistry,
        ToolValidationFailure,
    },
    support::{definition, definition_with_metadata, definition_with_schema},
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

// 실행 인자 한도는 모델 projection을 바꾸지 않는 host 정책이며 0바이트 설정은 거절한다.
#[test]
fn definition_argument_limit_is_nonzero_and_does_not_change_model_projection() {
    let original = definition("command", "command_tool");
    assert_eq!(original.argument_byte_limit(), None);
    assert!(original.clone().with_argument_byte_limit(0).is_err());
    let limited = original
        .clone()
        .with_argument_byte_limit(4 * 1024 * 1024)
        .unwrap();
    let original = ToolRegistry::new([original]).unwrap().freeze();
    let limited = ToolRegistry::new([limited]).unwrap().freeze();

    assert_eq!(
        limited.definitions()[0].argument_byte_limit(),
        Some(4 * 1024 * 1024)
    );
    assert_eq!(
        original.function_tools().unwrap(),
        limited.function_tools().unwrap()
    );
    assert_eq!(original.replay_tools(), limited.replay_tools());
}

// 공백으로 원문만 한계에 도달한 호출은 허용하되 첫 초과 바이트는 정규화 전에 거절한다.
#[test]
fn definition_raw_argument_limit_accepts_four_mebibytes_and_rejects_first_excess_byte() {
    const LIMIT: usize = 4 * 1024 * 1024;
    let registry = ToolRegistry::new([definition("command", "command_tool")
        .with_argument_byte_limit(LIMIT)
        .unwrap()])
    .unwrap()
    .freeze();
    let mut raw = r#"{"path":"a"}"#.to_owned();
    raw.extend(std::iter::repeat_n(' ', LIMIT - raw.len()));
    let call = registry
        .validate_call("call", "command_tool", &raw, 101 * 1024 * 1024)
        .unwrap();
    assert_eq!(call.argument_bytes().len(), LIMIT);
    assert_eq!(call.normalized_arguments(), br#"{"path":"a"}"#);
    raw.push(' ');
    assert_eq!(
        registry
            .validate_call("call", "command_tool", &raw, 101 * 1024 * 1024)
            .unwrap_err()
            .kind(),
        ToolValidationFailure::ArgumentLimit
    );
}

// JSON 자체가 원문 한도 안이어도 stdin의 마지막 LF까지 포함한 첫 초과 바이트는 거절한다.
#[test]
fn definition_normalized_limit_reserves_lf_at_the_four_mebibyte_boundary() {
    const LIMIT: usize = 4 * 1024 * 1024;
    let registry = ToolRegistry::new([definition("command", "command_tool")
        .with_argument_byte_limit(LIMIT)
        .unwrap()])
    .unwrap()
    .freeze();
    let framing = r#"{"path":""}"#.len();
    let raw = format!(r#"{{"path":"{}"}}"#, "a".repeat(LIMIT - framing - 1));
    let call = registry
        .validate_call("call", "command_tool", &raw, 101 * 1024 * 1024)
        .unwrap();
    let mut stdin = call.normalized_arguments().to_vec();
    stdin.push(b'\n');
    assert_eq!(stdin.len(), LIMIT);
    assert_eq!(&stdin[..stdin.len() - 1], raw.as_bytes());
    let excess = format!(r#"{{"path":"{}"}}"#, "a".repeat(LIMIT - framing));
    assert_eq!(excess.len(), LIMIT);
    assert_eq!(
        registry
            .validate_call("call", "command_tool", &excess, 101 * 1024 * 1024)
            .unwrap_err()
            .kind(),
        ToolValidationFailure::ArgumentLimit
    );
}

// 큰 도구별 한도가 작은 caller 한도를 넓히지 않고 원문과 JSON+LF 양쪽에 적용된다.
#[test]
fn definition_argument_limit_intersects_the_caller_cap_for_raw_and_normalized_json() {
    let registry = ToolRegistry::new([definition("command", "command_tool")
        .with_argument_byte_limit(4 * 1024 * 1024)
        .unwrap()])
    .unwrap()
    .freeze();
    let raw = r#"{"path":"a"}"#;
    assert!(
        registry
            .validate_call("call", "command_tool", raw, raw.len() + 1)
            .is_ok()
    );
    for (input, limit) in [
        (raw.to_owned(), raw.len()),
        (format!("{raw}  "), raw.len() + 1),
    ] {
        assert_eq!(
            registry
                .validate_call("call", "command_tool", &input, limit)
                .unwrap_err()
                .kind(),
            ToolValidationFailure::ArgumentLimit
        );
    }
}

// 숫자의 정규화로 JSON이 커지는 경우에도 실행에 넘기는 실제 bytes와 LF를 검사한다.
#[test]
fn normalized_number_expansion_is_checked_against_the_effective_limit() {
    let registry = ToolRegistry::new([definition_with_schema(
        "command",
        "command_tool",
        json!({
            "type": "object",
            "properties": {"n": {"type": "number"}},
            "required": ["n"],
            "additionalProperties": false
        }),
    )
    .unwrap()
    .with_argument_byte_limit(4 * 1024 * 1024)
    .unwrap()])
    .unwrap()
    .freeze();
    let raw = r#"{"n":1e1}"#;
    let normalized = br#"{"n":10.0}"#;
    let call = registry
        .validate_call("call", "command_tool", raw, normalized.len() + 1)
        .unwrap();
    assert_eq!(call.normalized_arguments(), normalized);
    assert_eq!(
        registry
            .validate_call("call", "command_tool", raw, normalized.len())
            .unwrap_err()
            .kind(),
        ToolValidationFailure::ArgumentLimit
    );
}

// 새 한도가 없는 built-in은 101 MiB caller 설정에서 4 MiB 초과 인자를 계속 허용한다.
// 기존 raw-only admission에는 새 JSON+LF 제한을 소급 적용하지 않는다.
#[test]
fn definitions_without_limits_preserve_large_legacy_and_raw_only_admission() {
    let registry = ToolRegistry::new([definition("write", "write_tool")])
        .unwrap()
        .freeze();
    let raw = format!(r#"{{"path":"{}"}}"#, "a".repeat(4 * 1024 * 1024));
    let call = registry
        .validate_call("call", "write_tool", &raw, 101 * 1024 * 1024)
        .unwrap();
    assert!(call.normalized_arguments().len() > 4 * 1024 * 1024);
    let small = r#"{"path":"a"}"#;
    assert!(
        registry
            .validate_call("call", "write_tool", small, small.len())
            .is_ok()
    );
    assert_eq!(
        registry
            .validate_call("call", "write_tool", small, small.len() - 1)
            .unwrap_err()
            .kind(),
        ToolValidationFailure::ArgumentLimit
    );
}

// worker용 accessor는 승인 digest와 같은 정규화 bytes를 제공하고 배열·숫자 종류를 보존한다.
// manifest 전용 floating-zero 규칙을 기존 호출 인자 normalization에 섞지 않는다.
#[test]
fn execution_argument_bytes_preserve_existing_recursive_normalization() {
    let registry = ToolRegistry::new([definition_with_schema(
        "command",
        "command_tool",
        json!({
            "type": "object",
            "properties": {
                "z": {"type": "array", "items": {"type": "number"}},
                "a": {
                    "type": "object",
                    "properties": {
                        "z": {"type": "string"},
                        "a": {"type": "string"}
                    },
                    "required": ["z", "a"],
                    "additionalProperties": false
                }
            },
            "required": ["z", "a"],
            "additionalProperties": false
        }),
    )
    .unwrap()
    .with_argument_byte_limit(4 * 1024 * 1024)
    .unwrap()])
    .unwrap()
    .freeze();
    let raw = r#"{ "z": [1.0,1,-0.0], "a": {"z":"\u0061", "a":"한"} }"#;
    let call = registry
        .validate_call("call", "command_tool", raw, 1024)
        .unwrap();
    assert_eq!(
        call.normalized_arguments(),
        r#"{"a":{"a":"한","z":"a"},"z":[1.0,1,-0.0]}"#.as_bytes()
    );
    assert_eq!(call.argument_bytes(), raw);
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
