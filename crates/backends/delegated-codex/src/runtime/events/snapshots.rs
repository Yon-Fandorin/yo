use serde_json::{Map, Value, json, to_string_pretty};
use yo_core::{
    ActivityDocument, ActivityKind, ActivityNotice, ActivitySummary, BackendFailure, NoticeLevel,
    SummaryKind, ToolOutput,
};

use crate::protocol;

#[derive(Clone, Copy)]
pub(super) struct TokenUsageBreakdown {
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    reasoning_tokens: u64,
    cache_read_input_tokens: u64,
    cache_write_input_tokens: u64,
}

impl TokenUsageBreakdown {
    pub(super) fn to_json(self) -> Value {
        json!({
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
            "total_tokens": self.total_tokens,
            "reasoning_tokens": self.reasoning_tokens,
            "cache_read_input_tokens": self.cache_read_input_tokens,
            "cache_write_input_tokens": self.cache_write_input_tokens,
        })
    }
}

pub(super) fn token_usage_breakdown_at(
    value: &Value,
    field: &'static str,
) -> Result<TokenUsageBreakdown, BackendFailure> {
    let value = value_at(value, &[field], "token usage breakdown")?;
    Ok(TokenUsageBreakdown {
        input_tokens: non_negative_at(value, "inputTokens", "input tokens")?,
        output_tokens: non_negative_at(value, "outputTokens", "output tokens")?,
        total_tokens: non_negative_at(value, "totalTokens", "total tokens")?,
        reasoning_tokens: non_negative_at(
            value,
            "reasoningOutputTokens",
            "reasoning output tokens",
        )?,
        cache_read_input_tokens: non_negative_at(
            value,
            "cachedInputTokens",
            "cached input tokens",
        )?,
        cache_write_input_tokens: optional_non_negative_at(
            value,
            "cacheWriteInputTokens",
            "cache write input tokens",
        )?
        .unwrap_or(0),
    })
}

pub(super) fn value_at<'a>(
    value: &'a Value,
    path: &[&str],
    label: &'static str,
) -> Result<&'a Value, BackendFailure> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment).ok_or_else(|| {
            protocol::protocol_failure(format!("Codex message is missing {label}"))
        })?;
    }
    if !current.is_object() {
        return Err(protocol::protocol_failure(format!(
            "Codex {label} is not an object"
        )));
    }
    Ok(current)
}

fn non_negative_at(
    value: &Value,
    field: &'static str,
    label: &'static str,
) -> Result<u64, BackendFailure> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| protocol::protocol_failure(format!("Codex {label} is not non-negative")))
}

pub(super) fn optional_non_negative_at(
    value: &Value,
    field: &'static str,
    label: &'static str,
) -> Result<Option<u64>, BackendFailure> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| {
            protocol::protocol_failure(format!("Codex {label} is not non-negative"))
        }),
    }
}

pub(super) fn activity_kind(item_type: &str) -> Option<ActivityKind> {
    match item_type {
        "agentMessage" => Some(ActivityKind::AgentMessage),
        "functionCallOutput" => Some(ActivityKind::ToolResult),
        "reasoning" | "plan" | "contextCompaction" | "enteredReviewMode" | "exitedReviewMode"
        | "subAgentActivity" | "hookPrompt" => Some(ActivityKind::ModelWork),
        "commandExecution"
        | "mcpToolCall"
        | "dynamicToolCall"
        | "webSearch"
        | "imageGeneration"
        | "imageView"
        | "collabAgentToolCall"
        | "sleep" => Some(ActivityKind::ToolCall),
        "fileChange" => Some(ActivityKind::FileChange),
        _ => None,
    }
}

pub(super) fn item_text_snapshot(params: &Value) -> Option<String> {
    let item = params.get("item")?;
    match item.get("type")?.as_str()? {
        "agentMessage" => item.get("text")?.as_str().map(str::to_owned),
        "functionCallOutput" => Some(function_output_snapshot(item)),
        "plan" => proposed_plan_snapshot(item.get("text")?.as_str()?).ok(),
        "enteredReviewMode" => Some(review_snapshot(item, false)),
        "exitedReviewMode" => Some(review_snapshot(item, true)),
        "commandExecution" => command_snapshot(item),
        "imageGeneration" => Some(image_generation_snapshot(item)),
        "imageView" => Some(image_view_snapshot(item)),
        "collabAgentToolCall" => Some(collab_tool_snapshot(item)),
        "subAgentActivity" => Some(sub_agent_activity_snapshot(item)),
        "hookPrompt" => Some(hook_context_snapshot(item)),
        "sleep" => Some(sleep_snapshot(item)),
        "fileChange" => file_change_snapshot(item),
        "mcpToolCall" | "dynamicToolCall" => tool_snapshot(item),
        "reasoning" => {
            // Only the host's public summary belongs in the conversation.
            // The separate raw content field is not a fallback for absent summaries.
            let summary = item
                .get("summary")?
                .as_array()?
                .iter()
                .map(Value::as_str)
                .collect::<Option<Vec<_>>>()?
                .join("\n\n");
            reasoning_snapshot(summary)
        },
        "webSearch" => Some(web_search_snapshot(item)),
        "contextCompaction" => Some(compaction_snapshot(true)),
        _ => None,
    }
}

fn hook_context_snapshot(item: &Value) -> String {
    let fallback = || format!("Hook context\n{item:#}");
    let Some(fragments) = item.get("fragments").and_then(Value::as_array) else {
        return fallback();
    };
    let mut parts = Vec::new();
    for fragment in fragments {
        let (Some(id), Some(text)) = (
            fragment.get("hookRunId").and_then(Value::as_str),
            fragment.get("text").and_then(Value::as_str),
        ) else {
            return fallback();
        };
        let mut source = format!("Hook run: {id}\n{text}");
        let mut metadata = fragment.clone();
        if let Some(fields) = metadata.as_object_mut() {
            fields.remove("hookRunId");
            fields.remove("text");
            if !fields.is_empty() {
                source.push_str(&format!("\nMetadata: {metadata:#}"));
            }
        }
        parts.push(source);
    }
    if parts.is_empty() {
        parts.push("No hook context fragments reported".to_owned());
    }
    let mut metadata = item.clone();
    if let Some(fields) = metadata.as_object_mut() {
        for field in ["id", "type", "fragments"] {
            fields.remove(field);
        }
        if !fields.is_empty() {
            parts.push(format!("Metadata: {metadata:#}"));
        }
    }
    let source = parts.join("\n\n");
    let longest = source.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.saturating_add(1).max(3));
    ActivityDocument {
        title: "Hook context".to_owned(),
        markdown: format!("{fence}text\n{source}\n{fence}"),
    }
    .to_snapshot()
    .unwrap_or_else(fallback)
}

fn function_output_snapshot(item: &Value) -> String {
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

fn sub_agent_activity_snapshot(item: &Value) -> String {
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

fn sleep_snapshot(item: &Value) -> String {
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

fn collab_tool_snapshot(item: &Value) -> String {
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

fn image_view_snapshot(item: &Value) -> String {
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

fn image_generation_snapshot(item: &Value) -> String {
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
    // Keep the complete provider item for custom renderers; common image blocks
    // carry the PNG payload and never load savedPath from the client filesystem.
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

fn review_snapshot(item: &Value, ended: bool) -> String {
    let title = if ended {
        "Review ended"
    } else {
        "Review started"
    };
    let Some(markdown) = item.get("review").and_then(Value::as_str) else {
        return format!("{title}\n{item:#}");
    };
    ActivityDocument {
        title: title.to_owned(),
        markdown: markdown.to_owned(),
    }
    .to_snapshot()
    .unwrap_or_else(|| format!("{title}\n{item:#}"))
}

pub(super) fn proposed_plan_snapshot(markdown: &str) -> Result<String, BackendFailure> {
    ActivityDocument {
        title: "Proposed plan".to_owned(),
        markdown: markdown.to_owned(),
    }
    .to_snapshot()
    .ok_or_else(|| protocol::protocol_failure("proposed plan exceeds output profile limit"))
}

pub(super) fn reasoning_snapshot(summary: String) -> Option<String> {
    ActivitySummary {
        kind: SummaryKind::Reasoning,
        summary,
        tokens_before: None,
    }
    .to_snapshot()
}

pub(super) fn compaction_snapshot(completed: bool) -> String {
    ActivityNotice {
        title: if completed {
            "Context compacted"
        } else {
            "Compacting context"
        }
        .to_owned(),
        message: if completed {
            "Codex compacted the conversation context."
        } else {
            "Codex is compacting the conversation context."
        }
        .to_owned(),
        level: NoticeLevel::Info,
    }
    .to_snapshot()
    .expect("bounded static compaction notice")
}

fn web_search_snapshot(item: &Value) -> String {
    let mut plain_text = web_search_text(item);
    // The app-server deliberately leaves result entries opaque. Preserve new
    // result kinds and fields without inventing a universal citation schema.
    let result = item
        .get("results")
        .filter(|value| !value.is_null())
        .map(|results| {
            plain_text.push_str(&format!("\nResults:\n{results:#}"));
            json!({"results":results})
        });
    let arguments = ["query", "action"]
        .into_iter()
        .filter_map(|key| item.get(key).map(|value| (key.to_owned(), value.clone())))
        .collect::<Map<_, _>>();
    ToolOutput {
        tool: "webSearch".to_owned(),
        server: None,
        arguments: Some(Value::Object(arguments)),
        result,
        content_items: None,
        error: None,
        plain_text: plain_text.clone(),
    }
    .to_snapshot()
    .unwrap_or(plain_text)
}

fn web_search_text(item: &Value) -> String {
    let action = item.get("action").filter(|value| !value.is_null());
    let action_type = action
        .and_then(|action| action.get("type"))
        .and_then(Value::as_str);
    let mut lines = vec![
        match action_type {
            Some("openPage") => "Open web page",
            Some("findInPage") => "Find in web page",
            Some("search") => "Web search",
            None if action.is_none() => "Web search",
            _ => "Web search · other action",
        }
        .to_owned(),
    ];
    let mut queries = Vec::new();
    if let Some(query) = action
        .and_then(|action| action.get("query"))
        .and_then(Value::as_str)
        .filter(|query| !query.is_empty())
    {
        queries.push(query);
    }
    if let Some(values) = action
        .and_then(|action| action.get("queries"))
        .and_then(Value::as_array)
    {
        for query in values
            .iter()
            .filter_map(Value::as_str)
            .filter(|query| !query.is_empty())
        {
            if !queries.contains(&query) {
                queries.push(query);
            }
        }
    }
    lines.extend(queries.into_iter().map(|query| format!("Query: {query}")));
    for (field, label) in [("url", "URL"), ("pattern", "Find")] {
        if let Some(value) = action
            .and_then(|action| action.get(field))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            lines.push(format!("{label}: {value}"));
        }
    }
    // Codex keeps an item-level query when no usable action detail is available.
    if lines.len() == 1
        && let Some(query) = item
            .get("query")
            .and_then(Value::as_str)
            .filter(|query| !query.is_empty())
    {
        lines.push(format!("Query: {query}"));
    }
    if let Some(action) = action {
        let known_fields: &[&str] = match action_type {
            Some("search") => &["type", "query", "queries"],
            Some("openPage") => &["type", "url"],
            Some("findInPage") => &["type", "url", "pattern"],
            _ => &[],
        };
        let mut extra = action.clone();
        if let Some(fields) = extra.as_object_mut() {
            for field in known_fields {
                let valid = match *field {
                    "queries" => fields.get(*field).is_some_and(|value| {
                        value.is_null()
                            || value
                                .as_array()
                                .is_some_and(|values| values.iter().all(Value::is_string))
                    }),
                    _ => fields
                        .get(*field)
                        .is_some_and(|value| value.is_null() || value.is_string()),
                };
                if valid {
                    fields.remove(*field);
                }
            }
        }
        if !extra.as_object().is_some_and(Map::is_empty) {
            lines.push(to_string_pretty(&extra).expect("JSON is serializable"));
        }
    }
    if lines.len() == 1 {
        lines.push("Details not reported".to_owned());
    }
    lines.join("\n")
}

pub(super) fn checked_command_snapshot(item: &Value) -> Result<String, BackendFailure> {
    command_snapshot(item)
        .ok_or_else(|| protocol::protocol_failure("command output exceeds output profile limit"))
}

fn command_snapshot(item: &Value) -> Option<String> {
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

pub(super) fn command_plain_text(item: &Value) -> Option<String> {
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

pub(super) fn file_change_snapshot(item: &Value) -> Option<String> {
    let changes = item.get("changes")?.as_array()?;
    let lines = changes
        .iter()
        .filter_map(|change| {
            let path = change.get("path")?.as_str()?;
            let kind = change.get("kind");
            let name = kind
                .and_then(|kind| kind.as_str().or_else(|| kind.get("type")?.as_str()))
                .unwrap_or("update");
            let mut text = format!("{name}: {path}");
            if let Some(destination) = kind
                .and_then(|kind| kind.get("move_path"))
                .and_then(Value::as_str)
            {
                text.push_str(&format!(" -> {destination}"));
            }
            if let Some(diff) = change
                .get("diff")
                .and_then(Value::as_str)
                .filter(|diff| !diff.is_empty())
            {
                text.push('\n');
                match name {
                    "add" | "delete" => {
                        // App-server sends whole file content for add/delete;
                        // only update carries a unified diff.
                        let prefix = if name == "add" { '+' } else { '-' };
                        for line in diff.split_inclusive('\n') {
                            text.push(prefix);
                            text.push_str(line);
                        }
                        if !diff.ends_with('\n') {
                            text.push_str("\n\\ No newline at end of file");
                        }
                    },
                    _ => text.push_str(diff),
                }
            }
            Some(text)
        })
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn tool_snapshot(item: &Value) -> Option<String> {
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
    // Keep structured output and extension metadata independently of human text.
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
            // Text resources are readable; binary and unfamiliar shapes retain their
            // complete JSON until a typed media handoff is available.
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
