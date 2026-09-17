use serde_json::{Map, Value, json};
use yo_core::{BackendFailure, ToolOutput};

use crate::protocol;

pub(in crate::runtime::events) fn checked_command_snapshot(
    item: &Value,
) -> Result<String, BackendFailure> {
    command_snapshot(item)
        .ok_or_else(|| protocol::protocol_failure("command output exceeds output profile limit"))
}

pub(super) fn command_snapshot(item: &Value) -> Option<String> {
    let mut arguments = Map::new();
    let mut result = Map::new();
    for field in ["command", "cwd"] {
        if let Some(value) = item.get(field) {
            arguments.insert(field.to_owned(), value.clone());
        }
    }
    for field in ["exitCode", "durationMs", "status"] {
        if let Some(value) = item.get(field) {
            result.insert(field.to_owned(), value.clone());
        }
    }
    if let Some(output) = item
        .get("aggregatedOutput")
        .filter(|value| !value.is_null())
    {
        if let Some(text) = output.as_str() {
            result.insert(
                "content".to_owned(),
                json!([{"type": "text", "text": text}]),
            );
        } else {
            result.insert("aggregatedOutput".to_owned(), output.clone());
        }
    }
    ToolOutput {
        tool: "commandExecution".to_owned(),
        server: None,
        arguments: (!arguments.is_empty()).then_some(Value::Object(arguments)),
        result: (!result.is_empty()).then_some(Value::Object(result)),
        content_items: None,
        error: None,
        plain_text: command_plain_text(item).unwrap_or_else(|| "Command execution".to_owned()),
    }
    .to_snapshot()
}

pub(in crate::runtime::events) fn command_plain_text(item: &Value) -> Option<String> {
    let command = item.get("command").and_then(Value::as_str);
    let output = item.get("aggregatedOutput").and_then(Value::as_str);
    let mut lines = Vec::new();
    if let Some(command) = command {
        lines.push(format!("$ {command}"));
    }
    if let Some(cwd) = item.get("cwd").and_then(Value::as_str) {
        lines.push(format!("Directory: {cwd}"));
    }
    if let Some(output) = output.filter(|output| !output.is_empty()) {
        lines.push(output.to_owned());
    }
    let mut result = Vec::new();
    if let Some(code) = item.get("exitCode").and_then(Value::as_i64) {
        result.push(format!("Exit: {code}"));
    }
    if let Some(duration) = item.get("durationMs").and_then(Value::as_u64) {
        result.push(format!("Duration: {duration} ms"));
    }
    if !result.is_empty() {
        lines.push(result.join(" · "));
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}
