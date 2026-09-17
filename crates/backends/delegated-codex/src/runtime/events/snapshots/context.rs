use serde_json::Value;
use yo_core::{
    ActivityDocument, ActivityNotice, ActivitySummary, BackendFailure, NoticeLevel, SummaryKind,
};

use crate::protocol;

pub(super) fn hook_context_snapshot(item: &Value) -> String {
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

pub(super) fn review_snapshot(item: &Value, ended: bool) -> String {
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

pub(in crate::runtime::events) fn proposed_plan_snapshot(
    markdown: &str,
) -> Result<String, BackendFailure> {
    ActivityDocument {
        title: "Proposed plan".to_owned(),
        markdown: markdown.to_owned(),
    }
    .to_snapshot()
    .ok_or_else(|| protocol::protocol_failure("proposed plan exceeds output profile limit"))
}

pub(in crate::runtime::events) fn reasoning_snapshot(summary: String) -> Option<String> {
    ActivitySummary {
        kind: SummaryKind::Reasoning,
        summary,
        tokens_before: None,
    }
    .to_snapshot()
}

pub(in crate::runtime::events) fn compaction_snapshot(completed: bool) -> String {
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
