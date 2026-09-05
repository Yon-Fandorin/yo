//! Normalized record reads, front matter, YAML, and local validation.

use std::{collections::BTreeSet, fs, path::Path};

use super::{
    Diagnostic,
    body::{BodyLine, classify_body_lines},
    body_start_line,
    diagnostic::{local_diagnostic, sort_diagnostics},
    knowledge_revision,
};
use crate::model::{
    KNOWLEDGE_SCHEMA, KnowledgeKind, KnowledgeMetadata, KnowledgeUnit, OWNER_SCHEMA, Owner,
    OwnerRecord,
};

const MAX_RECORD_BYTES: usize = 256 * 1024;

pub(super) fn parse_knowledge_file(
    path: &Path,
    repository_root: &Path,
) -> Result<KnowledgeUnit, Vec<Diagnostic>> {
    let display_path = super::diagnostic::display_path(path, repository_root);
    let content = read_normalized(path, &display_path)?;
    let (frontmatter, body) = split_frontmatter(&content).map_err(|message| {
        vec![local_diagnostic(
            display_path.clone(),
            "invalid_frontmatter",
            message,
            Some(1),
            Some(1),
            Vec::new(),
        )]
    })?;
    let metadata = parse_yaml::<KnowledgeMetadata>(frontmatter, &display_path, 1)?;
    let body_start_line = body_start_line(&content, body);
    let mut diagnostics = validate_metadata(&metadata, body, body_start_line, &display_path);
    if !diagnostics.is_empty() {
        sort_diagnostics(&mut diagnostics);
        return Err(diagnostics);
    }
    let revision = knowledge_revision(&metadata, body);
    Ok(KnowledgeUnit {
        metadata,
        body: body.to_owned(),
        path: path.to_owned(),
        revision,
    })
}

pub(super) fn parse_owner_file(
    path: &Path,
    repository_root: &Path,
) -> Result<Owner, Vec<Diagnostic>> {
    let display_path = super::diagnostic::display_path(path, repository_root);
    let content = read_normalized(path, &display_path)?;
    let record = parse_yaml::<OwnerRecord>(&content, &display_path, 0)?;
    let mut diagnostics = Vec::new();
    if record.schema != OWNER_SCHEMA {
        diagnostics.push(local_diagnostic(
            display_path.clone(),
            "unsupported_schema",
            format!("expected owner schema `{OWNER_SCHEMA}`"),
            None,
            None,
            vec![record.id.clone()],
        ));
    }
    if !is_segment(&record.id) {
        diagnostics.push(local_diagnostic(
            display_path,
            "invalid_owner_id",
            "OwnerId must be one lowercase semantic segment".to_owned(),
            None,
            None,
            vec![record.id.clone()],
        ));
    }
    if diagnostics.is_empty() {
        Ok(Owner {
            id: record.id,
            path: path.to_owned(),
        })
    } else {
        Err(diagnostics)
    }
}

pub(crate) fn read_normalized(path: &Path, display_path: &str) -> Result<String, Vec<Diagnostic>> {
    let bytes = fs::read(path).map_err(|error| {
        vec![local_diagnostic(
            display_path.to_owned(),
            "file_unreadable",
            format!("cannot read record: {error}"),
            None,
            None,
            Vec::new(),
        )]
    })?;
    normalize_record_bytes(&bytes, display_path)
}

pub(crate) fn normalize_record_bytes(
    bytes: &[u8],
    display_path: &str,
) -> Result<String, Vec<Diagnostic>> {
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(vec![local_diagnostic(
            display_path.to_owned(),
            "record_too_large",
            format!(
                "record is {} bytes; the Pilot limit is {MAX_RECORD_BYTES} bytes",
                bytes.len()
            ),
            None,
            None,
            Vec::new(),
        )]);
    }
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Err(vec![local_diagnostic(
            display_path.to_owned(),
            "bom_forbidden",
            "UTF-8 BOM is not part of the canonical representation".to_owned(),
            Some(1),
            Some(1),
            Vec::new(),
        )]);
    }
    let content = String::from_utf8(bytes.to_vec()).map_err(|error| {
        vec![local_diagnostic(
            display_path.to_owned(),
            "invalid_utf8",
            format!("record is not valid UTF-8: {error}"),
            None,
            None,
            Vec::new(),
        )]
    })?;
    Ok(normalize_line_endings(content))
}

pub(super) fn normalize_line_endings(content: String) -> String {
    content.replace("\r\n", "\n").replace('\r', "\n")
}

pub(super) fn split_frontmatter(content: &str) -> Result<(&str, &str), String> {
    let remainder = content
        .strip_prefix("---\n")
        .ok_or_else(|| "knowledge file must start with a `---` delimiter".to_owned())?;
    let (frontmatter, body) = remainder
        .split_once("\n---\n")
        .ok_or_else(|| "knowledge frontmatter must end with a `---` delimiter".to_owned())?;
    if body.trim().is_empty() {
        return Err("canonical Markdown body must not be empty".to_owned());
    }
    Ok((frontmatter, body))
}

pub(crate) fn parse_yaml<T>(yaml: &str, path: &str, line_offset: u64) -> Result<T, Vec<Diagnostic>>
where
    T: serde::de::DeserializeOwned,
{
    serde_norway::from_str(yaml).map_err(|error| {
        let location = error.location();
        vec![local_diagnostic(
            path.to_owned(),
            "invalid_yaml",
            error.to_string(),
            location
                .as_ref()
                .map(|location| location.line() as u64 + line_offset),
            location.as_ref().map(|location| location.column() as u64),
            Vec::new(),
        )]
    })
}

pub(crate) fn validate_metadata(
    metadata: &KnowledgeMetadata,
    body: &str,
    body_start_line: u64,
    path: &str,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let affected = vec![metadata.id.clone()];
    if metadata.schema != KNOWLEDGE_SCHEMA {
        diagnostics.push(local_diagnostic(
            path.to_owned(),
            "unsupported_schema",
            format!("expected knowledge schema `{KNOWLEDGE_SCHEMA}`"),
            None,
            None,
            affected.clone(),
        ));
    }
    if !is_semantic_id(&metadata.id) {
        diagnostics.push(local_diagnostic(
            path.to_owned(),
            "invalid_knowledge_id",
            "KnowledgeId must use lowercase dot-separated semantic segments".to_owned(),
            None,
            None,
            affected.clone(),
        ));
    }
    if !is_segment(&metadata.owner) {
        diagnostics.push(local_diagnostic(
            path.to_owned(),
            "invalid_owner_id",
            "OwnerId must be one lowercase semantic segment".to_owned(),
            None,
            None,
            affected.clone(),
        ));
    }
    if metadata.sources.is_empty() {
        diagnostics.push(local_diagnostic(
            path.to_owned(),
            "missing_source",
            "at least one SourceId is required".to_owned(),
            None,
            None,
            affected.clone(),
        ));
    }
    let source_ids = metadata
        .sources
        .iter()
        .map(|source| source.id.clone())
        .collect::<Vec<_>>();
    validate_unique_ids(&source_ids, "source", path, &affected, &mut diagnostics);
    for source in &metadata.sources {
        if !is_semantic_id(&source.id) {
            diagnostics.push(local_diagnostic(
                path.to_owned(),
                "invalid_source_id",
                format!("invalid SourceId `{}`", source.id),
                None,
                None,
                affected.clone(),
            ));
        }
        if !valid_hash(&source.revision) {
            diagnostics.push(local_diagnostic(
                path.to_owned(),
                "invalid_source_revision",
                format!(
                    "Source `{}` must pin a lowercase SHA-256 SourceRevision",
                    source.id
                ),
                None,
                None,
                affected.clone(),
            ));
        }
    }
    for (relation, targets) in metadata.relations.typed() {
        validate_unique_ids(targets, relation, path, &affected, &mut diagnostics);
        for target in targets {
            if target.is_empty() {
                diagnostics.push(local_diagnostic(
                    path.to_owned(),
                    "empty_relation_target",
                    format!("relation `{relation}` contains an empty target"),
                    None,
                    None,
                    affected.clone(),
                ));
            } else if matches!(relation, "depends_on" | "constrained_by" | "supersedes")
                && !is_semantic_id(target)
            {
                diagnostics.push(local_diagnostic(
                    path.to_owned(),
                    "invalid_relation_target",
                    format!("relation `{relation}` has invalid KnowledgeId `{target}`"),
                    None,
                    None,
                    affected.clone(),
                ));
            }
        }
    }
    let body_lines = classify_body_lines(body);
    if let Some((index, _)) = body_lines
        .iter()
        .enumerate()
        .find(|(_, line)| line.forbidden_html)
    {
        diagnostics.push(local_diagnostic(
            path.to_owned(),
            "raw_html_forbidden",
            "canonical Markdown bodies must not contain raw HTML blocks".to_owned(),
            Some(body_start_line + index as u64),
            Some(1),
            affected.clone(),
        ));
    }
    require_body_section(
        &body_lines,
        "Statement",
        body_start_line,
        path,
        &affected,
        &mut diagnostics,
    );
    if metadata.kind == KnowledgeKind::Decision {
        require_body_section(
            &body_lines,
            "Rationale",
            body_start_line,
            path,
            &affected,
            &mut diagnostics,
        );
    }
    if metadata.kind == KnowledgeKind::Procedure {
        require_body_section(
            &body_lines,
            "Steps",
            body_start_line,
            path,
            &affected,
            &mut diagnostics,
        );
        require_body_section(
            &body_lines,
            "Completion Criteria",
            body_start_line,
            path,
            &affected,
            &mut diagnostics,
        );
    }
    diagnostics
}

fn validate_unique_ids(
    values: &[String],
    field: &str,
    path: &str,
    affected: &[String],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            diagnostics.push(local_diagnostic(
                path.to_owned(),
                "duplicate_reference",
                format!("`{field}` contains duplicate target `{value}`"),
                None,
                None,
                affected.to_vec(),
            ));
        }
    }
}

fn require_body_section(
    lines: &[BodyLine<'_>],
    name: &str,
    body_start_line: u64,
    path: &str,
    affected: &[String],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let heading = format!("## {name}");
    let positions = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| (line.heading == Some(heading.as_str())).then_some(index))
        .collect::<Vec<_>>();
    match positions.as_slice() {
        [] => diagnostics.push(local_diagnostic(
            path.to_owned(),
            "missing_body_section",
            format!("canonical body requires section `{heading}`"),
            None,
            None,
            affected.to_vec(),
        )),
        [position] => {
            let content_exists = lines[position + 1..]
                .iter()
                .take_while(|line| line.heading.is_none())
                .any(|line| line.has_content);
            if !content_exists {
                diagnostics.push(local_diagnostic(
                    path.to_owned(),
                    "empty_body_section",
                    format!("canonical body section `{heading}` must not be empty"),
                    Some(body_start_line + *position as u64),
                    Some(1),
                    affected.to_vec(),
                ));
            }
        },
        _ => diagnostics.push(local_diagnostic(
            path.to_owned(),
            "duplicate_body_section",
            format!("canonical body section `{heading}` appears more than once"),
            None,
            None,
            affected.to_vec(),
        )),
    }
}

pub(crate) fn is_semantic_id(id: &str) -> bool {
    !id.is_empty() && id.split('.').all(is_segment)
}

pub(crate) fn valid_hash(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

pub(crate) fn is_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    matches!(bytes.first(), Some(b'a'..=b'z'))
        && matches!(bytes.last(), Some(b'a'..=b'z' | b'0'..=b'9'))
        && bytes
            .iter()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
        && !segment.contains("--")
}
