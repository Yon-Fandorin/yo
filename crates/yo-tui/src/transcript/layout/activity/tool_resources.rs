use serde_json::{Map, Value};
use yo_core::ToolOutput;

use super::{
    media::image_markdown,
    tool_files::{file_language, literal_block},
    tool_shell::shell_details_markdown,
};

// This is a presentation shape, not provider detection or execution authority. Partial or
// unfamiliar metadata keeps the generic renderer so no original field disappears.
pub(super) fn execution_details(result: &Value) -> Option<String> {
    let text = |key| result.get(key)?.as_str().filter(|value| !value.is_empty());
    let call = text("call_id")?;
    let tool = text("tool_id")?;
    let host = text("execution_host")?;
    let outcome = text("outcome")?;
    let failed = result.get("isError")?.as_bool()?;
    result.get("truncated")?.as_bool()?;
    if !matches!(outcome, "completed" | "failed" | "interrupted")
        || failed != (outcome != "completed")
    {
        return None;
    }
    Some(format!(
        "Status: {outcome}\nTool: {tool}\nCall: {call}\nHost: {host}\nError: {failed}"
    ))
}

pub(super) fn embedded_resource_markdown(block: &Value, successful: bool) -> Option<String> {
    let fields = block.as_object()?;
    let resource = fields.get("resource")?.as_object()?;
    let uri = resource.get("uri")?.as_str()?;
    if uri.is_empty() {
        return None;
    }
    let mime = match resource.get("mimeType") {
        Some(value) => Some(value.as_str()?),
        None => None,
    };
    let mut heading = format!("Resource · {uri}");
    if let Some(mime) = mime {
        heading.push_str(&format!("\nType: {mime}"));
    }
    let body = match (resource.get("text"), resource.get("blob")) {
        (Some(text), None) => literal_block(
            if successful {
                file_language(uri)
            } else {
                "text"
            },
            text.as_str()?,
        ),
        (None, Some(blob)) => {
            let blob = blob.as_str()?;
            if let Some(mime @ ("image/png" | "image/jpeg")) = mime {
                image_markdown(mime, blob)
            } else {
                literal_block("text", "Binary resource (retained in output record)")
            }
        },
        _ => return None,
    };
    let mut metadata = fields
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "type" | "resource"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Map<_, _>>();
    let resource_metadata = resource
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "uri" | "mimeType" | "text" | "blob"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Map<_, _>>();
    if !resource_metadata.is_empty() {
        metadata.insert("resource".to_owned(), Value::Object(resource_metadata));
    }
    let mut sections = vec![literal_block("text", &heading), body];
    if !metadata.is_empty() {
        sections.push(format!(
            "**Resource metadata**\n\n{}",
            literal_block("json", &format!("{:#}", Value::Object(metadata)))
        ));
    }
    Some(sections.join("\n\n"))
}

pub(super) fn resource_link_markdown(block: &Value) -> Option<String> {
    if block.to_string().len() > 256 * 1024 {
        return None;
    }
    let fields = block.as_object()?;
    let name = fields.get("name")?.as_str()?;
    let uri = fields.get("uri")?.as_str()?;
    if name.is_empty() || uri.is_empty() {
        return None;
    }
    let mut metadata = fields.clone();
    for key in ["type", "name", "uri"] {
        metadata.remove(key);
    }
    let mut labels = vec![format!("Name: {name}"), format!("URI: {uri}")];
    for (key, label) in [("title", "Title"), ("mimeType", "Type")] {
        if let Some(value) = fields.get(key) {
            labels.push(format!("{label}: {}", value.as_str()?));
            metadata.remove(key);
        }
    }
    if let Some(size) = fields.get("size") {
        labels.push(format!("Size: {} bytes", size.as_u64()?));
        metadata.remove("size");
    }
    let mut sections = vec![format!(
        "**Resource link**\n\n{}",
        literal_block("text", &labels.join("\n"))
    )];
    if let Some(description) = fields.get("description") {
        sections.push(format!(
            "**Description**\n\n{}",
            literal_block("text", description.as_str()?)
        ));
        metadata.remove("description");
    }
    if !metadata.is_empty() {
        sections.push(format!(
            "**Resource metadata**\n\n{}",
            literal_block("json", &format!("{:#}", Value::Object(metadata)))
        ));
    }
    Some(sections.join("\n\n"))
}

pub(super) fn search_details_markdown(
    details: &Value,
    output: &ToolOutput,
) -> Option<(String, bool)> {
    if details.to_string().len() > 256 * 1024 {
        return None;
    }
    let fields = details.as_object()?;
    let is_grep = output.tool == "grep";
    let limit_key = if is_grep {
        "matchLimitReached"
    } else {
        "resultLimitReached"
    };
    if fields.is_empty()
        || fields.keys().any(|key| {
            key != limit_key && key != "truncation" && !(is_grep && key == "linesTruncated")
        })
    {
        return None;
    }
    let mut sections = Vec::new();
    let mut truncated = false;
    if let Some(limit) = fields.get(limit_key) {
        let limit = limit.as_u64().filter(|limit| *limit > 0)?;
        let (heading, label) = if is_grep {
            ("Partial content search", "Match limit reached")
        } else {
            ("Partial file search", "Result limit reached")
        };
        sections.push(format!("**{heading}**\n\n{label}: {limit}"));
        truncated = true;
    }
    if let Some(lines) = fields.get("linesTruncated")
        && lines.as_bool()?
    {
        sections.push(
            "**Partial search lines**\n\nSome matching or context lines were truncated.".to_owned(),
        );
        truncated = true;
    }
    if let Some(truncation) = fields.get("truncation") {
        let details = Value::Object(
            [("truncation".to_owned(), truncation.clone())]
                .into_iter()
                .collect(),
        );
        let (markdown, partial) = shell_details_markdown(&details, output)?;
        sections.push(markdown);
        truncated |= partial;
    }
    Some((sections.join("\n\n"), truncated))
}

pub(super) fn directory_markdown(source: &str, truncated: bool) -> Option<String> {
    if source.len() > 256 * 1024 {
        return None;
    }
    let source = if truncated {
        source
            .strip_suffix("\n[yo: tool output truncated]")
            .unwrap_or(source)
    } else {
        source
    };
    if !source.is_empty() && !source.ends_with('\n') {
        return None;
    }
    let entries = source.split_terminator('\n').collect::<Vec<_>>();
    if entries.len() > 1024
        || entries.iter().any(|entry| {
            entry.is_empty() || entry.len() > 1024 || entry.chars().any(char::is_control)
        })
    {
        return None;
    }
    let directories = entries.iter().filter(|entry| entry.ends_with('/')).count();
    let mut body = format!(
        "**{} {} shown · {} {}**",
        entries.len(),
        if entries.len() == 1 {
            "entry"
        } else {
            "entries"
        },
        directories,
        if directories == 1 {
            "directory"
        } else {
            "directories"
        }
    );
    if entries.is_empty() {
        body.push_str("\n\n");
        body.push_str(if truncated {
            "No complete entries were returned."
        } else {
            "No entries were returned."
        });
    } else {
        let rows = entries
            .iter()
            .map(|entry| {
                let kind = if entry.ends_with('/') { "dir " } else { "file" };
                format!("{kind}  {entry}")
            })
            .collect::<Vec<_>>()
            .join("\n");
        body.push_str(&format!("\n\n{}", literal_block("text", &rows)));
    }
    if truncated {
        body.push_str("\n\n**Partial listing · output was truncated**");
    }
    Some(body)
}
