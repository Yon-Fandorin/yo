use serde_json::{Value, json};

use super::{
    super::{
        MAX_DESCRIPTION_BYTES, MAX_SCHEMA_BYTES, TOOL_SCHEMA_DIALECT, ToolApprovalRequirement,
        ToolDefinition, ToolEffect, ToolId, ToolRegistry, ToolValidationFailure,
    },
    support::{basic_schema, definition_with_metadata, definition_with_schema},
};

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
