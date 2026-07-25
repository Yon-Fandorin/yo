//! Exact Methexis Projection lineage and canonical-byte validation.

use serde::Serialize;

use super::{
    markdown::has_forbidden_html,
    records::{
        PROJECTION_COMPILER, PROJECTION_PROFILE, PROJECTION_REQUEST_SCHEMA, PROJECTION_SCHEMA,
        ProjectionMetadata,
    },
};
use crate::hash::digest;

#[derive(Serialize)]
struct ProjectionInput<'a> {
    schema: &'static str,
    knowledge_id: &'a str,
    expected_revision: &'a str,
    korean_markdown: &'a str,
}

pub(crate) fn validate(
    metadata: &ProjectionMetadata,
    body: &str,
    exact_bytes: &[u8],
) -> Result<(), &'static str> {
    let translation = body
        .strip_prefix("# Korean Review Projection\n\n## Translation\n\n")
        .map(str::trim)
        .filter(|translation| !translation.is_empty())
        .ok_or("Projection body must contain the canonical non-empty Translation section")?;
    if metadata.schema != PROJECTION_SCHEMA
        || metadata.profile != PROJECTION_PROFILE
        || metadata.compiler != PROJECTION_COMPILER
        || !super::validation::valid_hash(&metadata.revision)
        || !super::validation::valid_hash(&metadata.request_hash)
    {
        return Err("Projection schema, compiler, profile, or hashes are invalid");
    }
    let request = ProjectionInput {
        schema: PROJECTION_REQUEST_SCHEMA,
        knowledge_id: &metadata.knowledge_id,
        expected_revision: &metadata.revision,
        korean_markdown: translation,
    };
    let request_bytes =
        serde_json::to_vec(&request).expect("closed Projection input always serializes");
    if metadata.request_hash != digest(&request_bytes) {
        return Err("Projection request lineage does not match its Translation");
    }
    if has_forbidden_html(translation) {
        return Err("Projection Translation contains forbidden raw HTML");
    }
    let rendered = render(metadata, translation);
    if exact_bytes != rendered {
        return Err("Projection bytes are not the canonical Methexis rendering");
    }
    Ok(())
}

fn render(metadata: &ProjectionMetadata, translation: &str) -> Vec<u8> {
    format!(
        "---\nschema: {PROJECTION_SCHEMA}\nknowledge_id: {}\nrevision: {}\nprofile: {PROJECTION_PROFILE}\ncompiler: {PROJECTION_COMPILER}\nrequest_hash: {}\n---\n# Korean Review Projection\n\n## Translation\n\n{translation}\n",
        metadata.knowledge_id, metadata.revision, metadata.request_hash
    )
    .into_bytes()
}

#[cfg(test)]
pub(crate) fn fixture_bytes(knowledge_id: &str, revision: &str, translation: &str) -> Vec<u8> {
    let request = ProjectionInput {
        schema: PROJECTION_REQUEST_SCHEMA,
        knowledge_id,
        expected_revision: revision,
        korean_markdown: translation,
    };
    let request_hash =
        digest(&serde_json::to_vec(&request).expect("closed Projection input always serializes"));
    render(
        &ProjectionMetadata {
            schema: PROJECTION_SCHEMA.to_owned(),
            knowledge_id: knowledge_id.to_owned(),
            revision: revision.to_owned(),
            profile: PROJECTION_PROFILE.to_owned(),
            compiler: PROJECTION_COMPILER.to_owned(),
            request_hash,
        },
        translation,
    )
}
