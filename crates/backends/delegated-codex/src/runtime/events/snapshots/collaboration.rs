use serde_json::{Value, json};
use yo_core::{ActivityNotice, NoticeLevel, ToolOutput};

pub(super) fn sub_agent_activity_snapshot(item: &Value) -> String {
    let Some(kind) = item.get("kind").and_then(Value::as_str) else {
        return format!("Agent activity\n{item:#}");
    };
    let mut lines = Vec::new();
    for (field, label) in [
        ("agentPath", "Agent path"),
        ("agentThreadId", "Agent thread"),
    ] {
        if let Some(value) = item.get(field) {
            lines.push(format!(
                "{label}: {}",
                value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{value:#}"))
            ));
        }
    }
    let mut metadata = item.clone();
    if let Some(fields) = metadata.as_object_mut() {
        for field in ["id", "type", "kind", "agentPath", "agentThreadId"] {
            fields.remove(field);
        }
        if !fields.is_empty() {
            lines.push(format!("Metadata: {metadata:#}"));
        }
    }
    ActivityNotice {
        title: format!("Agent activity · {kind}"),
        message: lines.join("\n"),
        level: NoticeLevel::Info,
    }
    .to_snapshot()
    .unwrap_or_else(|| format!("Agent activity\n{item:#}"))
}

pub(super) fn sleep_snapshot(item: &Value) -> String {
    let Some(duration) = item.get("durationMs").and_then(Value::as_u64) else {
        return format!("Wait\n{item:#}");
    };
    let mut plain_text = format!("Wait\nRequested duration: {duration} ms");
    let mut metadata = item.clone();
    if let Some(fields) = metadata.as_object_mut() {
        for field in ["id", "type", "durationMs"] {
            fields.remove(field);
        }
        if !fields.is_empty() {
            plain_text.push_str(&format!("\nMetadata: {metadata:#}"));
        }
    }
    ToolOutput {
        tool: "clock.sleep".to_owned(),
        server: None,
        arguments: Some(json!({"durationMs":duration})),
        result: None,
        content_items: Some(json!([{"type":"text","text":plain_text,"source":item}])),
        error: None,
        plain_text,
    }
    .to_snapshot()
    .unwrap_or_else(|| format!("Wait\n{item:#}"))
}

pub(super) fn collab_tool_snapshot(item: &Value) -> String {
    let Some(tool) = item
        .get("tool")
        .and_then(Value::as_str)
        .filter(|tool| !tool.is_empty())
    else {
        return format!("Agent task\n{item:#}");
    };
    let mut parts = vec![format!("Agent task · {tool}")];
    let mut metadata = item.clone();
    for (field, label) in [
        ("status", "Tool status"),
        ("senderThreadId", "Sender"),
        ("receiverThreadIds", "Recipients"),
        ("agentsStates", "Reported agent states"),
        ("model", "Requested model"),
        ("reasoningEffort", "Requested reasoning effort"),
        ("prompt", "Prompt"),
    ] {
        if let Some(value) = item.get(field).filter(|value| !value.is_null()) {
            let text = if field == "agentsStates" {
                collab_agent_states(value)
            } else {
                value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{value:#}"))
            };
            parts.push(format!("{label}: {text}"));
        }
        if let Some(fields) = metadata.as_object_mut() {
            fields.remove(field);
        }
    }
    if let Some(fields) = metadata.as_object_mut() {
        for field in ["id", "type", "tool"] {
            fields.remove(field);
        }
        if !fields.is_empty() {
            parts.push(format!("Metadata: {metadata:#}"));
        }
    }
    let plain_text = parts.join("\n");
    ToolOutput {
        tool: tool.to_owned(),
        server: None,
        arguments: None,
        result: None,
        content_items: Some(json!([{"type":"text","text":plain_text,"source":item}])),
        error: None,
        plain_text,
    }
    .to_snapshot()
    .unwrap_or_else(|| format!("Agent task\n{item:#}"))
}

fn collab_agent_states(value: &Value) -> String {
    let Some(states) = value.as_object() else {
        return format!("{value:#}");
    };
    if states.is_empty() {
        return "(none reported)".to_owned();
    }
    states
        .iter()
        .map(|(id, state)| {
            let Some(status) = state.get("status").and_then(Value::as_str) else {
                return format!("Agent {id}\n{state:#}");
            };
            let mut text = format!("Agent {id} · {status}");
            let mut metadata = state.clone();
            if let Some(fields) = metadata.as_object_mut() {
                fields.remove("status");
                match fields.get("message") {
                    Some(Value::String(message)) => {
                        if !message.is_empty() {
                            text.push_str(&format!("\n{message}"));
                        }
                        fields.remove("message");
                    },
                    Some(Value::Null) => {
                        fields.remove("message");
                    },
                    _ => {},
                }
                if !fields.is_empty() {
                    text.push_str(&format!("\n{metadata:#}"));
                }
            }
            text
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
