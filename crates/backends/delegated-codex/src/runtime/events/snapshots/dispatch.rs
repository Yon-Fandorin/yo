use serde_json::Value;
use yo_core::ActivityKind;

use super::{
    collaboration::{collab_tool_snapshot, sleep_snapshot, sub_agent_activity_snapshot},
    command::command_snapshot,
    context::{
        compaction_snapshot, hook_context_snapshot, proposed_plan_snapshot, reasoning_snapshot,
        review_snapshot,
    },
    file::file_change_snapshot,
    media::{image_generation_snapshot, image_view_snapshot},
    tool::{function_output_snapshot, tool_snapshot},
    web::web_search_snapshot,
};
pub(in crate::runtime::events) fn activity_kind(item_type: &str) -> Option<ActivityKind> {
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

pub(in crate::runtime::events) fn item_text_snapshot(params: &Value) -> Option<String> {
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
            // 호스트의 공개 요약만 대화에 포함합니다.
            // 별도의 원시 콘텐츠 필드는 요약이 없을 때의 대체값이 아닙니다.
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
