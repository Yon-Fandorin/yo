//! Projection 및 canonical approval record의 결정적 encoding과 parsing을 담당한다.

use std::path::Path;

use serde::{Deserialize, de};

use super::io::{decode_utf8, read_record};
use crate::{
    check::Diagnostic,
    review::{
        APPROVAL_REQUEST_SCHEMA, APPROVAL_SCHEMA, ApprovalBasis, ApprovalInput, ApprovalRecord,
        ApprovalRequest, CANONICAL_APPROVAL_REQUEST_SCHEMA, CANONICAL_APPROVAL_SCHEMA,
        CANONICAL_COMPILER, CANONICAL_PROFILE, COMPILER, CanonicalApprovalInput, CanonicalBasis,
        MAX_RECORD_BYTES, PROFILE, ProjectionRecord, hash_bytes, local_diagnostic, relative_path,
        semantic_hash, valid_hash, valid_review_time,
    },
};

pub(crate) fn render_approval(
    request: &ApprovalRequest,
    projection: Option<&ProjectionRecord>,
    request_hash: &str,
) -> Vec<u8> {
    match request {
        ApprovalRequest::Projection {
            knowledge_id,
            expected_revision,
            reviewer,
            reviewed_at,
            ..
        } => {
            let projection = projection.expect("Projection request was validated with evidence");
            format!(
                "schema: {APPROVAL_SCHEMA}\nknowledge_id: {knowledge_id}\nrevision: {expected_revision}\nreviewer: {reviewer}\nreviewed_at: {reviewed_at}\nprojection_profile: {}\nprojection_compiler: {}\nprojection_hash: {}\nrequest_hash: {request_hash}\n",
                projection.metadata.profile, projection.metadata.compiler, projection.hash,
            )
            .into_bytes()
        },
        ApprovalRequest::Canonical {
            knowledge_id,
            expected_revision,
            reviewer,
            reviewed_at,
            ..
        } => format!(
            "schema: {CANONICAL_APPROVAL_SCHEMA}\nknowledge_id: {knowledge_id}\nrevision: {expected_revision}\nreviewer: {reviewer}\nreviewed_at: {reviewed_at}\nreview_basis: canonical\nrequest_hash: {request_hash}\n"
        )
        .into_bytes(),
    }
}

#[allow(clippy::result_large_err)]
pub(crate) fn parse_approval(
    path: &Path,
    repository_root: &Path,
) -> Result<ApprovalRecord, Diagnostic> {
    let bytes = read_record(path, repository_root, "approval_unreadable", "approval")?;
    parse_approval_bytes(&bytes, path, repository_root)
}

#[allow(clippy::result_large_err)]
pub(crate) fn parse_approval_bytes(
    bytes: &[u8],
    path: &Path,
    repository_root: &Path,
) -> Result<ApprovalRecord, Diagnostic> {
    let display = relative_path(repository_root, path);
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(local_diagnostic(
            display,
            "review_record_too_large",
            format!("approval exceeds {MAX_RECORD_BYTES} bytes"),
            Vec::new(),
        ));
    }
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Err(local_diagnostic(
            display,
            "review_record_bom_forbidden",
            "approval must not start with a UTF-8 BOM".to_owned(),
            Vec::new(),
        ));
    }
    let content = decode_utf8(bytes, path, repository_root, "approval_not_utf8")?;
    #[derive(Deserialize)]
    struct SchemaOnly {
        schema: String,
    }
    let schema: serde_norway::Value = serde_norway::from_str(content).map_err(|error| {
        local_diagnostic(
            relative_path(repository_root, path),
            "invalid_approval_yaml",
            error.to_string(),
            Vec::new(),
        )
    })?;
    let schema = serde_norway::from_value::<SchemaOnly>(schema)
        .map_err(|error| {
            local_diagnostic(
                relative_path(repository_root, path),
                "invalid_approval_yaml",
                error.to_string(),
                Vec::new(),
            )
        })?
        .schema;
    let mut record = match schema.as_str() {
        APPROVAL_SCHEMA => {
            parse_projection_approval(content, &relative_path(repository_root, path))?
        },
        CANONICAL_APPROVAL_SCHEMA => {
            parse_canonical_approval(content, &relative_path(repository_root, path))?
        },
        _ => {
            return Err(local_diagnostic(
                relative_path(repository_root, path),
                "invalid_approval",
                "approval schema is unsupported".to_owned(),
                Vec::new(),
            ));
        },
    };
    let affected = vec![record.knowledge_id.clone()];
    if !valid_hash(&record.revision)
        || !valid_hash(&record.review_hash)
        || !valid_hash(&record.request_hash)
        || !valid_review_time(&record.reviewed_at)
    {
        return Err(local_diagnostic(
            relative_path(repository_root, path),
            "invalid_approval",
            "approval identity, review time, or evidence hash is invalid".to_owned(),
            affected,
        ));
    }
    record.hash = hash_bytes(bytes);
    Ok(record)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionApprovalWire {
    schema: String,
    knowledge_id: String,
    revision: String,
    reviewer: String,
    reviewed_at: String,
    projection_profile: String,
    projection_compiler: String,
    projection_hash: String,
    request_hash: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CanonicalApprovalWire {
    schema: String,
    knowledge_id: String,
    revision: String,
    reviewer: String,
    reviewed_at: String,
    review_basis: CanonicalBasis,
    request_hash: String,
}

#[allow(clippy::result_large_err)]
fn parse_projection_approval(content: &str, display: &str) -> Result<ApprovalRecord, Diagnostic> {
    let wire: ProjectionApprovalWire = parse_approval_wire(content, display)?;
    let expected_request_hash = semantic_hash(&ApprovalInput {
        schema: APPROVAL_REQUEST_SCHEMA,
        knowledge_id: &wire.knowledge_id,
        expected_revision: &wire.revision,
        projection_hash: &wire.projection_hash,
        reviewer: &wire.reviewer,
        reviewed_at: &wire.reviewed_at,
    });
    if wire.schema != APPROVAL_SCHEMA
        || wire.projection_profile != PROFILE
        || wire.projection_compiler != COMPILER
        || !valid_hash(&wire.revision)
        || !valid_hash(&wire.projection_hash)
        || !valid_hash(&wire.request_hash)
        || !valid_review_time(&wire.reviewed_at)
    {
        return Err(local_diagnostic(
            display.to_owned(),
            "invalid_approval",
            "approval schema, Projection profile, or compiler is invalid".to_owned(),
            vec![wire.knowledge_id],
        ));
    }
    if wire.request_hash != expected_request_hash {
        return Err(approval_lineage_diagnostic(display, &wire.knowledge_id));
    }
    Ok(ApprovalRecord {
        knowledge_id: wire.knowledge_id,
        revision: wire.revision,
        reviewer: wire.reviewer,
        reviewed_at: wire.reviewed_at,
        basis: ApprovalBasis::Projection,
        review_profile: wire.projection_profile,
        review_compiler: wire.projection_compiler,
        review_hash: wire.projection_hash,
        request_hash: wire.request_hash,
        hash: String::new(),
    })
}

#[allow(clippy::result_large_err)]
fn parse_canonical_approval(content: &str, display: &str) -> Result<ApprovalRecord, Diagnostic> {
    let wire: CanonicalApprovalWire = parse_approval_wire(content, display)?;
    let expected_request_hash = semantic_hash(&CanonicalApprovalInput {
        schema: CANONICAL_APPROVAL_REQUEST_SCHEMA,
        knowledge_id: &wire.knowledge_id,
        expected_revision: &wire.revision,
        review_basis: wire.review_basis,
        reviewer: &wire.reviewer,
        reviewed_at: &wire.reviewed_at,
    });
    if wire.schema != CANONICAL_APPROVAL_SCHEMA
        || !valid_hash(&wire.revision)
        || !valid_hash(&wire.request_hash)
        || !valid_review_time(&wire.reviewed_at)
    {
        return Err(local_diagnostic(
            display.to_owned(),
            "invalid_approval",
            "canonical approval schema, identity, review time, or request hash is invalid"
                .to_owned(),
            vec![wire.knowledge_id],
        ));
    }
    if wire.request_hash != expected_request_hash {
        return Err(approval_lineage_diagnostic(display, &wire.knowledge_id));
    }
    Ok(ApprovalRecord {
        knowledge_id: wire.knowledge_id,
        revision: wire.revision.clone(),
        reviewer: wire.reviewer,
        reviewed_at: wire.reviewed_at,
        basis: ApprovalBasis::Canonical,
        review_profile: CANONICAL_PROFILE.to_owned(),
        review_compiler: CANONICAL_COMPILER.to_owned(),
        review_hash: wire.revision,
        request_hash: wire.request_hash,
        hash: String::new(),
    })
}

#[allow(clippy::result_large_err)]
fn parse_approval_wire<T: de::DeserializeOwned>(
    content: &str,
    display: &str,
) -> Result<T, Diagnostic> {
    serde_norway::from_str(content).map_err(|error| {
        local_diagnostic(
            display.to_owned(),
            "invalid_approval_yaml",
            error.to_string(),
            Vec::new(),
        )
    })
}

fn approval_lineage_diagnostic(display: &str, knowledge_id: &str) -> Diagnostic {
    local_diagnostic(
        display.to_owned(),
        "approval_lineage_mismatch",
        "approval fields do not match their deterministic request lineage".to_owned(),
        vec![knowledge_id.to_owned()],
    )
}
