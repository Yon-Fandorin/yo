use std::{
    io::{self, Write},
    ops::Range,
    path::{Component, Path},
    str,
    time::Duration,
};

use serde_json::{Value, from_str};
use similar::TextDiff;
use unicode_segmentation::UnicodeSegmentation;
use yo_core::{
    ActivityKind, FilePublicationEvidence, FilePublicationEvidenceState,
    FilePublicationEvidenceUnavailableReason, ToolOutput,
};

use crate::transcript::TranscriptActivityOutcome;

const MAX_LOCAL_FILE_PATH_BYTES: usize = 1_024;
const MAX_EDIT_PRESENTATION_BYTES: usize = 256 * 1024;
const MAX_PUBLICATION_PRESENTATION_BYTES: usize = 256 * 1024;
const MAX_PUBLICATION_CAPTURE_BYTES: usize = 1024 * 1024;
const MAX_PUBLICATION_DIFF_LINES: usize = 40_000;
const PUBLICATION_DIFF_TRUNCATION: &str = "\n… publication diff truncated at 256 KiB …\n";

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FilePublicationPresentation {
    pub(crate) path: String,
    pub(crate) heading: &'static str,
    pub(crate) provenance: &'static str,
    pub(crate) body: String,
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

// 완료된 managed edit_file의 typed capture만 publication evidence로 표시합니다.
pub(crate) fn completed_file_publication(
    output: &ToolOutput,
    kind: Option<ActivityKind>,
    outcome: Option<TranscriptActivityOutcome>,
) -> Option<FilePublicationEvidence> {
    if output.tool != "edit_file"
        || output.server.is_some()
        || kind != Some(ActivityKind::ToolResult)
        || outcome != Some(TranscriptActivityOutcome::Completed)
        || output.error.is_some()
        || output.content_items.is_some()
    {
        return None;
    }

    let result = output.result.as_ref()?.as_object()?;
    if result.get("tool_id")?.as_str()? != "edit-file"
        || result.get("outcome")?.as_str()? != "completed"
        || result.get("truncated")?.as_bool()?
        || result.get("isError")?.as_bool()?
        || result.get("call_id")?.as_str()?.trim().is_empty()
        || result.get("execution_host")?.as_str()?.trim().is_empty()
    {
        return None;
    }

    let arguments = output.arguments.as_ref()?.as_object()?;
    if !(arguments.len() == 1 || arguments.len() == 2) {
        return None;
    }
    let argument_path = arguments.get("path")?.as_str()?;
    if !admitted_local_path(argument_path) || contains_redaction_marker(argument_path) {
        return None;
    }

    let content = result.get("content")?.as_array()?;
    let [block] = content.as_slice() else {
        return None;
    };
    let block = block.as_object()?;
    if block.get("type")?.as_str()? != "text" {
        return None;
    }
    let (receipt_path, replacement_count) =
        mutation_result_receipt("edit_file", block.get("text")?.as_str()?)?;
    match arguments.get("edits") {
        Some(Value::Array(edits)) if arguments.len() == 2 => {
            if edits.is_empty() || edits.len() > 256 || replacement_count != edits.len() as u64 {
                return None;
            }
            if edits.iter().any(|edit| {
                let Some(fields) = edit.as_object() else {
                    return true;
                };
                fields.len() != 2
                    || fields
                        .get("oldText")
                        .and_then(Value::as_str)
                        .is_none_or(str::is_empty)
                    || fields.get("newText").and_then(Value::as_str).is_none()
            }) {
                return None;
            }
        },
        None if arguments.len() == 1 => {},
        _ => return None,
    }

    let evidence =
        FilePublicationEvidence::from_snapshot(result.get("publicationEvidence")?.as_str()?)?;
    if evidence.path() != argument_path || evidence.path() != receipt_path {
        return None;
    }
    Some(evidence)
}

pub(crate) fn file_publication_presentation(
    evidence: &FilePublicationEvidence,
) -> FilePublicationPresentation {
    let path = evidence.path().to_owned();
    match evidence.state() {
        FilePublicationEvidenceState::Complete { before, after } => FilePublicationPresentation {
            path,
            heading: "Saved edit comparison",
            provenance: "Saved from this completed edit; the current file may have changed since.",
            body: publication_diff(before, after),
            body_style: LocalFileProposalBodyStyle::Diff,
        },
        FilePublicationEvidenceState::Unavailable { reason } => FilePublicationPresentation {
            path,
            heading: "Saved edit comparison unavailable",
            provenance: "The completed edit result recorded no content comparison.",
            body: unavailable_reason(*reason).to_owned(),
            body_style: LocalFileProposalBodyStyle::Plain,
        },
    }
}

fn unavailable_reason(reason: FilePublicationEvidenceUnavailableReason) -> &'static str {
    match reason {
        FilePublicationEvidenceUnavailableReason::Disabled => "Content capture was disabled.",
        FilePublicationEvidenceUnavailableReason::OverBound => {
            "Captured content exceeded the evidence limit."
        },
        FilePublicationEvidenceUnavailableReason::GenerationFailed => {
            "The content capture could not be prepared."
        },
        FilePublicationEvidenceUnavailableReason::SemanticAdmission => {
            "Privacy checks excluded the captured content."
        },
        FilePublicationEvidenceUnavailableReason::InvalidEvidence => {
            "The captured content could not be recognized."
        },
        FilePublicationEvidenceUnavailableReason::SnapshotCapacity => {
            "The saved comparison exceeded the storage limit."
        },
    }
}

pub(crate) fn file_publication_markdown(evidence: &FilePublicationEvidence) -> String {
    let presentation = file_publication_presentation(evidence);
    let body = match presentation.body_style {
        LocalFileProposalBodyStyle::Diff => literal_block("diff", &presentation.body),
        LocalFileProposalBodyStyle::Plain => literal_block("text", &presentation.body),
    };
    let rendered = format!(
        "**{}**\n\nPath:\n\n{}\n\n{}\n\n{}",
        presentation.heading,
        literal_block("text", &presentation.path),
        presentation.provenance,
        body,
    );
    if rendered.len() <= MAX_PUBLICATION_PRESENTATION_BYTES {
        rendered
    } else {
        format!(
            "**{}**\n\nPath:\n\n{}\n\n{}\n\n{}",
            presentation.heading,
            literal_block("text", &presentation.path),
            presentation.provenance,
            literal_block(
                "text",
                "Publication diff omitted because its literal rendering exceeds 256 KiB."
            ),
        )
    }
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
    let label = match tool {
        "edit_file" => "Applied replacements",
        "write_file" => "Written bytes",
        _ => return None,
    };
    let (path, count) = mutation_result_receipt(tool, source)?;
    Some(literal_block("text", &format!("{path}\n{label}: {count}")))
}

fn mutation_result_receipt(tool: &str, source: &str) -> Option<(String, u64)> {
    let field = match tool {
        "edit_file" => "replacements",
        "write_file" => "bytes",
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
    Some((path.to_owned(), count))
}

fn publication_diff(before: &str, after: &str) -> String {
    if before == after {
        return "No textual differences were captured.".to_owned();
    }
    let capture_bytes = before.len().saturating_add(after.len());
    if capture_bytes > MAX_PUBLICATION_CAPTURE_BYTES {
        return "Publication diff omitted: captured content exceeds the presentation limit."
            .to_owned();
    }
    let line_count = before
        .split_inclusive('\n')
        .take(MAX_PUBLICATION_DIFF_LINES + 1)
        .count()
        .saturating_add(
            after
                .split_inclusive('\n')
                .take(MAX_PUBLICATION_DIFF_LINES + 1)
                .count(),
        );
    if line_count > MAX_PUBLICATION_DIFF_LINES {
        return "Publication diff omitted: captured content exceeds the 40,000-line comparison limit."
            .to_owned();
    }

    let diff = TextDiff::configure()
        .timeout(Duration::from_millis(50))
        .diff_lines(before, after);
    let mut writer = BoundedDiffWriter::default();
    let formatted = diff
        .unified_diff()
        .context_radius(3)
        .header("read before edit", "written by edit")
        .to_writer(&mut writer);
    if writer.exceeded || formatted.is_err() {
        writer.output.push_str(PUBLICATION_DIFF_TRUNCATION);
    }
    if writer.output.is_empty() {
        "No textual differences were captured.".to_owned()
    } else {
        writer.output
    }
}

#[derive(Default)]
struct BoundedDiffWriter {
    output: String,
    exceeded: bool,
}

impl Write for BoundedDiffWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let text = str::from_utf8(bytes)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "diff output was not UTF-8"))?;
        let budget = MAX_PUBLICATION_PRESENTATION_BYTES - PUBLICATION_DIFF_TRUNCATION.len();
        let remaining = budget.saturating_sub(self.output.len());
        if bytes.len() <= remaining {
            self.output.push_str(text);
            return Ok(bytes.len());
        }

        let mut appended = 0;
        for grapheme in text.graphemes(true) {
            if grapheme.len() > remaining.saturating_sub(appended) {
                break;
            }
            self.output.push_str(grapheme);
            appended += grapheme.len();
        }
        self.exceeded = true;
        Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "publication diff output limit reached",
        ))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
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
