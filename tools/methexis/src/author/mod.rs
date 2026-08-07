//! Revision authoring facade and wire-contract types.
//!
//! `author-revision` accepts the meaningful inputs for one unit revision (new
//! Source content, new Knowledge body, new Korean review Markdown) and derives
//! every CAS value in a single call: SourceRevision, Knowledge source pin and
//! RevisionId, replacement Projection, and the review packet. It writes
//! tracked Draft proposals only; approval records are never touched and
//! nothing is activated. Callers enter through [`AuthorService`].

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::review::OperationFailure;

const AUTHOR_SCHEMA: &str = "methexis.author-revision/v1alpha1";
const AUTHOR_REQUEST_SCHEMA: &str = "methexis.author-revision-request/v1alpha1";

mod operations;
mod records;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthorRevisionRequest {
    schema: String,
    knowledge_id: String,
    expected_revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    knowledge_body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    korean_markdown: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct PacketRef {
    path: String,
    hash: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct AuthorSuccess {
    schema: &'static str,
    ok: bool,
    operation: &'static str,
    status: &'static str,
    authority: &'static str,
    affected_ids: Vec<String>,
    revision: String,
    projection_hash: String,
    packet: PacketRef,
    changed_paths: Vec<String>,
    request_hash: String,
    next_actions: Vec<String>,
}

pub(crate) struct AuthorService<'a> {
    repository_root: &'a Path,
}

impl<'a> AuthorService<'a> {
    pub(crate) fn new(repository_root: &'a Path) -> Self {
        Self { repository_root }
    }

    pub(crate) fn author_revision(
        &self,
        request_path: &Path,
    ) -> Result<AuthorSuccess, OperationFailure> {
        operations::author_revision(self.repository_root, request_path)
    }
}
