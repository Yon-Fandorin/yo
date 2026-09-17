//! Projection 입력, rendering, lineage 검증을 담당한다.

use std::{path::Path, str};

use super::io::{decode_utf8, read_record};
use crate::{
    check,
    check::Diagnostic,
    model::KnowledgeUnit,
    review::{
        COMPILER, PROFILE, PROJECTION_REQUEST_SCHEMA, PROJECTION_SCHEMA, ProjectionInput,
        ProjectionMetadata, ProjectionRecord, hash_bytes, local_diagnostic, relative_path,
        semantic_hash, valid_hash,
    },
};

pub(crate) fn projection_input_hash(id: &str, revision: &str, korean_markdown: &str) -> String {
    semantic_hash(&ProjectionInput {
        schema: PROJECTION_REQUEST_SCHEMA,
        knowledge_id: id,
        expected_revision: revision,
        korean_markdown,
    })
}

pub(crate) fn render_projection(unit: &KnowledgeUnit, request_hash: &str, korean: &str) -> Vec<u8> {
    render_projection_fields(&unit.metadata.id, &unit.revision, request_hash, korean)
}

fn render_projection_fields(
    knowledge_id: &str,
    revision: &str,
    request_hash: &str,
    korean: &str,
) -> Vec<u8> {
    format!(
        "---\nschema: {PROJECTION_SCHEMA}\nknowledge_id: {}\nrevision: {}\nprofile: {PROFILE}\ncompiler: {COMPILER}\nrequest_hash: {request_hash}\n---\n{}",
        knowledge_id,
        revision,
        render_projection_body(korean),
    )
    .into_bytes()
}

/// Projection 하단에 기록하는 결정적 Markdown 본문을 렌더링한다.
pub(crate) fn render_projection_body(korean: &str) -> String {
    format!("# Korean Review Projection\n\n## Translation\n\n{korean}\n")
}

#[allow(clippy::result_large_err)]
pub(crate) fn parse_projection(
    path: &Path,
    repository_root: &Path,
) -> Result<ProjectionRecord, Diagnostic> {
    let display = relative_path(repository_root, path);
    let bytes = read_record(path, repository_root, "projection_unreadable", "Projection")?;
    let content = decode_utf8(&bytes, path, repository_root, "projection_not_utf8")?.to_owned();
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    let remainder = normalized.strip_prefix("---\n").ok_or_else(|| {
        local_diagnostic(
            display.clone(),
            "invalid_projection_frontmatter",
            "Projection must start with `---`".to_owned(),
            Vec::new(),
        )
    })?;
    let (frontmatter, body) = remainder.split_once("\n---\n").ok_or_else(|| {
        local_diagnostic(
            display.clone(),
            "invalid_projection_frontmatter",
            "Projection frontmatter must end with `---`".to_owned(),
            Vec::new(),
        )
    })?;
    let metadata: ProjectionMetadata = serde_norway::from_str(frontmatter).map_err(|error| {
        local_diagnostic(
            display.clone(),
            "invalid_projection_yaml",
            error.to_string(),
            Vec::new(),
        )
    })?;
    let affected = vec![metadata.knowledge_id.clone()];
    let translation = body
        .strip_prefix("# Korean Review Projection\n\n## Translation\n\n")
        .map(str::trim)
        .filter(|translation| !translation.is_empty());
    if metadata.schema != PROJECTION_SCHEMA
        || metadata.profile != PROFILE
        || metadata.compiler != COMPILER
        || !valid_hash(&metadata.revision)
        || !valid_hash(&metadata.request_hash)
        || translation.is_none()
    {
        return Err(local_diagnostic(
            display,
            "invalid_review_projection",
            "Projection schema, lineage, revision, or body is invalid".to_owned(),
            affected,
        ));
    }
    let translation = translation.ok_or_else(|| {
        local_diagnostic(
            relative_path(repository_root, path),
            "invalid_review_projection",
            "Projection body is missing its non-empty Translation section".to_owned(),
            vec![metadata.knowledge_id.clone()],
        )
    })?;
    if metadata.request_hash
        != projection_input_hash(&metadata.knowledge_id, &metadata.revision, translation)
        || check::body_has_forbidden_html(translation)
        || bytes
            != render_projection_fields(
                &metadata.knowledge_id,
                &metadata.revision,
                &metadata.request_hash,
                translation,
            )
    {
        return Err(local_diagnostic(
            display,
            "projection_lineage_mismatch",
            "Projection body does not match its deterministic request lineage".to_owned(),
            affected,
        ));
    }
    Ok(ProjectionRecord {
        metadata,
        path: path.to_owned(),
        hash: hash_bytes(&bytes),
        body: body.to_owned(),
    })
}
