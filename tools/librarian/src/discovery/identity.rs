//! Stable request and candidate-set identities.

use crate::{
    error::DiscoveryError,
    hash::StableHasher,
    wire::{COMPILER, Candidate, DiscoveryRequest},
};

const REQUEST_HASH_DOMAIN: &[u8] = b"librarian.discovery-request/v1alpha1";
const RESULT_ID_DOMAIN: &[u8] = b"librarian.candidate-set/v1alpha1";

pub(super) fn request(request: &DiscoveryRequest) -> Result<String, DiscoveryError> {
    let bytes = serde_json::to_vec(request).map_err(|error| {
        DiscoveryError::request(
            "request_serialization_failed",
            format!("cannot canonicalize request: {error}"),
        )
    })?;
    let mut hasher = StableHasher::new(REQUEST_HASH_DOMAIN);
    hasher.part(b"request", &bytes);
    Ok(hasher.finish())
}

pub(super) fn candidate_set(
    request_hash: &str,
    catalog_hash: &str,
    candidates: &[Candidate],
) -> Result<String, DiscoveryError> {
    let bytes = serde_json::to_vec(candidates).map_err(|error| {
        DiscoveryError::request(
            "result_serialization_failed",
            format!("cannot identify candidate set: {error}"),
        )
    })?;
    let mut hasher = StableHasher::new(RESULT_ID_DOMAIN);
    hasher.part(b"request_hash", request_hash.as_bytes());
    hasher.part(b"catalog_hash", catalog_hash.as_bytes());
    hasher.part(b"compiler", COMPILER.as_bytes());
    hasher.part(b"candidates", &bytes);
    Ok(hasher.finish())
}
