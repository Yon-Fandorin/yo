use serde_json::{Value, json};

use super::*;
use crate::state::config::{Config, parse};

fn command() -> Value {
    json!({
        "id": "project-check",
        "name": "project_check",
        "description": "Check the selected project component.",
        "executable": "/missing/interpreter",
        "parameters": {"type": "object", "properties": {}, "additionalProperties": false}
    })
}

fn parse_commands(commands: Vec<Value>) -> Result<Config, ConfigError> {
    parse(
        Path::new("config.yaml"),
        &json!({"tools": {"commands": commands}}).to_string(),
    )
}

// 설정 미지정·빈 배열은 기존 registry 선택을 보존하며 command 정의를 만들지 않는다.
#[test]
fn absent_or_empty_commands_preserve_configuration_defaults() {
    assert!(Config::default().command_tools().is_empty());
    for text in ["{}", "tools: {}", "tools: {commands: []}"] {
        assert!(
            parse(Path::new("config.yaml"), text)
                .unwrap()
                .command_tools()
                .is_empty()
        );
    }
}

// YAML backend의 null→빈 container 기본 동작을 새 tools 설정에는 적용하지 않는다.
#[test]
fn command_containers_reject_explicit_and_implicit_yaml_nulls() {
    for null in ["null", "NULL", "~", ""] {
        for document in [
            format!("tools: {null}\n"),
            format!("tools:\n  commands: {null}\n"),
        ] {
            let error = parse(Path::new("config.yaml"), &document).unwrap_err();
            assert!(matches!(error, ConfigError::InvalidTools { .. }));
        }
    }
    assert!(parse_commands(vec![Value::Null]).is_err());
    for field in ["executable_args", "argv"] {
        let mut value = command();
        value[field] = json!([null]);
        assert!(parse_commands(vec![value]).is_err(), "null {field} entry");
    }
}

// 존재하지 않는 interpreter도 구조 검증만 통과하며 literal 인자·순서·고정 승인 정책을 보존한다.
#[test]
fn command_config_preserves_literal_launch_values_without_reading_artifacts() {
    let mut first = command();
    first["script"] = json!("scripts/../tools/check.py");
    first["executable_args"] = json!(["-I", ""]);
    first["argv"] = json!(["$(never-expanded)", "a\nb", "한"]);
    let mut second = command();
    second["id"] = json!("native");
    second["name"] = json!("native_tool");
    let config = parse_commands(vec![first, second]).unwrap();
    let tools = config.command_tools();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].definition().id().as_str(), "project-check");
    assert_eq!(tools[0].executable(), "/missing/interpreter");
    assert_eq!(tools[0].script(), Some("scripts/../tools/check.py"));
    assert_eq!(tools[0].executable_args(), ["-I", ""]);
    assert_eq!(tools[0].argv(), ["$(never-expanded)", "a\nb", "한"]);
    assert_eq!(tools[0].definition().effect(), ToolEffect::Process);
    assert_eq!(
        tools[0].definition().approval(),
        ToolApprovalRequirement::Required
    );
    assert_eq!(
        tools[0].definition().argument_byte_limit(),
        Some(4 * 1024 * 1024)
    );
    assert_eq!(tools[1].definition().id().as_str(), "native");
    assert_eq!(tools[1].script(), None);
    assert!(tools[1].executable_args().is_empty());
    assert!(tools[1].argv().is_empty());
}

// 설정 envelope와 각 필드는 null·unknown·duplicate를 거절하지만 enum 내부 null은 유지한다.
#[test]
fn command_config_rejects_closed_field_violations_and_preserves_enum_null() {
    for text in [
        "tools: null",
        "tools: {commands: null}",
        "tools: {other: []}",
        "tools: {commands: [], commands: []}",
        "tools: {}\ntools: {}",
    ] {
        assert!(parse(Path::new("config.yaml"), text).is_err(), "{text}");
    }
    for field in [
        "id",
        "name",
        "description",
        "executable",
        "parameters",
        "script",
        "executable_args",
        "argv",
    ] {
        let mut value = command();
        value[field] = Value::Null;
        assert!(parse_commands(vec![value]).is_err(), "null {field}");
    }
    for field in ["effect", "approval", "environment", "cwd", "unknown"] {
        let mut value = command();
        value[field] = json!("not admitted");
        assert!(parse_commands(vec![value]).is_err(), "unknown {field}");
    }
    for field in ["id", "name", "description", "executable", "parameters"] {
        let mut value = command();
        value.as_object_mut().unwrap().remove(field);
        assert!(parse_commands(vec![value]).is_err(), "missing {field}");
    }
    let duplicate_command = command()
        .to_string()
        .replacen('{', "{\"id\":\"duplicate\",", 1);
    let text = format!(r#"{{"tools":{{"commands":[{duplicate_command}]}}}}"#);
    assert!(parse(Path::new("config.yaml"), &text).is_err());
    let mut value = command();
    value["parameters"] = json!({"type":"object", "properties":{"choice":{"type":"null","enum":[null]}}, "additionalProperties":false});
    let config = parse_commands(vec![value]).unwrap();
    assert_eq!(
        config.command_tools()[0].definition().input_schema()["properties"]["choice"]["enum"][0],
        Value::Null
    );
    let duplicate_parameter = command().to_string().replace(
        "\"type\":\"object\"",
        "\"type\":\"object\",\"type\":\"object\"",
    );
    let text = format!(r#"{{"tools":{{"commands":[{duplicate_parameter}]}}}}"#);
    assert!(parse(Path::new("config.yaml"), &text).is_err());
}

// command 개수의 첫 초과와 custom 간·BasicFiles 간 ID/name 충돌을 모두 거절한다.
#[test]
fn command_count_and_identity_collisions_are_rejected() {
    let mut commands = (0..16)
        .map(|index| {
            let mut value = command();
            value["id"] = json!(format!("command-{index}"));
            value["name"] = json!(format!("command_{index}"));
            value
        })
        .collect::<Vec<_>>();
    assert_eq!(
        parse_commands(commands.clone())
            .unwrap()
            .command_tools()
            .len(),
        16
    );
    commands.push(command());
    assert!(parse_commands(commands).is_err());
    for (id, name) in [
        ("list-files", "list_files"),
        ("read-files", "read_files"),
        ("edit-file", "edit_file"),
        ("write-file", "write_file"),
        ("run-command", "run_command"),
    ] {
        for (field, reserved) in [("id", id), ("name", name)] {
            let mut value = command();
            value[field] = json!(reserved);
            assert!(parse_commands(vec![value]).is_err(), "{field} {reserved}");
        }
    }
    for field in ["id", "name"] {
        let first = command();
        let mut second = command();
        second["id"] = json!("different-id");
        second["name"] = json!("different_name");
        second[field] = first[field].clone();
        assert!(parse_commands(vec![first, second]).is_err());
    }
}

// locator의 UTF-8 bytes·control·절대 executable·workspace 상대경계만 검사하며 파일은 열지 않는다.
#[test]
fn command_locator_bounds_preserve_original_paths_and_reject_escapes() {
    for field in ["executable", "script"] {
        let mut value = command();
        value[field] = json!(format!("/{}", "한".repeat(1365)));
        assert_eq!(value[field].as_str().unwrap().len(), 4096);
        assert!(parse_commands(vec![value.clone()]).is_ok());
        value[field] = json!(format!("{}a", value[field].as_str().unwrap()));
        assert!(parse_commands(vec![value]).is_err());
        for locator in ["", "/bad\npath", "/bad\0path", "/bad\u{85}path"] {
            let mut value = command();
            value[field] = json!(locator);
            let error = parse_commands(vec![value]).unwrap_err();
            assert!(matches!(error, ConfigError::InvalidTools { .. }));
            assert!(!error.to_string().contains(locator) || locator.is_empty());
        }
    }
    let mut relative_executable = command();
    relative_executable["executable"] = json!("bin/interpreter");
    assert!(parse_commands(vec![relative_executable]).is_err());
    for script in ["../outside", "a/../../outside"] {
        let mut value = command();
        value["script"] = json!(script);
        assert!(parse_commands(vec![value]).is_err());
    }
    for script in ["/outside/script", "./tools/script", "a/../script"] {
        let mut value = command();
        value["script"] = json!(script);
        assert_eq!(
            parse_commands(vec![value]).unwrap().command_tools()[0].script(),
            Some(script)
        );
    }
}

// 두 fixed argv 배열의 합산 개수·bytes와 각 원소의 첫 초과를 검사하고 NUL만 금지한다.
#[test]
fn fixed_arguments_share_entry_and_byte_budgets() {
    let mut value = command();
    value["executable_args"] = json!(vec![""; 16]);
    value["argv"] = json!(vec![""; 16]);
    assert!(parse_commands(vec![value.clone()]).is_ok());
    value["argv"].as_array_mut().unwrap().push(json!(""));
    assert!(parse_commands(vec![value]).is_err());
    let entry = "한".repeat(1365) + "a";
    assert_eq!(entry.len(), 4096);
    let mut value = command();
    value["executable_args"] = json!([entry.clone(), entry.clone()]);
    value["argv"] = json!([entry.clone(), entry.clone()]);
    assert!(parse_commands(vec![value.clone()]).is_ok());
    value["argv"].as_array_mut().unwrap().push(json!("a"));
    assert!(parse_commands(vec![value]).is_err());
    for field in ["executable_args", "argv"] {
        for entry in [entry.clone() + "a", "private-argument\0suffix".to_owned()] {
            let mut value = command();
            value[field] = json!([entry]);
            let error = parse_commands(vec![value]).unwrap_err();
            assert!(matches!(error, ConfigError::InvalidTools { .. }));
            assert!(!error.to_string().contains("private-argument"));
        }
    }
}

// 기존 ToolDefinition의 ID·description·schema byte 한도를 config에서도 동일하게 적용한다.
#[test]
fn command_definitions_reuse_exact_tool_metadata_and_schema_limits() {
    for (field, limit) in [("id", 128), ("name", 128), ("description", 4096)] {
        let mut value = command();
        value[field] = json!("a".repeat(limit));
        assert!(parse_commands(vec![value.clone()]).is_ok(), "{field}");
        value[field] = json!("a".repeat(limit + 1));
        assert!(parse_commands(vec![value]).is_err(), "{field}");
    }
    let mut properties = serde_json::Map::new();
    for index in 0..16 {
        properties.insert(
            format!("field_{index}"),
            json!({"type":"string","description":"a".repeat(3900)}),
        );
    }
    let mut parameters = json!({"type":"object","properties":properties,"description":"", "additionalProperties":false});
    let framing = serde_json::to_vec(&parameters).unwrap().len();
    parameters["description"] = json!("a".repeat(64 * 1024 - framing));
    let mut value = command();
    value["parameters"] = parameters.clone();
    assert!(parse_commands(vec![value.clone()]).is_ok());
    parameters["description"] = json!("a".repeat(64 * 1024 - framing + 1));
    value["parameters"] = parameters;
    assert!(parse_commands(vec![value]).is_err());
    for parameters in [
        json!({"type":"array"}),
        json!({"type":"object","unknown":true}),
    ] {
        let mut value = command();
        value["parameters"] = parameters;
        assert!(parse_commands(vec![value]).is_err());
    }
}

// tools의 타입·unknown·null·깨진 YAML 오류에서도 실행 경로와 argv 원문을 노출하지 않는다.
#[test]
fn tools_decode_diagnostics_do_not_echo_launch_values_or_source_snippets() {
    const MARKER: &str = "private-launch-marker-7f32";
    for field in ["executable", "script", "executable_args", "argv"] {
        for replacement in [Value::Null, json!({"wrong_type": MARKER})] {
            let mut value = command();
            value["executable"] = json!(format!("/{MARKER}/interpreter"));
            value["script"] = json!(format!("{MARKER}/script"));
            value["argv"] = json!([MARKER]);
            value[field] = replacement;
            let error = parse_commands(vec![value]).unwrap_err();
            assert!(matches!(error, ConfigError::InvalidTools { .. }));
            assert!(!error.to_string().contains(MARKER));
        }
    }
    let mut value = command();
    value["executable"] = json!(format!("/{MARKER}/interpreter"));
    value["unknown"] = json!(MARKER);
    let error = parse_commands(vec![value]).unwrap_err();
    assert!(matches!(error, ConfigError::InvalidTools { .. }));
    assert!(!error.to_string().contains(MARKER));
    let malformed = format!(
        "tools:\n  commands:\n    - executable: /{MARKER}/interpreter\n      argv: [\"{MARKER}\", {{]\n"
    );
    let error = parse(Path::new("config.yaml"), &malformed).unwrap_err();
    assert!(matches!(error, ConfigError::InvalidTools { .. }));
    assert!(!error.to_string().contains(MARKER));
}

// tools 밖의 오류나 중복 root key도 같은 source snippet에 담긴 실행 정의를 노출하지 않는다.
#[test]
fn root_yaml_failures_do_not_echo_command_launch_values() {
    const MARKER: &str = "private-root-launch-marker-5a91";
    let mut value = command();
    value["executable"] = json!(format!("/{MARKER}/interpreter"));
    value["script"] = json!(format!("{MARKER}/script"));
    value["executable_args"] = json!([MARKER]);
    value["argv"] = json!([MARKER]);
    let tools = json!({"commands": [value]});
    let documents = [
        format!(r#"{{"tools":{tools},"tools":{{}}}}"#),
        format!("tools: {tools}\nother: [broken, {{]\n"),
        format!("other: [broken, {{]\ntools: {tools}\n"),
        format!(r#"{{"unknown":true,"tools":{tools}}}"#),
        format!(r#"{{"tools":{tools},"tui":{{"unknown":true}}}}"#),
    ];
    for document in documents {
        let error = parse(Path::new("config.yaml"), &document).unwrap_err();
        assert!(!error.to_string().contains(MARKER));
        if let ConfigError::InvalidYaml { source, .. } = error {
            assert!(!source.to_string().contains(MARKER));
        }
    }
}

// tools가 없다고 구조적으로 확인된 문서의 기존 unknown-field 상세 진단은 유지한다.
#[test]
fn unrelated_configuration_type_errors_keep_their_existing_diagnostics() {
    for document in [
        "model: {}",
        "tui: {unknown: true}",
        "session: {unknown: true}",
    ] {
        let error = parse(Path::new("config.yaml"), document).unwrap_err();
        assert!(matches!(error, ConfigError::InvalidYaml { .. }));
        assert!(error.to_string().contains("unknown field"));
    }
}
