use serde_json::{Value, json};
use yo_core::ToolOutput;

pub(super) fn image_view_snapshot(item: &Value) -> String {
    let Some(path) = item.get("path").and_then(Value::as_str) else {
        return format!("Image view\n{item:#}");
    };
    let mut plain_text =
        format!("Image view\nPath: {path}\nImage bytes are not included in this event.");
    let mut metadata = item.clone();
    if let Some(fields) = metadata.as_object_mut() {
        for field in ["id", "type", "path"] {
            fields.remove(field);
        }
        if !fields.is_empty() {
            plain_text.push_str(&format!("\nMetadata: {metadata:#}"));
        }
    }
    ToolOutput {
        tool: "view_image".to_owned(),
        server: None,
        arguments: Some(json!({"path":path})),
        result: None,
        content_items: Some(json!([{"type":"text","text":plain_text,"source":item}])),
        error: None,
        plain_text,
    }
    .to_snapshot()
    .unwrap_or_else(|| format!("Image view\n{item:#}"))
}

pub(super) fn image_generation_snapshot(item: &Value) -> String {
    let mut details = Vec::new();
    for (field, label) in [
        ("status", "Status"),
        ("revisedPrompt", "Prompt"),
        ("savedPath", "Saved path"),
    ] {
        if let Some(text) = item
            .get(field)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
        {
            details.push(format!("{label}: {text}"));
        }
    }
    if let Some(failure) = item.get("failure").filter(|value| !value.is_null()) {
        details.push(format!("Failure: {failure:#}"));
    }
    let mut metadata = item.clone();
    if let Some(fields) = metadata.as_object_mut() {
        for field in [
            "type",
            "id",
            "status",
            "revisedPrompt",
            "savedPath",
            "failure",
            "result",
        ] {
            fields.remove(field);
        }
        if !fields.is_empty() {
            details.push(format!("Metadata: {metadata:#}"));
        }
    }
    let data = item
        .get("result")
        .and_then(Value::as_str)
        .filter(|data| !data.is_empty());
    details.push(if data.is_some() {
        "Generated PNG image (original payload retained)".to_owned()
    } else {
        "No image payload received".to_owned()
    });
    let plain_text = format!("Image generation\n{}", details.join("\n"));
    // 사용자 지정 렌더러를 위해 전체 provider 항목을 유지합니다. 공통 이미지 블록은
    // PNG 페이로드를 전달하며 클라이언트 파일 시스템에서 savedPath를 읽지 않습니다.
    let mut content = vec![json!({"type":"text","text":plain_text,"source":item})];
    if let Some(data) = data {
        content.push(json!({"type":"image","mimeType":"image/png","data":data}));
    }
    ToolOutput {
        tool: "image_generation".to_owned(),
        server: None,
        arguments: None,
        result: None,
        content_items: Some(Value::Array(content)),
        error: item
            .get("failure")
            .filter(|value| !value.is_null())
            .cloned(),
        plain_text,
    }
    .to_snapshot()
    .unwrap_or_else(|| format!("Image generation\n{item:#}"))
}
