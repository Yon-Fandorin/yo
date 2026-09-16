use serde_json::json;
use yo_core::{
    ActivityApproval, ActivityDocument, ActivityKind, ActivityNotice, ActivityPlan,
    ActivityQuestion, ActivityReasoning, ActivitySummary, ApprovalChoice, MessageContent,
    NoticeLevel, PlanStep, PlanStepStatus, QuestionChoice, SummaryKind, ToolOutput,
};

use super::media::{sample_batch_read, sample_image_data, sample_tool_image};

pub(super) fn approval_profile() -> ActivityApproval {
    ActivityApproval {
        related_change: None,
        plain_text: "Command: cargo test\nWorking directory: /workspace/yo\nOffline preview: no command, permission or policy change.\n\nSession scope: future matching prompts in this session.\nPersistent command rule: prefix [\"cargo\",\"test\"].\nPersistent network rules: host example.com.".to_owned(),
        choices: [
            ("Approve request", "Use the scope described above"),
            ("Approve for session", "Future matching prompts in this session may run without asking again"),
            ("Approve + save rule", "Persistent rule for commands starting with cargo test"),
            ("Always allow host", "Persistent network allow rule for example.com"),
            ("Always deny host", "Persistent network deny rule for example.com"),
            ("Decline", "Do not run this action; continue the turn"),
        ].into_iter().map(|(label, description)| ApprovalChoice { label:label.to_owned(), description:description.to_owned(), enabled:true }).collect(),
        decline_choice: Some(6),
    }
}

pub(super) fn interview_prompt(first: bool) -> String {
    let (plain_text, choices) = if first {
        (
            "Question 1 of 2 · Priority\n\nWhat should the chat make easiest?\n1. Reading code and explanations\n2. Reviewing changes and approvals\n3. None of the above\n\nChoose an option or describe your preference.",
            [
                ("Reading code and explanations", "Readable code and answers"),
                ("Reviewing changes and approvals", "Inspect proposed edits"),
            ],
        )
    } else {
        (
            "Question 2 of 2 · Density\n\nHow much detail should stay visible?\n1. Compact summaries, expand when needed\n2. More detail in the conversation\n\nEnter 1, 2, or write your own answer.",
            [
                (
                    "Compact summaries, expand when needed",
                    "Keep the conversation short",
                ),
                (
                    "More detail in the conversation",
                    "Show more output by default",
                ),
            ],
        )
    };
    let mut choices = choices
        .into_iter()
        .map(|(label, description)| QuestionChoice {
            label: label.to_owned(),
            description: description.to_owned(),
        })
        .collect::<Vec<_>>();
    if first {
        choices.push(QuestionChoice {
            label: "None of the above".to_owned(),
            description: "Choose an answer outside this list.".to_owned(),
        });
    }
    ActivityQuestion {
        allow_notes: true,
        previous_question: false,
        draft: None,
        draft_choice: None,
        plain_text: plain_text.to_owned(),
        choices,
    }
    .to_snapshot()
    .expect("bounded interview fixture")
}

pub(super) fn response(input: &str) -> (ActivityKind, String) {
    match input.trim() {
        "tools" => (ActivityKind::ToolCall, "[SIMULATED] cargo test\nChecking layout and Unicode wrapping...\nAll preview checks passed. No process or filesystem operation was performed.".into()),
        "long-tools" => (ActivityKind::ToolCall, (1..=30).map(|n| format!("[SIMULATED] Check {n:02}: passed.\n")).collect()),
        "error" => (ActivityKind::ToolCall, "[SIMULATED] Checking a missing fixture...\nThis request deliberately fails so you can inspect error presentation.".into()),
        "tables" => (ActivityKind::AgentMessage, "| Component | Status | Tests |\n| :--- | :---: | ---: |\n| Rendering | **ready** | 32 |\n| 한글 wrapping | ready | 12 |\n| Theme | `mono` | 8 |".into()),
        "file-links" => (ActivityKind::AgentMessage, "## Host-confirmed file link\n\n[README.md](README.md)\n\nREADME.md points to a file on the yo execution host. Opening it requires a compatible terminal file handler.\n\n[Unmapped file](missing.rs) and [raw file URL](file:///tmp/untrusted) stay plain.\n\n| Kind | Link |\n| --- | --- |\n| Registered file | [README.md](README.md) |\n| Web | [Documentation](https://example.com/docs) |\n\n```text\n[README.md](README.md)\n```\n\nOver SSH, this file URL does not download or open the remote file inside yo. Web links are independent. This preview does not open files.".into()),
        "usage" => (ActivityKind::ModelWork, r#"{"schema":"codex.app-server-token-usage-receipt/v1","source_profile":"codex.app-server.thread-token-usage-updated/v1","turn_id":"preview-only","model_context_window":200000,"usage":{"input_tokens":12500,"output_tokens":1800,"total_tokens":14300,"reasoning_tokens":600,"cache_read_input_tokens":9800,"cache_write_input_tokens":0}}"#.into()),
        "agent-tasks" => (ActivityKind::ToolCall, ToolOutput {
            tool: "wait".to_owned(), server: None, arguments: None, result: None, error: None,
            content_items: Some(json!([{"type":"text","text":"Agent task · wait\nTool status: completed\nReported agent states:\nAgent reviewer · completed\nReview finished.\n\nAgent tests · running\nChecks are still running.\n\nAgent docs · errored\nSource document unavailable.\n\nOffline example: no agents were started.","source":{"tool":"wait","status":"completed","agentsStates":{"reviewer":{"status":"completed","message":"Review finished."},"tests":{"status":"running","message":"Checks are still running."},"docs":{"status":"errored","message":"Source document unavailable."}}}}])),
            plain_text: "Agent task · wait\nTool status: completed\nAgent reviewer · completed\nAgent tests · running\nAgent docs · errored\nOffline example: no agents were started.".to_owned(),
        }.to_snapshot().expect("bounded agent task fixture")),
        "mcp" | "mcp-failure" => (ActivityKind::ToolCall, "docs.search\nArguments:\n  query: transcript wrapping\nResult:\nFound 3 matching pages.\nPreview only: no external tool was called.\n\nResource · Rendering guide\nURI: memory://docs/rendering\nNarrow output wraps within the configured body width.\n\nstructuredContent:\n{\"matches\": 3}".into()),
        "embedded-resource" => (ActivityKind::ToolCall, ToolOutput {
            tool:"lookup".to_owned(),server:Some("offline".to_owned()),arguments:None,
            result:Some(json!({"content":[{"type":"resource","resource":{"uri":"resource://preview/main.rs","mimeType":"text/x-rust","text":"use std::path::Path;\n\nfn main() {\n    let path = Path::new(\"demo\");\n}","_meta":{"revision":7}},"annotations":{"audience":["user"]}},{"type":"resource","resource":{"uri":"resource://preview/image","mimeType":"image/png","blob":sample_image_data()}}]})),
            content_items:None,error:None,plain_text:"Offline embedded resources: Rust source, metadata and PNG bytes. Nothing was fetched.".to_owned(),
        }.to_snapshot().expect("bounded embedded resource fixture")),
        "resource-link" => (ActivityKind::ToolCall, ToolOutput {
            tool:"resources".to_owned(), server:Some("offline".to_owned()), arguments:None,
            result:Some(json!({"content":[{"type":"resource_link","name":"build-report.txt","title":"Build report","uri":"resource://preview/build-report","mimeType":"text/plain","size":2048,"description":"Offline resource reference. Nothing was fetched.","annotations":{"audience":["user"]}}]})),
            content_items:None,error:None,plain_text:"Offline resource link: build-report.txt, resource://preview/build-report, text/plain, 2048 bytes. Nothing was fetched.".to_owned(),
        }.to_snapshot().expect("bounded resource link fixture")),
        "content-search" => (ActivityKind::ToolCall, ToolOutput {
            tool: "grep".to_owned(), server: Some("offline".to_owned()),
            arguments: Some(json!({"pattern":"render","path":"src","glob":"*.rs"})),
            result: Some(json!({"content":[{"type":"text","text":"view.rs:12: fn render() {\nview.rs-13-     // surrounding context"}],"details":{"matchLimitReached":1,"linesTruncated":true}})),
            content_items: None, error: None,
            plain_text: "Offline content search: no search was executed.\nview.rs:12: fn render() {\nview.rs-13-     // surrounding context\nMatch limit reached: 1; some lines truncated.".to_owned(),
        }.to_snapshot().expect("bounded content search fixture")),
        "files-find" => (ActivityKind::ToolCall, ToolOutput {
            tool: "find".to_owned(), server: Some("offline".to_owned()),
            arguments: Some(json!({"pattern":"**/*.rs","path":"src","limit":2})),
            result: Some(json!({"content":[{"type":"text","text":"src/main.rs\nsrc/lib.rs"}],"details":{"resultLimitReached":2}})),
            content_items: None, error: None,
            plain_text: "Offline file search: no filesystem search was executed.\nsrc/main.rs\nsrc/lib.rs\nResult limit reached: 2".to_owned(),
        }.to_snapshot().expect("bounded file search fixture")),
        "files-list" => (ActivityKind::ToolCall, ToolOutput {
            tool:"list_files".to_owned(), server:Some("preview".to_owned()),
            arguments:Some(json!({"path":"."})),
            result:Some(json!({"content":[{"type":"text","text":"Cargo.toml\nREADME.md\nsrc/\ntests/\n\n[yo: tool output truncated]"}],"truncated":true,"note":"Offline fixture: no directory was read."})),
            content_items:None, error:None,
            plain_text:"preview.list_files\nCargo.toml\nREADME.md\nsrc/\ntests/\n\n[yo: tool output truncated]\nOffline fixture: no directory was read.".to_owned(),
        }.to_snapshot().expect("bounded directory fixture")),
        "shell-retained" => {
            let retained = (1..=120).map(|row| format!("Captured row {row:03}: original command output")).collect::<Vec<_>>().join("\n");
            let preview = "status: 0\nstdout:\nCaptured row 001\n[yo: bytes omitted from model result]\nCaptured row 120\nstderr:\n";
            (ActivityKind::ToolCall, ToolOutput {
                tool: "run_command".to_owned(), server: None,
                arguments: Some(json!({"command":"offline retained-output example"})),
                result: Some(json!({"content":[{"type":"text","text":preview}],"truncated":true,"retainedOutput":{"truncated":false}})),
                content_items: None, error: None,
                plain_text: format!("Offline fixture: no command was executed.\n{retained}"),
            }.to_snapshot().expect("bounded retained output fixture"))
        },
        "shell-truncated" => {
            let text = "Last reported output line.\nOffline fixture: no output file was created.";
            (ActivityKind::ToolCall, ToolOutput {
                tool:"bash".to_owned(),server:None,
                arguments:Some(json!({"command":"cargo test --workspace"})),
                result:Some(json!({"content":[{"type":"text","text":text}],"truncated":true,"details":{"fullOutputPath":"offline-preview/build-output.log","truncation":{"content":text,"truncated":true,"truncatedBy":"lines","totalLines":100,"totalBytes":9000,"outputLines":2,"outputBytes":text.len(),"lastLinePartial":false,"firstLineExceedsLimit":false,"maxLines":2,"maxBytes":51200}}})),
                content_items:None,error:None,plain_text:format!("{text}\nReported full output file: offline-preview/build-output.log"),
            }.to_snapshot().expect("bounded truncation fixture"))
        },
        "shell-tail" => {
            let text = format!("{}\nLatest result: preview checks passed.\nOffline fixture: no command was executed.", (1..=20).map(|n|format!("test preview_case_{n:02} ... ok")).collect::<Vec<_>>().join("\n"));
            (ActivityKind::ToolCall, ToolOutput {
                tool:"commandExecution".to_owned(),server:None,
                arguments:Some(json!({"command":"cargo test --workspace","cwd":"/workspace/yo"})),
                result:Some(json!({"content":[{"type":"text","text":text}],"status":"completed","exitCode":0,"durationMs":1240})),
                content_items:None,error:None,plain_text:text,
            }.to_snapshot().expect("bounded shell tail fixture"))
        },
        "codex-shell" => (ActivityKind::ToolCall, ToolOutput {
            tool:"commandExecution".to_owned(), server:None,
            arguments:Some(json!({"command":"if true; then\n  cargo test --workspace\nfi","cwd":"/workspace/yo"})),
            result:Some(json!({"content":[{"type":"text","text":"Offline fixture: no command was executed.\nCombined process output stays literal."}],"status":"completed","exitCode":0,"durationMs":1240})),
            content_items:None, error:None,
            plain_text:"cargo test --workspace\nOffline fixture: no command was executed.\nExit: 0 · Duration: 1240 ms".to_owned(),
        }.to_snapshot().expect("bounded Codex command fixture")),
        "shell" => (ActivityKind::ToolCall, ToolOutput {
            tool:"run_command".to_owned(), server:Some("preview".to_owned()),
            arguments:Some(json!({"command":"cargo test --workspace"})),
            result:Some(json!({"content":[{"type":"text","text":"status: 0\nstdout:\nOffline fixture: no command was executed.\nAll preview checks passed.\nstderr:\n"}]})),
            content_items:None, error:None,
            plain_text:"preview.run_command\ncargo test --workspace\nstatus: 0\nstdout:\nOffline fixture: no command was executed.\nAll preview checks passed.\nstderr:\n".to_owned(),
        }.to_snapshot().expect("bounded shell fixture")),
        "file-edit" => (ActivityKind::ToolCall, ToolOutput {
            tool:"edit_file".to_owned(), server:Some("preview".to_owned()),
            arguments:Some(json!({"path":"src/greeting.rs","edits":[
                {"oldText":"    \"Hello\"\n","newText":"    \"Hello from Yo\"\n"},
                {"oldText":"// obsolete note","newText":""}
            ]})),
            result:Some(json!({"content":[{"type":"text","text":"Offline fixture: no replacements were applied."}]})),
            content_items:None, error:None,
            plain_text:"preview.edit_file\nsrc/greeting.rs\nProposed replacements: Hello -> Hello from Yo; remove obsolete note.\nOffline fixture: no replacements were applied.".to_owned(),
        }.to_snapshot().expect("bounded edit fixture")),
        "files-read" => (ActivityKind::ToolCall, sample_batch_read()),
        "file-write" => (ActivityKind::ToolCall, ToolOutput {
            tool:"write_file".to_owned(), server:Some("preview".to_owned()),
            arguments:Some(r#"{"path":"src/greeting.rs","content":"pub fn greeting() -> &'static str {\n    \"Hello from Yo\"\n}"}"#.parse().expect("fixture write arguments")),
            result:Some(r#"{"content":[{"type":"text","text":"Offline fixture: no file was written."}]}"#.parse().expect("fixture write result")),
            content_items:None, error:None,
            plain_text:"preview.write_file\nsrc/greeting.rs\npub fn greeting() -> &'static str {\n    \"Hello from Yo\"\n}\nOffline fixture: no file was written.".to_owned(),
        }.to_snapshot().expect("bounded write fixture")),
        "file-read" => (ActivityKind::ToolCall, ToolOutput {
            tool:"read".to_owned(), server:Some("preview".to_owned()),
            arguments:Some(r#"{"path":"src/main.rs","offset":1,"limit":5}"#.parse().expect("fixture arguments")),
            result:Some(r#"{"content":[{"type":"text","text":"use std::fmt;\n\nfn main() {\n    println!(\"offline file preview\");\n}"}],"note":"Offline fixture: no file was read."}"#.parse().expect("fixture result")),
            content_items:None, error:None,
            plain_text:"preview.read\nsrc/main.rs\nuse std::fmt;\n\nfn main() {\n    println!(\"offline file preview\");\n}\nOffline fixture: no file was read.".to_owned(),
        }.to_snapshot().expect("bounded file fixture")),
        "tool-diff" => {
            let patch = "--- settings.rs\n+++ settings.rs\n@@ -1,3 +1,3 @@\n fn settings() {\n-    let theme = \"fixed\";\n+    let theme = \"selected\";\n }\n";
            (ActivityKind::ToolResult, ToolOutput {
                tool: "workspace_patch".to_owned(), server: None, arguments: None,
                result: None, error: None,
                content_items: Some(json!([{
                    "type":"diff", "title":"File change · settings.rs", "text":patch,
                    "source":{"path":"settings.rs", "oldText":"fn settings() {\n    let theme = \"fixed\";\n}\n", "newText":"fn settings() {\n    let theme = \"selected\";\n}\n"}
                }])),
                plain_text: format!("Offline fixture: no file was changed.\n{patch}"),
            }.to_snapshot().expect("bounded generic diff fixture"))
        },
        "mcp-image" => (ActivityKind::ToolCall, sample_tool_image()),
        "summary" | "branch" => (ActivityKind::ModelWork, ActivitySummary {
            kind: if input == "branch" { SummaryKind::Branch } else { SummaryKind::Compaction },
            tokens_before: (input == "summary").then_some(24000),
            summary: "## Offline summary example\n\nThis is fixture content, not a summary of your session.\n\n### Decisions\n\n- Keep source text in the journal.\n- Wrap code and tables to the available width.\n- Preserve approval ownership.\n- Show actual tool outcomes.\n- Keep images behind explicit media output.\n- Apply the selected palette.\n- Retain the complete summary when collapsed.\n- Keep interruption details visible.\n\n### Next step\n\n```rust\nuse std::fmt;\n```\n\nReview the implementation and run the affected checks.".to_owned(),
        }.to_snapshot().expect("bounded preview summary")),
        "compaction" => (ActivityKind::ModelWork, ActivityNotice { title:"Context compacted".to_owned(), message:"Preview only: conversation context compacted.\nNo summary or token counts were reported.".to_owned(), level:NoticeLevel::Info }.to_snapshot().expect("bounded preview notice")),
        "turn-diff" => (ActivityKind::FileChange, "Turn aggregate diff\ndiff --git a/src/main.rs b/src/main.rs\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1 @@\n-fn main() {}\n+fn main() { println!(\"ready\"); }\ndiff --git a/README.md b/README.md\n--- a/README.md\n+++ b/README.md\n@@ -1 +1 @@\n-Old instructions\n+Updated instructions\n".to_owned()),
        "reroute" => (ActivityKind::ModelWork, ActivityNotice {title:"Model rerouted".to_owned(), message:"Preview only: provider-reported model change.\nFrom: requested-model\nTo: reported-model\nReason: highRiskCyberActivity\nNo provider call or model selection change occurred.".to_owned(),level:NoticeLevel::Warning}.to_snapshot().expect("bounded preview notice")),
        "retry" => (ActivityKind::ModelWork, ActivityNotice {title:"Retry announced".to_owned(), message:"Preview only: temporary connection interruption.\nThe provider reported that it will retry.".to_owned(),level:NoticeLevel::Warning}.to_snapshot().expect("bounded preview notice")),
        "diagrams" => (ActivityKind::AgentMessage, "## Diagrams\n\n```mermaid\ngraph LR; A[Request] --> B[Review] --> C[Apply]\n```\n\n```mermaid\nsequenceDiagram\nUser->>Agent: Inspect changes\nAgent-->>User: Review result\n```\n\nNarrow views keep the Mermaid source instead of clipping connections.".to_owned()),
        "agent-reasoning" => (ActivityKind::ModelWork, ActivityReasoning { content: json!("Offline reasoning fixture.\n\nChecking code, table and diff rendering at narrow widths.") }.to_snapshot().expect("bounded reasoning fixture")),
        "reasoning" => (ActivityKind::ModelWork, ActivitySummary {kind: SummaryKind::Reasoning, summary:"Inspecting the output components\n\nChecking narrow widths and configurable folding.".to_owned(), tokens_before:None}.to_snapshot().expect("bounded preview summary")),
        "search" => (ActivityKind::ToolCall, ToolOutput {
            tool: "webSearch".to_owned(), server: None,
            arguments: Some(json!({"query":"terminal output components", "action":{"type":"search", "queries":["terminal output components", "configurable rendering"]}})),
            result: None, content_items: None, error: None,
            plain_text: "Web search\nQuery: terminal output components\nQuery: configurable rendering".to_owned(),
        }.to_snapshot().expect("bounded search preview")),
        "proposed-plan" => (ActivityKind::ModelWork,ActivityDocument{title:"Proposed plan".to_owned(),markdown:"## Implementation approach\n\nOffline fixture: this document does not start a task or approve an action.\n\n1. Inspect the existing renderer.\n2. Preserve authoritative source through streaming.\n3. Reflow tables and code after terminal resize.\n4. Validate empty and interrupted states.\n\n```rust\nuse std::fmt;\n```\n\n| Check | Expected |\n| --- | --- |\n| Resize | Reflow |\n| Source | Preserved |".to_owned()}.to_snapshot().expect("bounded preview document")),
        "turn-duration" => (ActivityKind::ModelWork, ActivityNotice { title:"Turn completed".to_owned(), message:"Duration: 1m 2.345s (62345 ms, reported by Codex)\nOffline fixture: illustrative server duration.".to_owned(), level:NoticeLevel::Info }.to_snapshot().expect("bounded duration preview")),
        "terminal-wait" => (ActivityKind::ModelWork,ActivityDocument{title:"Waited for background terminal".to_owned(),markdown:"```text\nProcess: preview-7\nCommand: cargo test\n```\n\nOffline fixture: no process was polled.".to_owned()}.to_snapshot().expect("bounded terminal preview")),
        "terminal-input" => (ActivityKind::ModelWork,ActivityDocument{title:"Terminal input sent".to_owned(),markdown:"````text\nProcess: preview-7\nCommand: cargo test\nInput:\n```\nThis is literal terminal input.\n````\n\nOffline fixture: no input was sent.".to_owned()}.to_snapshot().expect("bounded terminal preview")),
        "plan" => (ActivityKind::ModelWork, ActivityPlan {
            explanation:Some("Offline example: observed task progress.".to_owned()),
            steps: vec![
                PlanStep {text:"Inspect the renderer".to_owned(),status:PlanStepStatus::Completed},
                PlanStep {text:"Connect real backend events".to_owned(),status:PlanStepStatus::Completed},
                PlanStep {text:"Verify narrow-terminal navigation and preserve wrapped task descriptions".to_owned(),status:PlanStepStatus::InProgress},
                PlanStep {text:"Review the final output".to_owned(),status:PlanStepStatus::Pending},
            ],
        }.to_snapshot().expect("bounded preview plan")),
        "changes" => (ActivityKind::FileChange, "update: src/settings.rs\n@@ -1,3 +1,5 @@\n fn settings() {\n-    let theme = \"fixed\";\n+    let theme = \"selected\";\n+    // 한글 테마 👩‍💻\n+    render(theme);\n }\nadd: tests/settings.rs\n@@ -0,0 +1,4 @@\n+#[test]\n+fn selected_theme_is_retained() {\n+    assert_eq!(theme(), \"selected\");\n+}\n".into()),
        "diff" => (ActivityKind::AgentMessage, "```diff\n--- a/settings.rs\n+++ b/settings.rs\n@@ -1,2 +1,2 @@\n-let theme = \"fixed\";\n+let theme = \"selected\";\n render(theme);\n```".into()),
        "links" => (ActivityKind::AgentMessage, "## Links\n\nBare URL: (https://example.com/plain?a=1&amp;b=2).\n\n[Codex source](https://github.com/openai/codex) · [pi source](https://github.com/earendil-works/pi)\n\n[**한글** and `inline code`](https://example.com/docs) keep their destination when wrapped.\n\n| Source | Link |\n| --- | --- |\n| Rust | [Documentation](https://doc.rust-lang.org/book/) |\n\n```text\n[Literal code](https://example.com/not-linked)\n```\n\n[Local file stays text](file:///tmp/example.rs)\n\nSupported terminals can open web links. This fixture does not open a browser or fetch a page.".to_owned()),
        "markdown" => (ActivityKind::AgentMessage, "## Clearer output\n\n**Readable prose**, `inline code`, and lists.\n\n- Preserve your layout\n- Check 한글 and wrapping\n\n```rust\nfn main() {\n    println!(\"hello\");\n}\n```\n\n> Ready to continue.".into()),
        "long" => (ActivityKind::AgentMessage, (1..=40).map(|n| format!("Line {n}: 한글과 English, wide characters and scrolling.  \n")).collect()),
        _ => (ActivityKind::AgentMessage, format!("받은 입력: {input}\n\n저는 화면 검증용 테스트 에이전트입니다. 실제 모델 호출 없이 응답을 스트리밍합니다.\n\n계속 입력해 대화를 이어가세요. tools · long-tools · error · long · markdown · tables · diff · changes · plan · usage · mcp · approval · interview 를 보내면 해당 UI를 확인할 수 있습니다. Esc로 중단할 수 있습니다.")),
    }
}
