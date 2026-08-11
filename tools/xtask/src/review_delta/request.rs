use super::{
    capture::require_hash,
    model::{DELIVERY_PROFILE, REQUEST_SCHEMA, Request, TOKENIZER_PROFILE},
};
use crate::{review_packet::VerifiedReview, review_protocol::require_commit, slice_contract};

pub(super) fn validate_request(request: &Request) -> Result<(), String> {
    if request.schema != REQUEST_SCHEMA {
        return Err(format!("expected request schema `{REQUEST_SCHEMA}`"));
    }
    if request.delivery_profile != DELIVERY_PROFILE {
        return Err(format!("expected delivery profile `{DELIVERY_PROFILE}`"));
    }
    if request.tokenizer_profile != TOKENIZER_PROFILE {
        return Err(format!("expected tokenizer profile `{TOKENIZER_PROFILE}`"));
    }
    if request.max_managed_payload_tokens == 0 {
        return Err("managed payload token budget must be positive".to_owned());
    }
    require_hash(&request.prior_manifest_hash, "prior manifest hash")?;
    require_hash(&request.prior_findings_hash, "prior findings hash")?;
    if request.finding_dispositions.is_empty() {
        return Err("at least one finding disposition is required".to_owned());
    }
    if request.affected_validation_evidence.is_empty() {
        return Err(
            "at least one affected validation evidence item is required for a replacement candidate"
                .to_owned(),
        );
    }
    Ok(())
}

pub(super) fn validate_prior(
    prior: &VerifiedReview,
    bound: &slice_contract::BoundSlice,
    replacement_candidate: &str,
) -> Result<(), String> {
    for (value, label) in [
        (prior.base_commit.as_str(), "prior base"),
        (prior.candidate_commit.as_str(), "prior candidate"),
        (prior.trusted_commit.as_str(), "prior trusted commit"),
        (replacement_candidate, "replacement candidate"),
    ] {
        require_commit(value, label)?;
    }
    if prior.base_commit != bound.base {
        return Err("prior review base differs from the bound Slice base".to_owned());
    }
    Ok(())
}
