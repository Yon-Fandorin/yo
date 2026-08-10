use std::num::NonZeroU64;

use serde_json::{Value, json};

use super::{
    MAX_DESCRIPTION_BYTES, MAX_ID_BYTES, MAX_SCHEMA_BYTES, TOOL_SCHEMA_DIALECT,
    ToolApprovalBinding, ToolApprovalRequirement, ToolDefinition, ToolEffect, ToolId, ToolRegistry,
    ToolValidationFailure,
};
use crate::{FunctionTool, ModelReplayTool, TurnId, TurnRef, fixture_session};

fn definition_with_schema(
    id: &str,
    name: &str,
    schema: Value,
) -> Result<ToolDefinition, super::ToolRegistryError> {
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

fn definition_with_metadata(
    id: &str,
    name: &str,
    description: &str,
    schema_version: &str,
    schema: Value,
    effect: ToolEffect,
    approval: ToolApprovalRequirement,
) -> Result<ToolDefinition, super::ToolRegistryError> {
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

fn basic_schema() -> Value {
    json!({
        "type": "object",
        "properties": {"path": {"type": "string"}},
        "required": ["path"],
        "additionalProperties": false
    })
}

fn schema_with_exact_bytes(target: usize) -> Value {
    let mut schema = json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    });

    for index in 0..512 {
        let name = format!("p{index:03}{}", "x".repeat(124));
        schema["properties"]
            .as_object_mut()
            .unwrap()
            .insert(name, json!({"type": "string"}));

        let base_length = serde_json::to_vec(&schema).unwrap().len();
        let mut with_empty_description = schema.clone();
        with_empty_description["description"] = Value::String(String::new());
        let description_overhead =
            serde_json::to_vec(&with_empty_description).unwrap().len() - base_length;
        let description_length = target.saturating_sub(base_length + description_overhead);
        if (1..MAX_DESCRIPTION_BYTES).contains(&description_length) {
            schema["description"] = Value::String("d".repeat(description_length));
            assert_eq!(serde_json::to_vec(&schema).unwrap().len(), target);
            return schema;
        }
    }

    panic!("could not construct a schema of exactly {target} bytes");
}

fn nested_object_schema(depth: usize) -> Value {
    let mut schema = json!({"type": "string"});
    for _ in 0..depth {
        schema = json!({
            "type": "object",
            "properties": {"child": schema},
            "required": ["child"],
            "additionalProperties": false
        });
    }
    schema
}

fn nested_object_instance(depth: usize) -> Value {
    let mut instance = json!("leaf");
    for _ in 0..depth {
        instance = json!({"child": instance});
    }
    instance
}

fn definition(id: &str, name: &str) -> ToolDefinition {
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

fn turn(value: u64) -> TurnRef {
    TurnRef::new(
        fixture_session(1),
        TurnId::new(NonZeroU64::new(value).unwrap()),
    )
}

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

// ToolId와 wire name은 허용된 ASCII 구두점만 포함한 128 byte까지 보존하고 그 밖은 거부한다.
#[test]
fn tool_id_and_wire_name_enforce_ascii_byte_boundaries() {
    assert!(ToolId::new("tool_1-name.v2").is_ok());
    let max_id = "a".repeat(MAX_ID_BYTES);
    let tool_id = ToolId::new(max_id.clone()).unwrap();
    assert_eq!(tool_id.as_str(), max_id);

    for value in [
        String::new(),
        "a".repeat(MAX_ID_BYTES + 1),
        "bad/name".to_owned(),
        "bad\nname".to_owned(),
        "café".to_owned(),
    ] {
        assert!(ToolId::new(value).is_err());
    }

    let max_wire_name = "a".repeat(MAX_ID_BYTES);
    let wire_definition =
        definition_with_schema("wire-edge", &max_wire_name, basic_schema()).unwrap();
    assert_eq!(wire_definition.wire_name(), max_wire_name);
    for (index, wire_name) in [
        String::new(),
        "a".repeat(MAX_ID_BYTES + 1),
        "bad/name".to_owned(),
        "bad\u{7f}name".to_owned(),
        "café".to_owned(),
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            definition_with_schema(&format!("wire-invalid-{index}"), &wire_name, basic_schema())
                .is_err()
        );
    }
}

// 설명·dialect·schema byte bound는 비어 있지 않은 정확한 상한만 허용하고 한 byte 초과를 차단한다.
#[test]
fn descriptions_schema_version_and_schema_bytes_have_exact_edges() {
    let description_at_limit = "d".repeat(MAX_DESCRIPTION_BYTES);
    assert!(
        definition_with_metadata(
            "description-edge",
            "description_edge",
            &description_at_limit,
            TOOL_SCHEMA_DIALECT,
            basic_schema(),
            ToolEffect::ReadOnly,
            ToolApprovalRequirement::Automatic,
        )
        .is_ok()
    );
    assert!(
        definition_with_metadata(
            "description-over",
            "description_over",
            &format!("{description_at_limit}d"),
            TOOL_SCHEMA_DIALECT,
            basic_schema(),
            ToolEffect::ReadOnly,
            ToolApprovalRequirement::Automatic,
        )
        .is_err()
    );
    assert!(
        definition_with_metadata(
            "description-empty",
            "description_empty",
            "",
            TOOL_SCHEMA_DIALECT,
            basic_schema(),
            ToolEffect::ReadOnly,
            ToolApprovalRequirement::Automatic,
        )
        .is_err()
    );
    assert!(
        definition_with_metadata(
            "description-control",
            "description_control",
            "description\ncontrol",
            TOOL_SCHEMA_DIALECT,
            basic_schema(),
            ToolEffect::ReadOnly,
            ToolApprovalRequirement::Automatic,
        )
        .is_err()
    );

    let mut schema_description_at_limit = basic_schema();
    schema_description_at_limit["description"] = Value::String(description_at_limit.clone());
    assert!(
        definition_with_schema(
            "schema-description-edge",
            "schema_description_edge",
            schema_description_at_limit,
        )
        .is_ok()
    );
    let mut schema_description_over = basic_schema();
    schema_description_over["description"] = Value::String(format!("{description_at_limit}d"));
    assert!(
        definition_with_schema(
            "schema-description-over",
            "schema_description_over",
            schema_description_over,
        )
        .is_err()
    );

    assert!(
        definition_with_metadata(
            "schema-version-edge",
            "schema_version_edge",
            "version fixture",
            TOOL_SCHEMA_DIALECT,
            basic_schema(),
            ToolEffect::ReadOnly,
            ToolApprovalRequirement::Automatic,
        )
        .is_ok()
    );
    for version in ["", "yo.tool-schema/v0", "yo.tool-schema/v1 "] {
        assert!(
            definition_with_metadata(
                "schema-version-invalid",
                "schema_version_invalid",
                "version fixture",
                version,
                basic_schema(),
                ToolEffect::ReadOnly,
                ToolApprovalRequirement::Automatic,
            )
            .is_err()
        );
    }

    let schema_at_limit = schema_with_exact_bytes(MAX_SCHEMA_BYTES);
    assert_eq!(
        serde_json::to_vec(&schema_at_limit).unwrap().len(),
        MAX_SCHEMA_BYTES
    );
    assert!(definition_with_schema("schema-edge", "schema_edge", schema_at_limit).is_ok());

    let schema_over_limit = schema_with_exact_bytes(MAX_SCHEMA_BYTES + 1);
    assert_eq!(
        serde_json::to_vec(&schema_over_limit).unwrap().len(),
        MAX_SCHEMA_BYTES + 1
    );
    assert!(definition_with_schema("schema-over", "schema_over", schema_over_limit).is_err());
}

// schema node는 16단계까지 허용하고 17단계 schema는 거부하며, 허용된 16단계 instance도 검증한다.
#[test]
fn schema_nesting_limit_is_sixteen_levels() {
    assert!(
        definition_with_schema("depth-sixteen", "depth_sixteen", nested_object_schema(16)).is_ok()
    );
    assert!(
        definition_with_schema(
            "depth-seventeen",
            "depth_seventeen",
            nested_object_schema(17)
        )
        .is_err()
    );

    let registry = ToolRegistry::new([definition_with_schema(
        "depth-runtime",
        "depth_runtime",
        nested_object_schema(16),
    )
    .unwrap()])
    .unwrap()
    .freeze();
    let instance = serde_json::to_string(&nested_object_instance(16)).unwrap();
    assert!(
        registry
            .validate_call("call-depth", "depth_runtime", &instance, usize::MAX)
            .is_ok()
    );
}

// 모든 scalar·array·enum·nested object의 일치 인스턴스는 schema mismatch 없이 admission된다.
#[test]
fn schema_validation_accepts_scalars_arrays_enums_and_nested_objects() {
    let schema = json!({
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "count": {"type": "integer"},
            "ratio": {"type": "number"},
            "enabled": {"type": "boolean"},
            "nothing": {"type": "null"},
            "tags": {"type": "array", "items": {"type": "string", "enum": ["fast", "safe"]}},
            "nested": {
                "type": "object",
                "properties": {
                    "mode": {"type": "string", "enum": ["fast", "safe"]},
                    "values": {"type": "array", "items": {"type": "number"}}
                },
                "required": ["mode", "values"],
                "additionalProperties": false
            }
        },
        "required": ["name", "count", "ratio", "enabled", "nothing", "tags", "nested"],
        "additionalProperties": false
    });
    let registry =
        ToolRegistry::new([definition_with_schema("typed", "typed_tool", schema).unwrap()])
            .unwrap()
            .freeze();
    let valid = json!({
        "name": "fixture",
        "count": 3,
        "ratio": 3.5,
        "enabled": true,
        "nothing": null,
        "tags": ["fast", "safe"],
        "nested": {"mode": "safe", "values": [1, 2.5]}
    });
    let valid_bytes = serde_json::to_string(&valid).unwrap();
    let call = registry
        .validate_call("call-typed", "typed_tool", &valid_bytes, usize::MAX)
        .unwrap();
    assert_eq!(call.arguments(), &valid);

    let mut wrong_integer = valid.clone();
    wrong_integer["count"] = json!(3.5);
    let mut wrong_array_item = valid.clone();
    wrong_array_item["tags"][0] = json!(1);
    let mut wrong_enum = valid.clone();
    wrong_enum["nested"]["mode"] = json!("unknown");
    let mut missing_nested = valid.clone();
    missing_nested["nested"]
        .as_object_mut()
        .unwrap()
        .remove("mode");
    let mut extra_nested = valid.clone();
    extra_nested["nested"]["extra"] = json!(true);
    let mut extra_root = valid;
    extra_root["extra"] = json!(true);

    for (label, candidate) in [
        ("integer type", wrong_integer),
        ("array item type", wrong_array_item),
        ("enum value", wrong_enum),
        ("nested required property", missing_nested),
        ("nested closed property", extra_nested),
        ("root closed property", extra_root),
    ] {
        let bytes = serde_json::to_string(&candidate).unwrap();
        assert_eq!(
            registry
                .validate_call("call-typed", "typed_tool", &bytes, usize::MAX)
                .unwrap_err()
                .kind(),
            ToolValidationFailure::SchemaMismatch,
            "{label} should fail schema validation"
        );
    }
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

// 승인은 턴·호출·도구·정규 인자 바이트·효과·실행 호스트가 모두 같을 때만 재사용된다.
#[test]
fn approval_binding_is_exact() {
    let registry = ToolRegistry::new([definition("read-one", "read_path")])
        .unwrap()
        .freeze();
    let call = registry
        .validate_call("call-1", "read_path", r#"{"path":"a"}"#, 100)
        .unwrap();
    let changed = registry
        .validate_call("call-1", "read_path", r#"{"path":"b"}"#, 100)
        .unwrap();
    let equivalent = registry
        .validate_call("call-1", "read_path", r#"{ "path" : "a" }"#, 100)
        .unwrap();
    let binding = ToolApprovalBinding::new(turn(1), &call, "local-v1");

    assert!(binding.matches(turn(1), &call, "local-v1"));
    assert!(binding.matches(turn(1), &equivalent, "local-v1"));
    assert!(!binding.matches(turn(2), &call, "local-v1"));
    assert!(!binding.matches(turn(1), &changed, "local-v1"));
    assert!(!binding.matches(turn(1), &call, "other-host"));
}

// 정규화는 중첩 object key만 정렬하고 array 원소 순서는 보존하므로 approval digest도 같은 경계를
// 따른다.
#[test]
fn approval_binding_normalizes_recursive_objects_but_preserves_array_order() {
    let schema = json!({
        "type": "object",
        "properties": {
            "payload": {
                "type": "object",
                "properties": {
                    "items": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "a": {"type": "integer"},
                                "b": {"type": "integer"}
                            },
                            "required": ["a", "b"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["items"],
                "additionalProperties": false
            }
        },
        "required": ["payload"],
        "additionalProperties": false
    });
    let registry =
        ToolRegistry::new([definition_with_schema("normalize", "normalize_tool", schema).unwrap()])
            .unwrap()
            .freeze();
    let original = registry
        .validate_call(
            "call-normalize",
            "normalize_tool",
            r#"{"payload":{"items":[{"b":2,"a":1},{"a":3,"b":4}]}}"#,
            usize::MAX,
        )
        .unwrap();
    let reordered_keys = registry
        .validate_call(
            "call-normalize",
            "normalize_tool",
            r#"{"payload":{"items":[{"a":1,"b":2},{"b":4,"a":3}]}}"#,
            usize::MAX,
        )
        .unwrap();
    let reordered_array = registry
        .validate_call(
            "call-normalize",
            "normalize_tool",
            r#"{"payload":{"items":[{"b":4,"a":3},{"a":1,"b":2}]}}"#,
            usize::MAX,
        )
        .unwrap();
    let binding = ToolApprovalBinding::new(turn(1), &original, "local-v1");

    assert!(binding.matches(turn(1), &reordered_keys, "local-v1"));
    assert!(!binding.matches(turn(1), &reordered_array, "local-v1"));
    assert_eq!(
        original.argument_bytes(),
        r#"{"payload":{"items":[{"b":2,"a":1},{"a":3,"b":4}]}}"#
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

// yo.tool-schema/v1은 닫힌 subset이므로 모호하거나 무시될 keyword 조합을 registry 생성
// 시점에 모두 거절해 runtime 검증과 모델에 보낸 schema가 어긋나지 않게 한다.
#[test]
fn registry_rejects_malformed_closed_subset_schemas() {
    let malformed = [
        (
            "unsupported top-level schema keyword",
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "additionalProperties": false
            }),
        ),
        (
            "required name is undeclared",
            json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["missing"],
                "additionalProperties": false
            }),
        ),
        (
            "object omits additionalProperties false",
            json!({"type": "object", "properties": {}}),
        ),
        ("root schema type is not object", json!({"type": "array"})),
        (
            "root schema is a scalar with object keyword",
            json!({"type": "string", "properties": {}}),
        ),
        (
            "nested schema has an empty enum",
            json!({
                "type": "object",
                "properties": {"field": {"type": "string", "enum": []}},
                "additionalProperties": false
            }),
        ),
        (
            "nested schema enum has the wrong type",
            json!({
                "type": "object",
                "properties": {"field": {"type": "string", "enum": [1]}},
                "additionalProperties": false
            }),
        ),
        (
            "nested schema enum repeats a value",
            json!({
                "type": "object",
                "properties": {
                    "field": {"type": "string", "enum": ["same", "same"]}
                },
                "additionalProperties": false
            }),
        ),
        (
            "nested schema description is not a string",
            json!({
                "type": "object",
                "properties": {"field": {"type": "string", "description": 1}},
                "additionalProperties": false
            }),
        ),
        (
            "unsupported nested schema keyword",
            json!({
                "type": "object",
                "properties": {
                    "field": {"type": "string", "title": "unsupported"}
                },
                "additionalProperties": false
            }),
        ),
        (
            "object properties is not an object",
            json!({
                "type": "object",
                "properties": 1,
                "additionalProperties": false
            }),
        ),
        (
            "nested schema node has no type",
            json!({
                "type": "object",
                "properties": {"field": {}},
                "additionalProperties": false
            }),
        ),
        (
            "object required is not an array",
            json!({
                "type": "object",
                "properties": {"field": {"type": "string"}},
                "required": "field",
                "additionalProperties": false
            }),
        ),
        (
            "object required entry is not a string",
            json!({
                "type": "object",
                "properties": {"field": {"type": "string"}},
                "required": [1],
                "additionalProperties": false
            }),
        ),
        (
            "object required entries repeat a name",
            json!({
                "type": "object",
                "properties": {"field": {"type": "string"}},
                "required": ["field", "field"],
                "additionalProperties": false
            }),
        ),
        (
            "object allows additional properties",
            json!({
                "type": "object",
                "properties": {"field": {"type": "string"}},
                "additionalProperties": true
            }),
        ),
        (
            "object declares array items",
            json!({
                "type": "object",
                "properties": {"field": {"type": "string"}},
                "additionalProperties": false,
                "items": {"type": "string"}
            }),
        ),
        (
            "array declares object properties",
            json!({
                "type": "object",
                "properties": {"values": {"type": "array", "properties": {}}},
                "additionalProperties": false
            }),
        ),
        (
            "array items is not a schema node",
            json!({
                "type": "object",
                "properties": {"values": {"type": "array", "items": 1}},
                "additionalProperties": false
            }),
        ),
        (
            "scalar declares array items",
            json!({
                "type": "object",
                "properties": {
                    "field": {"type": "string", "items": {"type": "string"}}
                },
                "additionalProperties": false
            }),
        ),
        (
            "nested schema type is unsupported",
            json!({
                "type": "object",
                "properties": {"field": {"type": "date"}},
                "additionalProperties": false
            }),
        ),
        (
            "nested enum is not an array",
            json!({
                "type": "object",
                "properties": {"field": {"type": "string", "enum": 1}},
                "additionalProperties": false
            }),
        ),
        (
            "nested description is empty",
            json!({
                "type": "object",
                "properties": {"field": {"type": "string", "description": ""}},
                "additionalProperties": false
            }),
        ),
        (
            "nested description contains a control character",
            json!({
                "type": "object",
                "properties": {
                    "field": {"type": "string", "description": "bad\ntext"}
                },
                "additionalProperties": false
            }),
        ),
    ];

    for (index, (label, schema)) in malformed.into_iter().enumerate() {
        assert!(
            ToolDefinition::new(
                ToolId::new(format!("bad-schema-{index}")).unwrap(),
                format!("bad_schema_{index}"),
                "schema fixture",
                TOOL_SCHEMA_DIALECT,
                schema,
                ToolEffect::ReadOnly,
                ToolApprovalRequirement::Automatic,
            )
            .is_err(),
            "malformed schema {label} was accepted"
        );
    }
}
