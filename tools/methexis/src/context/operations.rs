//! End-to-end Context Resolution orchestration.

use std::{collections::BTreeSet, fs::File, io::Read, path::Path};

use super::{
    candidate, payload, selection, storage,
    wire::{
        Anchor, REQUEST_SCHEMA, ResolveFailure, ResolveRequest, ResolveSuccess, TOKENIZER_PROFILE,
    },
};
use crate::checkpoint::{self, AuthorityFailure};

const MAX_REQUEST_BYTES: usize = 256 * 1024;
const MAX_ANCHORS: usize = 128;
const MAX_ANCHOR_BYTES: usize = 4 * 1024;
const MAX_PATH_BYTES: usize = 4 * 1024;
const MAX_TOKENS: usize = 1_000_000;

pub(super) fn resolve(
    repository_root: &Path,
    request_path: &Path,
) -> Result<ResolveSuccess, ResolveFailure> {
    let mut request = read_request(request_path)?;
    validate_request(&mut request)?;
    let authority =
        checkpoint::resolve_context_authority(repository_root).map_err(authority_failure)?;
    let candidates = request
        .candidates
        .as_ref()
        .map(|reference| candidate::capture(repository_root, reference))
        .transpose()
        .map_err(|failure| failure.with_trusted_commit(&authority.trusted_commit))?;
    let candidate_slice = candidates
        .as_ref()
        .map_or(&[][..], |capture| capture.set.candidates.as_slice());
    let selection = selection::pack(
        &authority,
        &request.anchors,
        candidate_slice,
        request.max_tokens,
        |included| {
            let context = payload::render(&authority, included);
            payload::count_tokens(&context)
        },
    )?;
    let artifacts = payload::compile(
        &authority,
        &selection,
        candidates.as_ref().map(|capture| capture.hash.as_str()),
        candidates.as_ref().map(|capture| &capture.set),
        request.max_tokens,
    );
    let stored = storage::publish(repository_root, &artifacts, || {
        if let Some(capture) = &candidates {
            candidate::final_revalidate(repository_root, capture)
                .map_err(|failure| failure.with_trusted_commit(&authority.trusted_commit))?;
        }
        checkpoint::final_revalidate_context_authority(repository_root, &authority)
            .map_err(authority_failure)
    })
    .map_err(|failure| failure.with_trusted_commit(&authority.trusted_commit))?;
    Ok(ResolveSuccess::new(
        stored.status,
        authority.trusted_commit,
        artifacts.build_id,
        stored.context,
        stored.manifest,
        artifacts.included_ids,
    ))
}

fn read_request(path: &Path) -> Result<ResolveRequest, ResolveFailure> {
    let file = File::open(path).map_err(|error| {
        request_failure(
            "request_unreadable",
            error.to_string(),
            vec![path.to_string_lossy().into_owned()],
        )
    })?;
    let mut bytes = Vec::new();
    file.take((MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            request_failure(
                "request_unreadable",
                error.to_string(),
                vec![path.to_string_lossy().into_owned()],
            )
        })?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(request_failure(
            "request_too_large",
            "Context Resolution request exceeds the Pilot size limit",
            vec![path.to_string_lossy().into_owned()],
        ));
    }
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Err(request_failure(
            "request_bom_forbidden",
            "Context Resolution request must not contain a UTF-8 BOM",
            vec![path.to_string_lossy().into_owned()],
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        request_failure(
            "invalid_request",
            error.to_string(),
            vec![path.to_string_lossy().into_owned()],
        )
    })
}

fn validate_request(request: &mut ResolveRequest) -> Result<(), ResolveFailure> {
    if request.schema != REQUEST_SCHEMA {
        return Err(request_failure(
            "unsupported_request_schema",
            format!("expected request schema `{REQUEST_SCHEMA}`"),
            Vec::new(),
        ));
    }
    if request.anchors.is_empty() && request.candidates.is_none() {
        return Err(request_failure(
            "empty_context_request",
            "provide at least one direct anchor or a Librarian candidate reference",
            Vec::new(),
        ));
    }
    if request.tokenizer_profile != TOKENIZER_PROFILE {
        return Err(request_failure(
            "unsupported_tokenizer_profile",
            format!("expected tokenizer profile `{TOKENIZER_PROFILE}`"),
            Vec::new(),
        ));
    }
    if request.max_tokens == 0 || request.max_tokens > MAX_TOKENS {
        return Err(request_failure(
            "invalid_token_budget",
            format!("max_tokens must be between 1 and {MAX_TOKENS}"),
            Vec::new(),
        ));
    }
    if request.anchors.len() > MAX_ANCHORS {
        return Err(request_failure(
            "too_many_anchors",
            format!("at most {MAX_ANCHORS} direct anchors are supported"),
            Vec::new(),
        ));
    }
    let mut seen = BTreeSet::<(String, String)>::new();
    for anchor in &mut request.anchors {
        let value = anchor.value().trim().to_owned();
        if value.is_empty() || value.len() > MAX_ANCHOR_BYTES {
            return Err(request_failure(
                "invalid_anchor",
                "anchor values must be non-empty and within the Pilot size limit",
                Vec::new(),
            ));
        }
        if matches!(anchor, Anchor::KnowledgeId { .. }) && !candidate::semantic_id(&value) {
            return Err(request_failure(
                "invalid_anchor",
                "KnowledgeId anchors must use lowercase semantic ID syntax",
                Vec::new(),
            ));
        }
        if !seen.insert((anchor.kind().to_owned(), value)) {
            return Err(request_failure(
                "duplicate_anchor",
                "identical trimmed anchors must not be repeated",
                Vec::new(),
            ));
        }
        match anchor {
            Anchor::KnowledgeId { value } | Anchor::Path { value } | Anchor::Symbol { value } => {
                *value = value.trim().to_owned()
            },
        }
    }
    request.anchors.sort();
    if let Some(reference) = &request.candidates
        && (reference.path.is_empty() || reference.path.len() > MAX_PATH_BYTES)
    {
        return Err(request_failure(
            "invalid_candidate_path",
            "candidate path must be non-empty and within the Pilot size limit",
            vec![reference.path.clone()],
        ));
    }
    Ok(())
}

fn authority_failure(failure: AuthorityFailure) -> ResolveFailure {
    let code = failure
        .diagnostics
        .first()
        .map_or("authority_invalid", |diagnostic| diagnostic.code.as_str());
    let message = failure.diagnostics.first().map_or_else(
        || "trusted authority is invalid".to_owned(),
        |diagnostic| diagnostic.message.clone(),
    );
    let affected_ids = failure
        .diagnostics
        .iter()
        .flat_map(|diagnostic| diagnostic.affected_ids.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let affected_paths = failure
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.path.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    ResolveFailure::new(
        failure.trusted_commit,
        code,
        message,
        failure.retryable,
        affected_ids,
        affected_paths,
        "repair trusted authority or retry after concurrent changes settle",
    )
}

fn request_failure(code: &str, message: impl Into<String>, paths: Vec<String>) -> ResolveFailure {
    ResolveFailure::new(
        None,
        code,
        message,
        false,
        Vec::new(),
        paths,
        "correct the Context Resolution request and retry",
    )
}
