use media::image_markdown;
use serde_json::Value;
use tool_files::{
    batch_read_markdown, edit_markdown, file_language, literal_block, mutation_result_markdown,
};
use tool_resources::{
    directory_markdown, embedded_resource_markdown, execution_details, resource_link_markdown,
    search_details_markdown,
};
use tool_shell::{
    command_progress_markdown, command_result_markdown, shell_details_markdown, shell_output_block,
};
use yo_core::{ActivityKind, ToolOutput};

use super::{
    GlyphRole, NonZeroU16, PositionedTranscriptGrapheme, PreparedBody, TranscriptLayoutConfig,
    TranscriptMessage, append_plain, markdown, media, tool_files, tool_resources, tool_shell,
};
use crate::transcript::ToolRenderInput;

pub(super) fn prepare_tool(
    config: &TranscriptLayoutConfig,
    message: &TranscriptMessage,
    width: NonZeroU16,
    format_markdown: bool,
) -> Option<PreparedBody> {
    let activity = message.activity?;
    let (heading, source, footer, rendered, skip_activity_folding) =
        tool_presentation(config, message, width, format_markdown)?;
    let prepared = rendered.and_then(|rendered| {
        markdown::prepare_with_links(
            &rendered,
            width,
            config.show_images,
            config.image_max_width,
            config.show_diagrams,
            config.active_link_resolver(),
            config.code_padding,
        )
        .ok()
    });
    let mut body = PreparedBody {
        skip_activity_folding,
        rasters: Vec::new(),
        glyphs: Vec::new(),
        row_styles: Vec::new(),
        height: 0,
    };
    append_plain(
        &mut body,
        &heading,
        width,
        GlyphRole::ActivityHeading(activity.outcome),
    )
    .ok()?;
    if let Some(prepared) = prepared {
        let offset = body.height;
        body.height = body.height.checked_add(prepared.height)?;
        body.glyphs
            .extend(prepared.glyphs.into_iter().map(|mut glyph| {
                glyph.point.y += offset;
                PositionedTranscriptGrapheme {
                    point: glyph.point,
                    grapheme: glyph.grapheme,
                    role: GlyphRole::ActivityBody,
                    decoration: glyph.decoration,
                    hyperlink: glyph.hyperlink,
                }
            }));
        body.row_styles.extend(
            prepared
                .row_styles
                .into_iter()
                .map(|(row, style)| (row + offset, style)),
        );
        body.rasters
            .extend(prepared.rasters.into_iter().map(|mut raster| {
                raster.area.origin.y += offset;
                raster
            }));
    } else {
        append_plain(&mut body, &source, width, GlyphRole::ActivityBody).ok()?;
    }
    if !footer.is_empty() {
        append_plain(
            &mut body,
            &footer,
            width,
            GlyphRole::ActivityOutcome(activity.outcome),
        )
        .ok()?;
    }
    Some(body)
}

// Both eager and paged rendering consume this exact presentation, including the
// selected custom renderer and shell-tail policy. Source/export stays separate.
pub(super) fn tool_presentation(
    config: &TranscriptLayoutConfig,
    message: &TranscriptMessage,
    width: NonZeroU16,
    format_markdown: bool,
) -> Option<(String, String, String, Option<String>, bool)> {
    let activity = message.activity?;
    let kind = activity
        .kind
        .filter(|kind| matches!(kind, ActivityKind::ToolCall | ActivityKind::ToolResult))?;
    let text = message.text();
    let header_end = text.find('\n').unwrap_or(text.len());
    let footer_start = activity.footer_start.unwrap_or(text.len());
    let source = text[header_end.min(footer_start)..footer_start].strip_prefix('\n')?;
    if source.is_empty() {
        return None;
    }
    let output = ToolOutput::from_snapshot(source);
    let source = output
        .as_ref()
        .map_or(source, |output| output.plain_text.as_str());
    let mut skip_activity_folding = false;
    let rendered = if format_markdown {
        config
            .tool_renderer
            .as_ref()
            .and_then(|renderer| {
                renderer.render(ToolRenderInput {
                    kind,
                    outcome: activity.outcome,
                    source,
                    output: output.as_ref(),
                    expanded: !config.compact_activities(),
                    columns: width,
                })
            })
            .or_else(|| {
                output.as_ref().map(|output| {
                    skip_activity_folding = config.shell_tail_rows > 0 && is_shell(&output.tool);
                    let preview = (skip_activity_folding && config.compact_activities())
                        .then_some((width, config.shell_tail_rows));
                    tool_markdown(output, preview)
                })
            })
    } else {
        None
    };
    if rendered.is_none() && output.is_none() {
        return None;
    }
    Some((
        text[..header_end].to_owned(),
        source.to_owned(),
        text[footer_start..].trim_start_matches('\n').to_owned(),
        rendered,
        skip_activity_folding,
    ))
}
// Source export uses the admitted readable body, independently of visual layout limits.
pub(super) fn tool_source_text(message: &TranscriptMessage) -> Option<String> {
    let output = ToolOutput::from_snapshot(message.tool_source()?)?;
    let text = message.text();
    let header_end = text.find('\n').unwrap_or(text.len());
    let footer_start = message.activity?.footer_start.unwrap_or(text.len());
    let mut source = text[..header_end].to_owned();
    if !output.plain_text.is_empty() {
        source.push('\n');
        source.push_str(&output.plain_text);
    }
    let footer = text[footer_start..].trim_start_matches('\n');
    if !footer.is_empty() {
        source.push('\n');
        source.push_str(footer);
    }
    Some(source)
}

fn is_shell(tool: &str) -> bool {
    matches!(
        tool,
        "run_command" | "bash" | "powershell" | "commandExecution"
    )
}
pub(super) fn content_block_markdown(block: &Value, successful: bool) -> String {
    let kind = block.get("type").and_then(Value::as_str).unwrap_or("");
    match (kind, block.get("text").and_then(Value::as_str)) {
        ("image", _) => {
            let mime = block
                .get("mimeType")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let data = block
                .get("data")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            image_markdown(mime, data)
        },
        ("inputImage", _) => {
            let url = block
                .get("imageUrl")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if let Some((mime, data)) = url
                .strip_prefix("data:")
                .and_then(|value| value.split_once(";base64,"))
            {
                image_markdown(mime, data)
            } else {
                literal_block("text", &format!("Image URL (not fetched)\n{url}"))
            }
        },
        ("resource_link", _) => resource_link_markdown(block)
            .unwrap_or_else(|| literal_block("json", &format!("{block:#}"))),
        ("resource", _) => embedded_resource_markdown(block, successful)
            .unwrap_or_else(|| literal_block("json", &format!("{block:#}"))),
        ("audio" | "inputAudio", _) => literal_block("text", "Audio output · playback unavailable"),
        _ => literal_block("json", &format!("{block:#}")),
    }
}

fn tool_markdown(output: &ToolOutput, preview: Option<(NonZeroU16, u16)>) -> String {
    if output.tool == "webSearch"
        && output.result.is_none()
        && output.content_items.is_none()
        && output.error.is_none()
        && let Some((heading, body)) = output.plain_text.split_once('\n')
        && matches!(
            heading,
            "Web search" | "Open web page" | "Find in web page" | "Web search · other action"
        )
    {
        return format!("**{heading}**\n\n{}", literal_block("text", body));
    }
    let identity = output.server.as_ref().map_or_else(
        || output.tool.clone(),
        |server| format!("{server}.{}", output.tool),
    );
    let is_shell = is_shell(&output.tool);
    let is_read = matches!(output.tool.as_str(), "read" | "read_file" | "readFile");
    let is_write = matches!(output.tool.as_str(), "write" | "write_file" | "writeFile");
    let is_edit = matches!(output.tool.as_str(), "edit" | "edit_file" | "editFile");
    let is_find = output.tool == "find";
    let is_grep = output.tool == "grep";
    let is_search = is_find || is_grep;
    let file_path = (is_read || is_write || is_edit || is_search || output.tool == "list_files")
        .then_some(output.arguments.as_ref())
        .flatten()
        .and_then(|args| args.get("file_path").or_else(|| args.get("path")))
        .and_then(|path| path.as_str());
    let successful = output.error.is_none()
        && output
            .result
            .as_ref()
            .and_then(|result| result.get("isError"))
            .and_then(|value| value.as_bool())
            != Some(true);
    let language = if successful && is_read {
        file_path.map_or("text", file_language)
    } else {
        "text"
    };
    let heading = file_path.map_or_else(|| identity.clone(), |path| format!("{identity} · {path}"));
    let mut sections = vec![literal_block("text", &heading)];
    let mut truncation_presented = false;
    let mut progress_presented = false;
    if let Some(arguments) = &output.arguments {
        let pattern = is_search
            .then(|| arguments.get("pattern").and_then(Value::as_str))
            .flatten();
        let command = is_shell
            .then(|| arguments.get("command").and_then(Value::as_str))
            .flatten();
        let content = is_write
            .then(|| arguments.get("content").and_then(|value| value.as_str()))
            .flatten();
        let edits = is_edit.then(|| edit_markdown(arguments)).flatten();
        let mut displayed_arguments = arguments.clone();
        if let Some(pattern) = pattern {
            displayed_arguments
                .as_object_mut()
                .unwrap()
                .remove("pattern");
            let label = if is_grep {
                "Content search pattern"
            } else {
                "File search pattern"
            };
            sections.push(format!("**{label}**\n\n{}", literal_block("text", pattern)));
        }
        if command.is_some()
            && let Some(fields) = displayed_arguments.as_object_mut()
        {
            fields.remove("command");
        }
        if let Some((_, array)) = &edits
            && let Some(fields) = displayed_arguments.as_object_mut()
        {
            if *array {
                fields.remove("edits");
            } else {
                fields.remove("oldText");
                fields.remove("newText");
            }
        }
        if content.is_some()
            && let Some(fields) = displayed_arguments.as_object_mut()
        {
            fields.remove("content");
        }
        if !displayed_arguments
            .as_object()
            .is_some_and(|fields| fields.is_empty())
        {
            sections.push(format!(
                "**Arguments**\n\n{}",
                literal_block("json", &format!("{displayed_arguments:#}"))
            ));
        }
        if let Some(command) = command {
            let language = if output.tool == "powershell" {
                "powershell"
            } else {
                "bash"
            };
            sections.push(format!(
                "**Command**\n\n{}",
                literal_block(
                    language,
                    if command.is_empty() {
                        "(empty command)"
                    } else {
                        command
                    }
                )
            ));
        }
        if let Some((edits, _)) = edits {
            sections.push(format!("**Proposed replacements**\n\n{edits}"));
        }
        if let Some(content) = content {
            sections.push(format!(
                "**Proposed file content**\n\n{}",
                if content.is_empty() {
                    literal_block("text", "(empty file)")
                } else {
                    literal_block(file_path.map_or("text", file_language), content)
                }
            ));
        }
    }
    for block in output.content_blocks() {
        let kind = block
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let text = block.get("text").and_then(|value| value.as_str());
        let body = match (kind, text) {
            ("diff", Some(text)) => format!(
                "{}\n\n{}",
                literal_block(
                    "text",
                    block
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("File change")
                ),
                if text.is_empty() {
                    literal_block("text", "No textual changes")
                } else {
                    literal_block("diff", text)
                },
            ),
            ("text" | "inputText", Some(text)) => (successful && output.tool == "read_files")
                .then(|| batch_read_markdown(text))
                .flatten()
                .or_else(|| {
                    (successful && output.tool == "list_files")
                        .then(|| {
                            let truncated = output.result.as_ref()?.get("truncated")?.as_bool()?;
                            directory_markdown(text, truncated)
                                .inspect(|_| truncation_presented = true)
                        })
                        .flatten()
                })
                .or_else(|| {
                    (output.tool == "run_command"
                        && output
                            .result
                            .as_ref()
                            .and_then(|result| result.get("progress"))
                            .and_then(Value::as_bool)
                            == Some(true))
                    .then(|| {
                        command_progress_markdown(text, preview)
                            .inspect(|_| progress_presented = true)
                    })
                    .flatten()
                })
                .or_else(|| {
                    (output.tool == "run_command")
                        .then(|| command_result_markdown(text, preview))
                        .flatten()
                })
                .or_else(|| {
                    successful
                        .then(|| mutation_result_markdown(&output.tool, text))
                        .flatten()
                })
                .unwrap_or_else(|| {
                    if is_search && successful {
                        let label = if is_grep {
                            "Search results"
                        } else {
                            "Matching paths"
                        };
                        format!("**{label}**\n\n{}", literal_block("text", text))
                    } else if is_shell {
                        shell_output_block(text, preview)
                    } else {
                        literal_block(language, text)
                    }
                }),
            _ => content_block_markdown(block, successful),
        };
        sections.push(body);
    }
    let has_content = output
        .result
        .as_ref()
        .and_then(|result| result.get("content"))
        .or(output.content_items.as_ref());
    if has_content.is_some_and(|content| content.as_array().is_some_and(|blocks| blocks.is_empty()))
    {
        sections.push(literal_block("text", "(empty content)"));
    }
    if let Some(items) = output
        .content_items
        .as_ref()
        .filter(|items| !items.is_array())
    {
        sections.push(literal_block("json", &format!("{items:#}")));
    }
    if let Some(result) = &output.result {
        let mut remaining = result.clone();
        if is_shell {
            let mut details = Vec::new();
            for (key, label, suffix) in [
                ("exitCode", "Exit status", ""),
                ("durationMs", "Duration", " ms"),
                ("status", "Status", ""),
            ] {
                if let Some(value) = remaining.get(key) {
                    let text = match key {
                        "exitCode" => value.as_i64().map(|value| value.to_string()),
                        "durationMs" => value.as_u64().map(|value| value.to_string()),
                        _ => value.as_str().map(str::to_owned),
                    };
                    if let Some(text) = text {
                        details.push(format!("{label}: {text}{suffix}"));
                        remaining.as_object_mut().unwrap().remove(key);
                    }
                }
            }
            if !details.is_empty() {
                sections.push(literal_block("text", &details.join("\n")));
            }
        }
        if is_shell
            && let Some(details) = remaining.get("details")
            && let Some((markdown, truncated)) = shell_details_markdown(details, output)
        {
            sections.push(markdown);
            truncation_presented |= truncated;
            remaining.as_object_mut().unwrap().remove("details");
        }
        if is_search
            && successful
            && let Some(details) = remaining.get("details")
            && let Some((markdown, truncated)) = search_details_markdown(details, output)
        {
            sections.push(markdown);
            truncation_presented |= truncated;
            remaining.as_object_mut().unwrap().remove("details");
        }
        if remaining.get("content").is_some_and(Value::is_array)
            && let Some(details) = execution_details(&remaining)
        {
            sections.push(format!(
                "**Execution details**\n\n{}",
                literal_block("text", &details)
            ));
            let fields = remaining
                .as_object_mut()
                .expect("validated execution metadata");
            for key in ["call_id", "tool_id", "execution_host", "outcome", "isError"] {
                fields.remove(key);
            }
        }
        let result = &remaining;
        if result.as_object().is_some_and(|fields| fields.is_empty()) {
            // Reported command metadata was already displayed above.
        } else if let Some(fields) = result
            .as_object()
            .filter(|fields| fields.get("content").is_some_and(|value| value.is_array()))
        {
            for (key, value) in fields.iter().filter(|(key, _)| *key != "content") {
                if key == "retainedOutput"
                    && value.as_object().is_some_and(|fields| fields.len() == 1)
                    && let Some(truncated) = value.get("truncated").and_then(Value::as_bool)
                {
                    sections.push(if truncated {
                        "**Retained output · /output**\n\nSome captured output was omitted or unavailable.".to_owned()
                    } else {
                        "**Retained output · /output**\n\nCaptured output is available in the output viewer.".to_owned()
                    });
                    continue;
                }
                if key == "progress" && progress_presented {
                    continue;
                }
                if key == "truncated"
                    && let Some(truncated) = value.as_bool()
                {
                    if truncated && !truncation_presented {
                        sections.push("**Output truncated**".to_owned());
                    }
                    continue;
                }
                let label = match key.as_str() {
                    "structuredContent" => "**Structured result**".to_owned(),
                    "_meta" => "**Metadata**".to_owned(),
                    _ => literal_block("text", key),
                };
                sections.push(format!(
                    "{label}\n\n{}",
                    literal_block("json", &format!("{value:#}"))
                ));
            }
        } else {
            sections.push(literal_block("json", &format!("{result:#}")));
        }
    }
    if let Some(error) = &output.error {
        sections.push(format!(
            "**Error details**\n\n{}",
            literal_block("json", &format!("{error:#}"))
        ));
    }
    sections.join("\n\n")
}
