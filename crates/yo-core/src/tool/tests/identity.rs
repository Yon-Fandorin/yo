use super::{
    super::{MAX_ID_BYTES, ToolId},
    support::{basic_schema, definition_with_schema},
};

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
