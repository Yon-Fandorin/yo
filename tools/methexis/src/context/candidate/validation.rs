//! Closed independent validation of Librarian candidate-set JSON.

use std::{collections::BTreeSet, path::Path};

use super::capture::safe_relative;
use crate::context::{
    hash::{StableHasher, valid},
    wire::{Anchor, CANDIDATE_SCHEMA, Candidate, CandidateReason, CandidateSet, ResolveFailure},
};

const RESULT_ID_DOMAIN: &[u8] = b"librarian.candidate-set/v1alpha1";

pub(super) fn validate(set: &CandidateSet, path: &str) -> Result<(), ResolveFailure> {
    if set.schema != CANDIDATE_SCHEMA || !set.ok {
        return Err(invalid(
            path,
            "candidate result schema or success marker is invalid",
        ));
    }
    if !valid(&set.candidate_set_id)
        || !valid(&set.request_hash)
        || !valid(&set.catalog_hash)
        || !valid_compiler(&set.compiler)
    {
        return Err(invalid(
            path,
            "candidate result identity fields are invalid",
        ));
    }
    let mut candidate_ids = BTreeSet::new();
    let mut previous: Option<&Candidate> = None;
    for candidate in &set.candidates {
        validate_candidate(candidate, path)?;
        if !candidate_ids.insert(candidate.id.as_str()) {
            return Err(invalid(path, "candidate IDs must be unique"));
        }
        if let Some(previous) = previous
            && (previous.score < candidate.score
                || (previous.score == candidate.score && previous.id >= candidate.id))
        {
            return Err(invalid(
                path,
                "candidates must be ordered by descending score then ascending KnowledgeId",
            ));
        }
        previous = Some(candidate);
    }
    validate_unresolved(&set.unresolved_anchors, path)?;
    let candidate_bytes = serde_json::to_vec(&set.candidates)
        .expect("closed candidate structs serialize deterministically");
    let mut identity = StableHasher::new(RESULT_ID_DOMAIN);
    identity.part(b"request_hash", set.request_hash.as_bytes());
    identity.part(b"catalog_hash", set.catalog_hash.as_bytes());
    identity.part(b"compiler", set.compiler.as_bytes());
    identity.part(b"candidates", &candidate_bytes);
    if identity.finish() != set.candidate_set_id {
        return Err(invalid(
            path,
            "candidate_set_id does not match candidate bytes",
        ));
    }
    Ok(())
}

fn validate_candidate(candidate: &Candidate, path: &str) -> Result<(), ResolveFailure> {
    if !semantic_id(&candidate.id)
        || !candidate.path.starts_with("methexis/knowledge/")
        || !candidate.path.ends_with(".md")
        || !safe_relative(Path::new(&candidate.path))
        || candidate.score == 0
        || candidate.reasons.is_empty()
    {
        return Err(invalid(path, "candidate fields are invalid"));
    }
    let reasons = candidate.reasons.iter().cloned().collect::<BTreeSet<_>>();
    if reasons.len() != candidate.reasons.len()
        || reasons.iter().ne(candidate.reasons.iter())
        || candidate.reasons.iter().any(|reason| !valid_reason(reason))
    {
        return Err(invalid(
            path,
            "candidate reasons are invalid, duplicated, or noncanonical",
        ));
    }
    let total = candidate
        .reasons
        .iter()
        .try_fold(0_u64, |total, reason| total.checked_add(reason.score()))
        .ok_or_else(|| invalid(path, "candidate reason score overflow"))?;
    if total != candidate.score {
        return Err(invalid(
            path,
            "candidate score must equal the sum of reason scores",
        ));
    }
    Ok(())
}

fn valid_reason(reason: &CandidateReason) -> bool {
    let searchable_field = |field: &str| {
        matches!(
            field,
            "id" | "path"
                | "title"
                | "body"
                | "projection"
                | "applies_to"
                | "constrained_by"
                | "depends_on"
                | "supersedes"
                | "validated_by"
        )
    };
    match reason {
        CandidateReason::Anchor {
            anchor_kind,
            value,
            field,
            score,
        } => {
            matches!(
                (anchor_kind.as_str(), field.as_str()),
                ("knowledge_id", "id") | ("path", "path" | "applies_to") | ("symbol", "applies_to")
            ) && !value.trim().is_empty()
                && *score > 0
        },
        CandidateReason::ExactQuery { field, score } => field == "id" && *score > 0,
        CandidateReason::QueryPhrase { field, score } => searchable_field(field) && *score > 0,
        CandidateReason::QueryToken { field, term, score } => {
            searchable_field(field)
                && !term.is_empty()
                && term.chars().all(char::is_alphanumeric)
                && *term == term.to_lowercase()
                && *score > 0
        },
        CandidateReason::Relation { via, score } => semantic_id(via) && *score > 0,
    }
}

fn validate_unresolved(anchors: &[Anchor], path: &str) -> Result<(), ResolveFailure> {
    let mut seen = BTreeSet::new();
    for anchor in anchors {
        let value = anchor.value().trim();
        if value.is_empty() || !seen.insert((anchor.kind(), value)) {
            return Err(invalid(
                path,
                "unresolved anchors are invalid or duplicated",
            ));
        }
    }
    Ok(())
}

fn valid_compiler(value: &str) -> bool {
    value.strip_prefix("librarian/").is_some_and(|version| {
        !version.is_empty()
            && version.len() <= 128
            && version
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
    })
}

pub(super) fn semantic_id(value: &str) -> bool {
    !value.is_empty() && value.split('.').all(valid_segment)
}

fn valid_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_lowercase)
        && bytes
            .last()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
        && !segment.contains("--")
}

fn invalid(path: &str, message: &str) -> ResolveFailure {
    ResolveFailure::new(
        None,
        "invalid_candidate_set",
        message,
        false,
        Vec::new(),
        vec![path.to_owned()],
        "correct or regenerate the Librarian candidate result and retry",
    )
}
