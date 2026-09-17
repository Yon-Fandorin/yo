use std::{collections::BTreeSet, path::Path};

use super::{
    bounds::{require_exact_hash, require_sha256},
    model::{CorrectionRequest, CorrectionResult},
    parse::inspect,
};
use crate::{
    bounded_file, review_delta, review_egress,
    review_protocol::{digest, resolve_input_path},
};

const CORRECTION_REQUEST_SCHEMA: &str =
    "yo.slice-review-result-correction-preflight-request/v1alpha1";
const CORRECTION_RESULT_SCHEMA: &str =
    "yo.slice-review-result-correction-preflight-result/v1alpha1";
const CORRECTION_REQUEST_LIMIT: usize = 64 * 1024;
const REVIEW_RESULT_LIMIT: usize = 64 * 1024;

pub(crate) fn correction_preflight(repository: &Path, request_path: &Path) -> Result<(), String> {
    let request_bytes = bounded_file::read_regular(
        request_path,
        CORRECTION_REQUEST_LIMIT,
        "review-result correction preflight request",
    )?;
    let request: CorrectionRequest = serde_json::from_slice(&request_bytes)
        .map_err(|error| format!("invalid review-result correction preflight request: {error}"))?;
    if request.schema != CORRECTION_REQUEST_SCHEMA {
        return Err(format!(
            "unsupported review-result correction preflight schema `{}`; expected `{CORRECTION_REQUEST_SCHEMA}`",
            request.schema
        ));
    }
    for (value, label) in [
        (&request.manifest_hash, "manifest_hash"),
        (&request.delivery_receipt_hash, "delivery_receipt_hash"),
        (&request.review_result_hash, "review_result_hash"),
    ] {
        require_sha256(value, label)?;
    }
    let manifest_path = resolve_input_path(repository, &request.manifest_path);
    let receipt_path = resolve_input_path(repository, &request.delivery_receipt_path);
    let result_path = resolve_input_path(repository, &request.review_result_path);
    let review = review_delta::verify_chain_head(
        repository,
        &manifest_path,
        &request.manifest_hash,
        &mut BTreeSet::new(),
        0,
    )?;
    let receipt_bytes = bounded_file::read_regular(
        &receipt_path,
        CORRECTION_REQUEST_LIMIT,
        "review delivery receipt",
    )?;
    require_exact_hash(
        &receipt_bytes,
        &request.delivery_receipt_hash,
        "review delivery receipt",
    )?;
    let route = review_egress::verify_any_completed_delivery(repository, &receipt_path, &review)?;
    let result_bytes = bounded_file::read_regular(
        &result_path,
        REVIEW_RESULT_LIMIT,
        "structured review result",
    )?;
    require_exact_hash(
        &result_bytes,
        &request.review_result_hash,
        "structured review result",
    )?;
    let inspected = inspect(&result_bytes, &review.review_lenses)?;
    let review_id_drift = inspected.review_id != review.review_id;
    let candidate_drift = inspected.candidate_commit != review.candidate_commit;
    if !review_id_drift && !candidate_drift {
        return Err("structured review result already has the exact identity envelope".to_owned());
    }
    let (route, session_id) = match route {
        review_egress::VerifiedDeliveryRoute::Managed {
            provider,
            model,
            session_id,
        } => (format!("managed/{provider}/{model}"), session_id),
        review_egress::VerifiedDeliveryRoute::Delegated { host, session_id } => {
            (format!("delegated/{host}"), session_id)
        },
    };
    let result = CorrectionResult {
        schema: CORRECTION_RESULT_SCHEMA,
        ok: true,
        status: "eligible_identity_envelope_only",
        next_action: "request_exact_same_session_envelope_correction_once",
        provider_requests: 0,
        expected_review_id: review.review_id,
        observed_review_id: inspected.review_id,
        expected_candidate_commit: review.candidate_commit,
        observed_candidate_commit: inspected.candidate_commit,
        session_id,
        route,
        immutable_result_hash: digest(&result_bytes),
    };
    println!(
        "{}",
        serde_json::to_string(&result)
            .map_err(|error| format!("cannot encode correction preflight result: {error}"))?
    );
    Ok(())
}
