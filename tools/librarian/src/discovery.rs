//! Deterministic lexical retrieval over an immutable catalog.

mod identity;
mod ranking;
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use crate::{
    catalog::Catalog,
    error::DiscoveryError,
    wire::{COMPILER, CandidateSet, DiscoveryRequest, REQUEST_SCHEMA, RESULT_SCHEMA},
};

pub(crate) fn discover(
    request: DiscoveryRequest,
    catalog: &Catalog,
) -> Result<CandidateSet, DiscoveryError> {
    validate_request(&request)?;
    let request_hash = identity::request(&request)?;
    let (candidates, unresolved_anchors) = ranking::rank(&request, catalog);
    let candidate_set_id = identity::candidate_set(&request_hash, &catalog.hash, &candidates)?;
    Ok(CandidateSet {
        schema: RESULT_SCHEMA,
        ok: true,
        candidate_set_id,
        request_hash,
        catalog_hash: catalog.hash.clone(),
        compiler: COMPILER,
        candidates,
        unresolved_anchors,
        truncated: 0,
    })
}

pub(crate) fn validate_request(request: &DiscoveryRequest) -> Result<(), DiscoveryError> {
    if request.schema != REQUEST_SCHEMA {
        return Err(DiscoveryError::request(
            "unsupported_request_schema",
            format!("expected request schema `{REQUEST_SCHEMA}`"),
        ));
    }
    let has_query = request
        .query
        .as_deref()
        .is_some_and(|query| !query.trim().is_empty());
    if request
        .anchors
        .iter()
        .any(|anchor| anchor.value().trim().is_empty())
    {
        return Err(DiscoveryError::request(
            "invalid_request",
            "anchor values must not be empty",
        ));
    }
    if request
        .anchors
        .iter()
        .map(|anchor| (anchor.kind(), anchor.value().trim()))
        .collect::<BTreeSet<_>>()
        .len()
        != request.anchors.len()
    {
        return Err(DiscoveryError::request(
            "duplicate_anchor",
            "identical anchors must not be repeated",
        ));
    }
    if !has_query && request.anchors.is_empty() {
        return Err(DiscoveryError::request(
            "empty_discovery_request",
            "provide a non-empty query or at least one anchor",
        ));
    }
    Ok(())
}
