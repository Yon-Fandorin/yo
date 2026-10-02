use std::{
    ops::Range,
    path::{Component, Path},
};

use serde_json::{Value, from_str};
use yo_core::ToolOutput;

const MAX_LOCAL_FILE_PATH_BYTES: usize = 1_024;
const MAX_EDIT_PRESENTATION_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LocalFileProposalBodyStyle {
    Diff,
    Plain,
}

pub(crate) struct LocalFileProposal<'a> {
    pub(crate) path: &'a str,
    kind: ValidatedLocalFileProposalKind<'a>,
    pub(crate) body_style: LocalFileProposalBodyStyle,
}

struct ValidatedLocalFileProposal<'a> {
    path: &'a str,
    kind: ValidatedLocalFileProposalKind<'a>,
}

enum ValidatedLocalFileProposalKind<'a> {
    Replacements(Vec<(&'a str, &'a str)>),
    FileContent(&'a str),
}

impl LocalFileProposal<'_> {
    pub(crate) const fn label(&self) -> &'static str {
        match &self.kind {
            ValidatedLocalFileProposalKind::Replacements(_) => "Proposed replacements",
            ValidatedLocalFileProposalKind::FileContent(_) => "Proposed file content",
        }
    }

    pub(crate) fn append_detail_body(&self, source: &mut String) -> Range<usize> {
        let start = source.len();
        match &self.kind {
            ValidatedLocalFileProposalKind::Replacements(replacements) => {
                append_replacement_details(source, replacements)
            },
            ValidatedLocalFileProposalKind::FileContent(content) => {
                if content.is_empty() {
                    source.push_str("(empty file)");
                } else {
                    source.push_str(content);
                }
            },
        }
        start..source.len()
    }
}

// 로컬 파일 제안은 허용된 인수 모양이 정확할 때만 구조화하고 나머지는 원본 그대로 둡니다.
pub(crate) fn local_file_proposal(output: &ToolOutput) -> Option<LocalFileProposal<'_>> {
    let validated = validate_local_file_proposal(output)?;
    match validated.kind {
        ValidatedLocalFileProposalKind::Replacements(replacements) => Some(LocalFileProposal {
            path: validated.path,
            kind: ValidatedLocalFileProposalKind::Replacements(replacements),
            body_style: LocalFileProposalBodyStyle::Diff,
        }),
        ValidatedLocalFileProposalKind::FileContent(content) => Some(LocalFileProposal {
            path: validated.path,
            kind: ValidatedLocalFileProposalKind::FileContent(content),
            body_style: LocalFileProposalBodyStyle::Plain,
        }),
    }
}

pub(crate) fn local_file_proposal_markdown(output: &ToolOutput) -> Option<String> {
    let validated = validate_local_file_proposal(output)?;
    match validated.kind {
        ValidatedLocalFileProposalKind::Replacements(replacements) => {
            Some(replacement_markdown(&replacements))
        },
        ValidatedLocalFileProposalKind::FileContent(content) => {
            Some(file_content_markdown(validated.path, content))
        },
    }
}

pub(crate) fn local_file_proposal_path(output: &ToolOutput) -> Option<&str> {
    validate_local_file_proposal(output).map(|proposal| proposal.path)
}

fn validate_local_file_proposal(output: &ToolOutput) -> Option<ValidatedLocalFileProposal<'_>> {
    let arguments = output.arguments.as_ref()?.as_object()?;
    let path = arguments.get("path")?.as_str()?;
    if !admitted_local_path(path) || contains_redaction_marker(path) {
        return None;
    }

    match output.tool.as_str() {
        "edit_file" => {
            if arguments.len() != 2 {
                return None;
            }
            let edits = arguments.get("edits")?.as_array()?;
            if edits.is_empty() || edits.len() > 256 {
                return None;
            }
            let mut replacements = Vec::with_capacity(edits.len());
            let mut bytes = 0usize;
            for edit in edits {
                let fields = edit.as_object()?;
                if fields.len() != 2 {
                    return None;
                }
                let old = fields.get("oldText")?.as_str()?;
                let new = fields.get("newText")?.as_str()?;
                bytes = bytes.checked_add(old.len())?.checked_add(new.len())?;
                if old.is_empty()
                    || bytes > MAX_EDIT_PRESENTATION_BYTES
                    || contains_redaction_marker(old)
                    || contains_redaction_marker(new)
                {
                    return None;
                }
                replacements.push((old, new));
            }
            Some(ValidatedLocalFileProposal {
                path,
                kind: ValidatedLocalFileProposalKind::Replacements(replacements),
            })
        },
        "write_file" => {
            if arguments.len() != 2 {
                return None;
            }
            let content = arguments.get("content")?.as_str()?;
            if content.len() > ToolOutput::MAX_SNAPSHOT_BYTES || contains_redaction_marker(content)
            {
                return None;
            }
            Some(ValidatedLocalFileProposal {
                path,
                kind: ValidatedLocalFileProposalKind::FileContent(content),
            })
        },
        _ => None,
    }
}

fn file_content_markdown(path: &str, content: &str) -> String {
    format!(
        "**Proposed file content**\n\n{}",
        if content.is_empty() {
            literal_block("text", "(empty file)")
        } else {
            literal_block(file_language(path), content)
        }
    )
}

fn admitted_local_path(path: &str) -> bool {
    if path.is_empty()
        || path.len() > MAX_LOCAL_FILE_PATH_BYTES
        || path.chars().any(char::is_control)
    {
        return false;
    }
    let parsed = Path::new(path);
    !parsed.is_absolute()
        && parsed
            .components()
            .any(|component| matches!(component, Component::Normal(_)))
        && !parsed.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

fn contains_redaction_marker(source: &str) -> bool {
    let source = source.as_bytes();
    ["[redacted]", "[redacted-output]", "[hidden]"]
        .iter()
        .any(|marker| {
            source.windows(marker.len()).any(|window| {
                window
                    .iter()
                    .zip(marker.as_bytes())
                    .all(|(byte, marker)| byte.eq_ignore_ascii_case(marker))
            })
        })
}

fn replacement_markdown(replacements: &[(&str, &str)]) -> String {
    let mut chat_sections = Vec::with_capacity(replacements.len());
    for (index, (old, new)) in replacements.iter().enumerate() {
        let diff = replacement_diff(old, new);
        let heading = format!("Replacement {}", index + 1);
        chat_sections.push(format!("**{heading}**\n\n{}", literal_block("diff", &diff)));
    }
    format!(
        "**Proposed replacements**\n\n{}",
        chat_sections.join("\n\n")
    )
}

fn append_replacement_details(source: &mut String, replacements: &[(&str, &str)]) {
    for (index, (old, new)) in replacements.iter().enumerate() {
        if index > 0 {
            source.push('\n');
        }
        source.push_str(&format!("Replacement {}\n", index + 1));
        source.push_str(&replacement_diff(old, new));
    }
}

fn replacement_diff(old: &str, new: &str) -> String {
    let mut diff = String::new();
    for (prefix, source) in [('-', old), ('+', new)] {
        for line in source.split_inclusive('\n') {
            diff.push(prefix);
            diff.push_str(line);
            if !line.ends_with('\n') {
                diff.push_str(if prefix == '-' {
                    "\n\\ Old text has no trailing newline\n"
                } else {
                    "\n\\ New text has no trailing newline\n"
                });
            }
        }
    }
    diff
}

pub(super) fn mutation_result_markdown(tool: &str, source: &str) -> Option<String> {
    let (field, label) = match tool {
        "edit_file" => ("replacements", "Applied replacements"),
        "write_file" => ("bytes", "Written bytes"),
        _ => return None,
    };
    if source.len() > 16 * 1024 {
        return None;
    }
    let value: Value = from_str(source).ok()?;
    let result = value.as_object()?;
    if result.len() != 3 || result.get("status")?.as_str()? != "ok" {
        return None;
    }
    let count = result.get(field)?.as_u64()?;
    let path = result.get("path")?.as_str()?;
    Some(literal_block("text", &format!("{path}\n{label}: {count}")))
}

pub(super) fn edit_markdown(arguments: &Value) -> Option<(String, bool)> {
    let array = arguments.get("edits");
    let replacements = if let Some(edits) = array {
        if arguments.get("oldText").is_some() || arguments.get("newText").is_some() {
            return None;
        }
        let edits = edits.as_array()?;
        if edits.is_empty() || edits.len() > 256 {
            return None;
        }
        edits
            .iter()
            .map(|edit| {
                if edit.as_object()?.len() != 2 {
                    return None;
                }
                Some((
                    edit.get("oldText")?.as_str()?,
                    edit.get("newText")?.as_str()?,
                ))
            })
            .collect::<Option<Vec<_>>>()?
    } else {
        vec![(
            arguments.get("oldText")?.as_str()?,
            arguments.get("newText")?.as_str()?,
        )]
    };
    let mut bytes = 0usize;
    for (old, new) in &replacements {
        bytes = bytes.checked_add(old.len())?.checked_add(new.len())?;
        if old.is_empty() || bytes > 256 * 1024 {
            return None;
        }
    }
    let markdown = replacement_markdown(&replacements);
    Some((
        markdown
            .strip_prefix("**Proposed replacements**\n\n")
            .unwrap_or(&markdown)
            .to_owned(),
        array.is_some(),
    ))
}

pub(super) fn batch_read_markdown(source: &str) -> Option<String> {
    // Only this native tool's bounded, complete result shape becomes file panels.
    // Unknown fields or truncated JSON retain the entire literal source.
    if source.len() > 256 * 1024 {
        return None;
    }
    let value: Value = from_str(source).ok()?;
    let root = value.as_object()?;
    if root.len() != 1 {
        return None;
    }
    let results = root.get("results")?.as_array()?;
    if results.is_empty() || results.len() > 8 {
        return None;
    }
    let mut sections = Vec::new();
    for result in results {
        let item = result.as_object()?;
        let path = item.get("path")?.as_str()?;
        let status = item.get("status")?.as_str()?;
        match status {
            "error" => {
                if item.len() != 3 {
                    return None;
                }
                let error = item.get("error")?.as_str()?;
                sections.push(literal_block(
                    "text",
                    &format!("Read failed · {path}\n{error}"),
                ));
            },
            "ok" => {
                if item.keys().any(|key| {
                    !matches!(
                        key.as_str(),
                        "path" | "status" | "start" | "end" | "total" | "content" | "next_offset"
                    )
                }) {
                    return None;
                }
                let start = item.get("start")?.as_u64()?;
                let end = item.get("end")?.as_u64()?;
                let total = item.get("total")?.as_u64()?;
                let content = item.get("content")?.as_str()?;
                let next = match item.get("next_offset") {
                    Some(value) => Some(value.as_u64()?),
                    None => None,
                };
                if total == 0 {
                    if start != 0 || end != 0 || !content.is_empty() || next.is_some() {
                        return None;
                    }
                    sections.push(literal_block("text", &format!("{path}\n(empty file)")));
                } else {
                    if start == 0
                        || end < start
                        || end > total
                        || content.lines().count() as u64 != end - start + 1
                        || next != (end < total).then(|| end + 1)
                    {
                        return None;
                    }
                    sections.push(literal_block(
                        "text",
                        &format!("{path}\nLines {start}–{end} of {total}"),
                    ));
                    sections.push(literal_block(file_language(path), content));
                    if let Some(next) = next {
                        sections.push(literal_block(
                            "text",
                            &format!("Continue reading at line {next}"),
                        ));
                    }
                }
            },
            _ => return None,
        }
    }
    Some(sections.join("\n\n"))
}

pub(super) fn file_language(path: &str) -> &'static str {
    // Presentation-only suffix inference: never resolve paths or interpret Markdown,
    // diagrams or image syntax contained in file output.
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match file
        .rsplit_once('.')
        .map_or("", |(_, extension)| extension)
        .to_ascii_lowercase()
        .as_str()
    {
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "json" => "json",
        "sh" | "bash" => "bash",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" => "cpp",
        "go" => "go",
        "rb" => "ruby",
        "java" => "java",
        "html" | "htm" => "html",
        "css" => "css",
        "xml" => "xml",
        "yaml" | "yml" => "yaml",
        "sql" => "sql",
        _ => "text",
    }
}

pub(super) fn literal_block(language: &str, source: &str) -> String {
    // A tool's literal fences must not escape into image/link presentation markup.
    let longest = source
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}{language}\n{source}\n{fence}")
}
