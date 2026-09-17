use serde_json::{Value, json, to_string_pretty};
use yo_core::ToolOutput;

pub(super) fn function_output_snapshot(item: &Value) -> String {
    let Some(name) = item
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
    else {
        return format!("Function output\n{item:#}");
    };
    let tool = item
        .get("namespace")
        .and_then(Value::as_str)
        .filter(|namespace| !namespace.is_empty())
        .map_or_else(
            || name.to_owned(),
            |namespace| format!("{namespace}.{name}"),
        );
    let mut metadata = item.clone();
    let mut heading = "Function output".to_owned();
    if let Some(fields) = metadata.as_object_mut() {
        for field in ["id", "type", "name", "namespace", "output"] {
            fields.remove(field);
        }
        if !fields.is_empty() {
            heading.push_str(&format!("\nMetadata: {metadata:#}"));
        }
    }
    let mut content = vec![json!({"type":"text","text":heading,"source":item})];
    let plain = match item.get("output") {
        Some(Value::String(text)) => {
            content.push(
                json!({"type":"text","text":if text.is_empty() { "(empty output)" } else { text }}),
            );
            text.clone()
        },
        Some(Value::Array(items)) => {
            content.extend(items.iter().map(function_output_block));
            if items.is_empty() {
                content.push(json!({"type":"text","text":"(empty output)"}));
            }
            format!("{}", Value::Array(items.clone()))
        },
        Some(value) => {
            content.push(json!({"type":"text","text":format!("{value:#}")}));
            format!("{value:#}")
        },
        None => {
            content.push(json!({"type":"text","text":"No output value reported"}));
            "No output value reported".to_owned()
        },
    };
    let plain_text = format!("{tool}\n{heading}\n{plain}");
    ToolOutput {
        tool,
        server: None,
        arguments: None,
        result: None,
        content_items: Some(Value::Array(content)),
        error: None,
        plain_text,
    }
    .to_snapshot()
    .unwrap_or_else(|| format!("Function output\n{item:#}"))
}

fn function_output_block(block: &Value) -> Value {
    let mut normalized = block.clone();
    let Some(fields) = normalized.as_object_mut() else {
        return normalized;
    };
    match fields.get("type").and_then(Value::as_str) {
        Some("input_text") if fields.get("text").is_some_and(Value::is_string) => {
            fields.insert("type".to_owned(), json!("text"));
        },
        Some("input_image" | "input_audio") => {
            let image = fields["type"] == "input_image";
            let (old, new, kind) = if image {
                ("image_url", "imageUrl", "inputImage")
            } else {
                ("audio_url", "audioUrl", "inputAudio")
            };
            if fields.get(old).is_some_and(Value::is_string) && !fields.contains_key(new) {
                let url = fields.remove(old).expect("validated URL remains present");
                fields.insert(new.to_owned(), url);
                fields.insert("type".to_owned(), json!(kind));
            }
        },
        _ => {},
    }
    normalized
}

pub(super) fn tool_snapshot(item: &Value) -> Option<String> {
    let tool = item.get("tool")?.as_str()?;
    let mut sections = vec![match item.get("server").and_then(Value::as_str) {
        Some(server) => format!("{server}.{tool}"),
        None => tool.to_owned(),
    }];
    for (field, label) in [
        ("arguments", "Arguments"),
        ("result", "Result"),
        ("contentItems", "Result"),
        ("error", "Error"),
    ] {
        if let Some(value) = item.get(field).filter(|value| !value.is_null()) {
            let text = match field {
                "result" => mcp_result_text(value),
                "contentItems" => tool_content_text(value),
                _ => None,
            }
            .unwrap_or_else(|| json_or_text(value));
            sections.push(format!("{label}:\n{text}"));
        }
    }
    let output = ToolOutput {
        tool: tool.to_owned(),
        server: item
            .get("server")
            .and_then(Value::as_str)
            .map(str::to_owned),
        arguments: item
            .get("arguments")
            .filter(|value| !value.is_null())
            .cloned(),
        result: item.get("result").filter(|value| !value.is_null()).cloned(),
        content_items: item
            .get("contentItems")
            .filter(|value| !value.is_null())
            .cloned(),
        error: item.get("error").filter(|value| !value.is_null()).cloned(),
        plain_text: sections.join("\n"),
    };
    Some(output.to_snapshot().unwrap_or(output.plain_text))
}

fn json_or_text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| to_string_pretty(value).expect("JSON values are serializable"))
}

fn mcp_result_text(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    let mut text = tool_content_text(object.get("content")?)?;
    // 구조화된 출력과 확장 메타데이터를 사람이 읽는 텍스트와 독립적으로 유지합니다.
    for (key, value) in object.iter().filter(|(key, _)| *key != "content") {
        text.push_str(&format!("\n{key}:\n{}", json_or_text(value)));
    }
    Some(text)
}

fn tool_content_text(value: &Value) -> Option<String> {
    let blocks = value.as_array()?;
    if blocks.is_empty() {
        return Some("(empty content)".to_owned());
    }
    Some(
        blocks
            .iter()
            .map(|block| tool_content_block(block).unwrap_or_else(|| json_or_text(block)))
            .collect::<Vec<_>>()
            .join("\n\n"),
    )
}

fn tool_content_block(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    let kind = object.get("type")?.as_str()?;
    let (mut text, consumed): (String, &[&str]) = match kind {
        "text" | "inputText" => (object.get("text")?.as_str()?.to_owned(), &["type", "text"]),
        "image" | "audio" => {
            let data = object.get("data")?.as_str()?;
            let mime = object.get("mimeType")?.as_str()?;
            let label = if kind == "image" { "Image" } else { "Audio" };
            (
                format!(
                    "{label} · {mime}\nEmbedded data: {} encoded bytes",
                    data.len()
                ),
                &["type", "data", "mimeType"],
            )
        },
        "inputImage" | "inputAudio" => {
            let field = if kind == "inputImage" {
                "imageUrl"
            } else {
                "audioUrl"
            };
            let url = object.get(field)?.as_str()?;
            let label = if kind == "inputImage" {
                "Image"
            } else {
                "Audio"
            };
            let text = if let Some((mime, data)) = url
                .strip_prefix("data:")
                .and_then(|url| url.split_once(";base64,"))
            {
                format!(
                    "{label} · {mime}\nEmbedded data: {} encoded bytes",
                    data.len()
                )
            } else {
                format!("{label} URL: {url}")
            };
            (
                text,
                if kind == "inputImage" {
                    &["type", "imageUrl"]
                } else {
                    &["type", "audioUrl"]
                },
            )
        },
        "resource_link" => {
            let uri = object.get("uri")?.as_str()?;
            let name = object
                .get("title")
                .or_else(|| object.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("Resource");
            (format!("Resource · {name}\nURI: {uri}"), &["type", "uri"])
        },
        "resource" => {
            let resource = object.get("resource")?.as_object()?;
            let uri = resource.get("uri")?.as_str()?;
            // 텍스트 리소스는 읽을 수 있지만, 이진 데이터와 알 수 없는 형태는
            // 형식이 지정된 미디어 전달이 가능해질 때까지 전체 JSON을 유지합니다.
            let body = resource.get("text")?.as_str()?;
            let mut text = format!("Resource\nURI: {uri}\n{body}");
            for (key, value) in resource
                .iter()
                .filter(|(key, _)| !["uri", "text"].contains(&key.as_str()))
            {
                text.push_str(&format!("\n{key}: {}", json_or_text(value)));
            }
            (text, &["type", "resource"])
        },
        _ => return None,
    };
    for (key, value) in object
        .iter()
        .filter(|(key, _)| !consumed.contains(&key.as_str()))
    {
        text.push_str(&format!("\n{key}: {}", json_or_text(value)));
    }
    Some(text)
}
