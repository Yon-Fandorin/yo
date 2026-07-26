//! Eligibility observations and atomic-bundle blocking reasons.

use std::collections::BTreeSet;

use super::{ApprovalObservation, UnitObservation};
use crate::{checkpoint::ContextAuthority, source::Eligibility};

pub(super) fn blocked_units(
    authority: &ContextAuthority,
    bundle: &BTreeSet<String>,
) -> Vec<String> {
    bundle
        .iter()
        .filter(|id| !authority.active.contains(*id))
        .cloned()
        .collect()
}

pub(super) fn blocked_reason(authority: &ContextAuthority, blocked: &[String]) -> &'static str {
    if blocked.iter().any(|id| {
        authority
            .freshness
            .get(id)
            .is_some_and(|state| state.eligibility == Eligibility::Invalid)
    }) {
        "bundle_invalid"
    } else if blocked.iter().any(|id| {
        authority
            .freshness
            .get(id)
            .is_some_and(|state| state.eligibility == Eligibility::Stale)
    }) {
        "bundle_stale"
    } else {
        "bundle_inactive"
    }
}

pub(super) fn observation(authority: &ContextAuthority, id: String) -> UnitObservation {
    let (eligibility, evidence) = authority.freshness.get(&id).map_or_else(
        || ("inactive", Vec::new()),
        |state| (state.eligibility.as_str(), state.evidence.clone()),
    );
    UnitObservation {
        approval: authority
            .approval_evidence
            .get(&id)
            .map(|evidence| ApprovalObservation {
                projection_hash: evidence.projection_hash.clone(),
                projection_profile: evidence.projection_profile.clone(),
                projection_compiler: evidence.projection_compiler.clone(),
                approval_hash: evidence.approval_hash.clone(),
            }),
        id,
        eligibility,
        evidence,
    }
}
