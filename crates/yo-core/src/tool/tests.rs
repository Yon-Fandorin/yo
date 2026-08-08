use std::num::NonZeroU64;

use serde_json::json;

use super::*;
use crate::{TurnId, fixture_session};

fn definition(id: &str, name: &str) -> ToolDefinition {
    ToolDefinition::new(
        ToolId::new(id).unwrap(),
        name,
        "reads one path",
        TOOL_SCHEMA_DIALECT,
        json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"],
            "additionalProperties": false
        }),
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

// yo.tool-schema/v1은 닫힌 subset이므로 모호하거나 무시될 keyword 조합을 registry 생성
// 시점에 모두 거절해 runtime 검증과 모델에 보낸 schema가 어긋나지 않게 한다.
#[test]
fn registry_rejects_malformed_closed_subset_schemas() {
    let malformed = [
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "additionalProperties": false
        }),
        json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["missing"],
            "additionalProperties": false
        }),
        json!({"type": "object", "properties": {}}),
        json!({"type": "array"}),
        json!({"type": "string", "properties": {}}),
        json!({"type": "string", "enum": []}),
        json!({"type": "string", "enum": [1]}),
        json!({"type": "string", "enum": ["same", "same"]}),
        json!({"type": "string", "description": 1}),
    ];

    for schema in malformed {
        assert!(
            ToolDefinition::new(
                ToolId::new("bad-schema").unwrap(),
                "bad_schema",
                "schema fixture",
                TOOL_SCHEMA_DIALECT,
                schema,
                ToolEffect::ReadOnly,
                ToolApprovalRequirement::Automatic,
            )
            .is_err()
        );
    }
}
