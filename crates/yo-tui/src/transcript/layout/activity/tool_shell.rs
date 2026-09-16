use std::iter;

use serde_json::{Value, from_str};
use yo_core::ToolOutput;

use super::{NonZeroU16, tool_files::literal_block};
use crate::text::flow::flow_tail_text;

pub(super) fn shell_details_markdown(
    details: &Value,
    output: &ToolOutput,
) -> Option<(String, bool)> {
    if details.to_string().len() > 256 * 1024 {
        return None;
    }
    let fields = details.as_object()?;
    if fields.is_empty()
        || fields
            .keys()
            .any(|key| !matches!(key.as_str(), "fullOutputPath" | "truncation"))
    {
        return None;
    }
    let mut sections = Vec::new();
    if let Some(path) = fields.get("fullOutputPath") {
        let path = path.as_str()?;
        if path.is_empty() || path.len() > 4096 || path.chars().any(char::is_control) {
            return None;
        }
        sections.push(format!(
            "**Full output file**\n\n{}",
            literal_block("text", path)
        ));
    }
    let mut truncated = false;
    if let Some(value) = fields.get("truncation") {
        let truncation = value.as_object()?;
        if truncation.len() != 11
            || truncation.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "content"
                        | "truncated"
                        | "truncatedBy"
                        | "totalLines"
                        | "totalBytes"
                        | "outputLines"
                        | "outputBytes"
                        | "lastLinePartial"
                        | "firstLineExceedsLimit"
                        | "maxLines"
                        | "maxBytes"
                )
            })
        {
            return None;
        }
        let content = value.get("content")?.as_str()?;
        if !content.is_empty()
            && !output.content_blocks().any(|block| {
                block
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| text.starts_with(content))
            })
        {
            return None;
        }

        truncated = value.get("truncated")?.as_bool()?;
        let total_lines = value.get("totalLines")?.as_u64()?;
        let total_bytes = value.get("totalBytes")?.as_u64()?;
        let output_lines = value.get("outputLines")?.as_u64()?;
        let output_bytes = value.get("outputBytes")?.as_u64()?;
        let max_lines = value.get("maxLines")?.as_u64()?;
        let max_bytes = value.get("maxBytes")?.as_u64()?;
        let partial = value.get("lastLinePartial")?.as_bool()?;
        let first_exceeded = value.get("firstLineExceedsLimit")?.as_bool()?;
        if output_lines > total_lines
            || output_lines > max_lines
            || output_bytes > max_bytes
            || output_lines != content.split_terminator('\n').count() as u64
            || output_bytes > total_bytes
            || output_bytes != content.len() as u64
        {
            return None;
        }
        if truncated {
            let reason = value.get("truncatedBy")?.as_str()?;
            let limit = match reason {
                "lines" => format!("Line limit: {max_lines}"),
                "bytes" => format!("Byte limit: {max_bytes} bytes"),
                _ => return None,
            };
            let mut lines = vec![
                format!("Output lines: {output_lines} of {total_lines}"),
                format!("Output bytes: {output_bytes} of {total_bytes}"),
                limit,
            ];
            if partial {
                lines.push("Last line is partial".to_owned());
            }
            if first_exceeded {
                lines.push("First line exceeds the byte limit".to_owned());
            }
            sections.insert(
                0,
                format!(
                    "**Output truncated**\n\n{}",
                    literal_block("text", &lines.join("\n"))
                ),
            );
        } else {
            if !value.get("truncatedBy")?.is_null()
                || partial
                || first_exceeded
                || output_lines != total_lines
                || output_bytes != total_bytes
            {
                return None;
            }
            sections.insert(
                0,
                literal_block(
                    "text",
                    &format!("Complete output: {output_lines} lines · {output_bytes} bytes"),
                ),
            );
        }
    }
    Some((sections.join("\n\n"), truncated))
}

pub(super) fn shell_output_block(source: &str, preview: Option<(NonZeroU16, u16)>) -> String {
    let Some((width, rows)) = preview else {
        return literal_block("text", source);
    };
    let indent = width.get().saturating_sub(2).min(1);
    let padding = (width.get() - indent).saturating_sub(4).min(1);
    let inner = NonZeroU16::new(width.get() - indent - padding).unwrap();
    let Some(rows) = NonZeroU16::new(rows) else {
        return literal_block("text", source);
    };
    let Ok(tail) = flow_tail_text(source, inner, rows) else {
        return literal_block("text", source);
    };
    if tail.skipped_rows == 0 {
        return literal_block("text", source);
    }
    let first_row = tail.skipped_rows;
    let flow = tail.flow;
    // Rebuild already flowed display cells so tabs and wide characters retain their columns.
    let mut lines = vec![String::new(); usize::from(flow.height)];
    let mut columns = vec![0_u16; usize::from(flow.height)];
    for glyph in flow.glyphs {
        let index = usize::from(glyph.point.y);
        let line = &mut lines[index];
        line.extend(iter::repeat_n(
            ' ',
            usize::from(glyph.point.x.saturating_sub(columns[index])),
        ));
        line.push_str(glyph.grapheme.as_str());
        columns[index] = glyph.point.x + glyph.grapheme.width().get();
    }
    format!(
        "{first_row} earlier output rows hidden · Ctrl+O expand\n\n{}",
        literal_block("text", &lines.join("\n"))
    )
}

pub(super) fn command_progress_markdown(
    source: &str,
    preview: Option<(NonZeroU16, u16)>,
) -> Option<String> {
    let value: Value = from_str(source).ok()?;
    if value.as_object()?.len() != 2 {
        return None;
    }
    let stdout = value.get("stdout")?.as_str()?;
    let stderr = value.get("stderr")?.as_str()?;
    Some(format!(
        "**stdout**\n\n{}\n\n**stderr**\n\n{}",
        shell_output_block(
            if stdout.is_empty() {
                "(no stdout yet)"
            } else {
                stdout
            },
            preview
        ),
        shell_output_block(
            if stderr.is_empty() {
                "(no stderr yet)"
            } else {
                stderr
            },
            preview
        )
    ))
}

pub(super) fn command_result_markdown(
    source: &str,
    preview: Option<(NonZeroU16, u16)>,
) -> Option<String> {
    // Native combined output has no escaping for its stream delimiter. Ambiguous
    // or truncated receipts must stay literal, without invented stream attribution.
    if source.contains("[yo: tool output truncated]") {
        return None;
    }
    let (status, body) = source.strip_prefix("status: ")?.split_once("\nstdout:\n")?;
    if status != "signal" && status.parse::<i32>().ok()?.to_string() != status {
        return None;
    }
    let separator = "\nstderr:\n";
    if body.match_indices(separator).count() != 1 {
        return None;
    }
    let (stdout, stderr) = body.split_once(separator)?;
    Some(format!(
        "**Exit status: {status}**\n\n**stdout**\n\n{}\n\n**stderr**\n\n{}",
        shell_output_block(
            if stdout.is_empty() {
                "(no stdout)"
            } else {
                stdout
            },
            preview
        ),
        shell_output_block(
            if stderr.is_empty() {
                "(no stderr)"
            } else {
                stderr
            },
            preview
        )
    ))
}
