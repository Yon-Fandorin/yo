use std::num::NonZeroU64;

use serde_json::json;

use super::{
    super::{ToolApprovalBinding, ToolRegistry},
    support::{definition, definition_with_schema},
};
use crate::{TurnId, TurnRef, fixture_session};

fn turn(value: u64) -> TurnRef {
    TurnRef::new(
        fixture_session(1),
        TurnId::new(NonZeroU64::new(value).unwrap()),
    )
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
