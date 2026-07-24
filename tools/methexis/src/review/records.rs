//! Deterministic Projection and approval record encoding and validation.

use std::{fs, io::Read, path::Path};

use super::{
    APPROVAL_REQUEST_SCHEMA, APPROVAL_SCHEMA, ApprovalInput, ApprovalRecord, ApprovalRequest,
    COMPILER, MAX_RECORD_BYTES, PROFILE, PROJECTION_REQUEST_SCHEMA, PROJECTION_SCHEMA,
    ProjectionInput, ProjectionMetadata, ProjectionRecord, hash_bytes, local_diagnostic,
    relative_path, semantic_hash, valid_hash, valid_review_time,
};
use crate::{check::Diagnostic, model::KnowledgeUnit};

pub(super) fn projection_input_hash(id: &str, revision: &str, korean_markdown: &str) -> String {
    semantic_hash(&ProjectionInput {
        schema: PROJECTION_REQUEST_SCHEMA,
        knowledge_id: id,
        expected_revision: revision,
        korean_markdown,
    })
}

pub(super) fn render_projection(unit: &KnowledgeUnit, request_hash: &str, korean: &str) -> Vec<u8> {
    render_projection_fields(&unit.metadata.id, &unit.revision, request_hash, korean)
}

fn render_projection_fields(
    knowledge_id: &str,
    revision: &str,
    request_hash: &str,
    korean: &str,
) -> Vec<u8> {
    format!(
        "---\nschema: {PROJECTION_SCHEMA}\nknowledge_id: {}\nrevision: {}\nprofile: {PROFILE}\ncompiler: {COMPILER}\nrequest_hash: {request_hash}\n---\n# Korean Review Projection\n\n## Translation\n\n{korean}\n",
        knowledge_id, revision
    )
    .into_bytes()
}

pub(super) fn render_review_packet(unit: &KnowledgeUnit, projection: &ProjectionRecord) -> String {
    let sources = unit
        .metadata
        .sources
        .iter()
        .map(|source| format!("- `{source}` — `not_evaluated`"))
        .collect::<Vec<_>>()
        .join("\n");
    let relations = unit
        .metadata
        .relations
        .typed()
        .into_iter()
        .flat_map(|(kind, targets)| {
            targets
                .iter()
                .map(move |target| format!("- `{kind}` → `{target}`"))
        })
        .collect::<Vec<_>>();
    let relations = if relations.is_empty() {
        "- none".to_owned()
    } else {
        relations.join("\n")
    };
    format!(
        "# Methexis Review Packet\n\n- KnowledgeId: `{}`\n- RevisionId: `{}`\n- OwnerId: `{}`\n- Projection profile: `{}`\n- Projection compiler: `{}`\n- Projection hash: `{}`\n- Source validation: `not_evaluated`\n\n## Canonical English\n\n{}\n\n## Korean Review Projection\n\n{}\n\n## Source references\n\n{}\n\n## Relations\n\n{}\n",
        unit.metadata.id,
        unit.revision,
        unit.metadata.owner,
        projection.metadata.profile,
        projection.metadata.compiler,
        projection.hash,
        unit.body.trim_end(),
        projection.body.trim_end(),
        sources,
        relations,
    )
}

pub(super) fn render_approval(
    request: &ApprovalRequest,
    projection: &ProjectionRecord,
    request_hash: &str,
) -> Vec<u8> {
    format!(
        "schema: {APPROVAL_SCHEMA}\nknowledge_id: {}\nrevision: {}\nreviewer: {}\nreviewed_at: {}\nprojection_profile: {}\nprojection_compiler: {}\nprojection_hash: {}\nrequest_hash: {request_hash}\n",
        request.knowledge_id,
        request.expected_revision,
        request.reviewer,
        request.reviewed_at,
        projection.metadata.profile,
        projection.metadata.compiler,
        projection.hash,
    )
    .into_bytes()
}

#[allow(clippy::result_large_err)]
pub(super) fn parse_projection(
    path: &Path,
    repository_root: &Path,
) -> Result<ProjectionRecord, Diagnostic> {
    let display = relative_path(repository_root, path);
    let bytes = read_record(path, repository_root, "projection_unreadable", "Projection")?;
    let content = String::from_utf8(bytes.clone()).map_err(|error| {
        local_diagnostic(
            display.clone(),
            "projection_not_utf8",
            error.to_string(),
            Vec::new(),
        )
    })?;
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
        || crate::check::body_has_forbidden_html(translation)
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

#[allow(clippy::result_large_err)]
pub(super) fn parse_approval(
    path: &Path,
    repository_root: &Path,
) -> Result<ApprovalRecord, Diagnostic> {
    let display = relative_path(repository_root, path);
    let bytes = read_record(path, repository_root, "approval_unreadable", "approval")?;
    let content = String::from_utf8(bytes).map_err(|error| {
        local_diagnostic(
            display.clone(),
            "approval_not_utf8",
            error.to_string(),
            Vec::new(),
        )
    })?;
    let record: ApprovalRecord = serde_norway::from_str(&content).map_err(|error| {
        local_diagnostic(
            display.clone(),
            "invalid_approval_yaml",
            error.to_string(),
            Vec::new(),
        )
    })?;
    let affected = vec![record.knowledge_id.clone()];
    let expected_request_hash = semantic_hash(&ApprovalInput {
        schema: APPROVAL_REQUEST_SCHEMA,
        knowledge_id: &record.knowledge_id,
        expected_revision: &record.revision,
        projection_hash: &record.projection_hash,
        reviewer: &record.reviewer,
        reviewed_at: &record.reviewed_at,
    });
    if record.schema != APPROVAL_SCHEMA
        || record.projection_profile != PROFILE
        || record.projection_compiler != COMPILER
        || !valid_hash(&record.revision)
        || !valid_hash(&record.projection_hash)
        || !valid_hash(&record.request_hash)
        || !valid_review_time(&record.reviewed_at)
    {
        return Err(local_diagnostic(
            display,
            "invalid_approval",
            "approval schema, identity, review time, or evidence hash is invalid".to_owned(),
            affected,
        ));
    }
    if record.request_hash != expected_request_hash {
        return Err(local_diagnostic(
            display,
            "approval_lineage_mismatch",
            "approval fields do not match their deterministic request lineage".to_owned(),
            affected,
        ));
    }
    Ok(record)
}

#[allow(clippy::result_large_err)]
fn read_record(
    path: &Path,
    repository_root: &Path,
    unreadable_code: &str,
    record_name: &str,
) -> Result<Vec<u8>, Diagnostic> {
    let display = relative_path(repository_root, path);
    let mut file = fs::File::open(path).map_err(|error| {
        local_diagnostic(
            display.clone(),
            unreadable_code,
            error.to_string(),
            Vec::new(),
        )
    })?;
    let mut bytes = Vec::new();
    Read::take(&mut file, (MAX_RECORD_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            local_diagnostic(
                display.clone(),
                unreadable_code,
                error.to_string(),
                Vec::new(),
            )
        })?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(local_diagnostic(
            display,
            "review_record_too_large",
            format!("{record_name} exceeds {MAX_RECORD_BYTES} bytes"),
            Vec::new(),
        ));
    }
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Err(local_diagnostic(
            display,
            "review_record_bom_forbidden",
            format!("{record_name} must not start with a UTF-8 BOM"),
            Vec::new(),
        ));
    }
    Ok(bytes)
}
