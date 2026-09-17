use std::{collections::BTreeSet, path::Path};

use super::model::{ManifestHeader, ReviewKind, Route};
use crate::{
    bounded_file, review_packet,
    review_protocol::{digest, resolve_input_path},
};

pub(super) const REQUEST_LIMIT: usize = 64 * 1024;
pub(super) const AUTHORIZATION_LIMIT: usize = 64 * 1024;
pub(super) const DELIVERY_RECEIPT_LIMIT: usize = 64 * 1024;
pub(super) const MANIFEST_LIMIT: usize = 8 * 1024 * 1024;
pub(super) const PACKET_LIMIT: usize = 32 * 1024 * 1024;
pub(super) const MAX_ROUTES: usize = 16;
pub(super) const MAX_ROUTE_TOKEN_BYTES: usize = 128;
pub(super) const MAX_SESSION_ID_BYTES: usize = 256;
pub(super) const MAX_AUTHORIZED_TOKENS: usize = 1_000_000;
pub(super) const MAX_REVIEW_CHAIN_REQUESTS: usize = 64;

#[derive(Debug)]
pub(super) struct ReviewClassification {
    pub(super) kind: ReviewKind,
    pub(super) finding_resolution_request_index: usize,
    pub(super) prior: Option<PriorReview>,
}

#[derive(Debug)]
pub(super) struct PriorReview {
    pub(super) review_id: String,
    pub(super) packet_hash: String,
}

pub(super) fn classify_review_kind(
    repository: &Path,
    manifest: &ManifestHeader,
) -> Result<ReviewClassification, String> {
    if review_packet::is_original_manifest_schema(&manifest.schema) {
        return Ok(ReviewClassification {
            kind: ReviewKind::Original,
            finding_resolution_request_index: 0,
            prior: None,
        });
    }

    let mut seen = BTreeSet::new();
    let mut finding_resolution_request_index = 0usize;
    let mut immediate_prior = None;
    let mut next_prior = manifest
        .inputs
        .as_ref()
        .and_then(|inputs| inputs.prior_manifest.as_ref())
        .map(|prior| (prior.path.clone(), prior.hash.clone()));
    loop {
        finding_resolution_request_index = finding_resolution_request_index
            .checked_add(1)
            .ok_or_else(|| "review continuation request count is exhausted".to_owned())?;
        if finding_resolution_request_index >= MAX_REVIEW_CHAIN_REQUESTS {
            return Err(format!(
                "review continuation chain exceeds the {}-request safety limit",
                MAX_REVIEW_CHAIN_REQUESTS - 1
            ));
        }
        let (prior_path_value, prior_hash) = next_prior
            .take()
            .ok_or_else(|| "finding-resolution manifest has no prior_manifest".to_owned())?;
        require_sha256(&prior_hash, "prior manifest hash")?;
        if !seen.insert(prior_hash.clone()) {
            return Err("review continuation chain contains a cycle".to_owned());
        }
        let prior_path = resolve_input_path(repository, &prior_path_value);
        let prior_bytes = bounded_file::read_regular(
            &prior_path,
            MANIFEST_LIMIT,
            "prior published review manifest",
        )?;
        require_exact_hash(&prior_hash, &prior_bytes, "prior published review manifest")?;
        let prior_manifest: ManifestHeader = serde_json::from_slice(&prior_bytes)
            .map_err(|error| format!("invalid prior published review manifest: {error}"))?;
        if immediate_prior.is_none() {
            let review_id = prior_manifest
                .review_id
                .as_ref()
                .or(prior_manifest.review_delta_id.as_ref())
                .ok_or_else(|| "prior review manifest has no review identity".to_owned())?
                .clone();
            require_sha256(&review_id, "prior review identity")?;
            require_sha256(&prior_manifest.packet.hash, "prior packet hash")?;
            immediate_prior = Some(PriorReview {
                review_id,
                packet_hash: prior_manifest.packet.hash.clone(),
            });
        }
        if review_packet::is_original_manifest_schema(&prior_manifest.schema) {
            break;
        }
        next_prior = prior_manifest
            .inputs
            .as_ref()
            .and_then(|inputs| inputs.prior_manifest.as_ref())
            .map(|prior| (prior.path.clone(), prior.hash.clone()));
    }
    Ok(ReviewClassification {
        kind: ReviewKind::FindingResolution,
        finding_resolution_request_index,
        prior: immediate_prior,
    })
}
pub(super) fn validate_route(route: &Route) -> Result<(), String> {
    compact_token(&route.provider, MAX_ROUTE_TOKEN_BYTES, "route provider")?;
    compact_token(&route.account, MAX_ROUTE_TOKEN_BYTES, "route account")?;
    compact_token(&route.model, MAX_ROUTE_TOKEN_BYTES, "route model")
}

pub(super) fn compact_path(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 4096 || value.contains('\0') {
        Err(format!(
            "{label} must be a non-empty path of at most 4096 bytes"
        ))
    } else {
        Ok(())
    }
}

pub(super) fn compact_token(value: &str, max: usize, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > max
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !byte.is_ascii_whitespace())
    {
        Err(format!(
            "{label} must be a non-empty visible ASCII token of at most {max} bytes"
        ))
    } else {
        Ok(())
    }
}

pub(super) fn require_sha256(value: &str, label: &str) -> Result<(), String> {
    let valid = value.strip_prefix("sha256:").is_some_and(|suffix| {
        suffix.len() == 64
            && suffix
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    });
    if valid {
        Ok(())
    } else {
        Err(format!("{label} must be a canonical SHA-256 identity"))
    }
}

pub(super) fn require_exact_hash(expected: &str, bytes: &[u8], label: &str) -> Result<(), String> {
    require_sha256(expected, &format!("{label} hash"))?;
    let actual = digest(bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "{label} hash mismatch: expected {expected}, found {actual}"
        ))
    }
}
