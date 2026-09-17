use std::path::{Path, PathBuf};

use super::{
    super::{
        VerifiedDeliveryRoute,
        model::{
            DELEGATED_DELIVERY_RECEIPT_SCHEMA, DELEGATED_DELIVERY_RECEIPT_SCHEMA_V1_ALPHA2,
            DelegatedDeliveryReceipt, DelegatedRequest, Session,
        },
        validation::{
            DELIVERY_RECEIPT_LIMIT, MAX_SESSION_ID_BYTES, ReviewClassification, compact_token,
            require_exact_hash, require_sha256,
        },
    },
    target::{require_execution_profile, validate_host},
};
use crate::{
    bounded_file, grok_outer_sandbox, review_packet::VerifiedReview,
    review_protocol::resolve_input_path,
};

const MAX_HOST_REQUEST_ID_BYTES: usize = 256;

pub(super) struct CapturedHostReceipt {
    pub(super) path: PathBuf,
    pub(super) bytes: Vec<u8>,
    pub(super) host_request_id: String,
    pub(super) execution_isolation: Option<String>,
}
pub(super) fn capture_prior_delivery(
    repository: &Path,
    request: &DelegatedRequest,
    classification: &ReviewClassification,
) -> Result<Option<CapturedHostReceipt>, String> {
    let Some(prior) = &classification.prior else {
        if request.prior_delivery.is_some() {
            return Err("an original review must not name prior_delivery evidence".to_owned());
        }
        return Ok(None);
    };
    let reference = request
        .prior_delivery
        .as_ref()
        .ok_or_else(|| "a finding-resolution review requires prior_delivery evidence".to_owned())?;
    let path = resolve_input_path(repository, &reference.path);
    let bytes = bounded_file::read_regular(
        &path,
        DELIVERY_RECEIPT_LIMIT,
        "prior delegated delivery receipt",
    )?;
    require_exact_hash(&reference.hash, &bytes, "prior delegated delivery receipt")?;
    let receipt = parse_delivery_receipt(&bytes, "prior delegated delivery receipt")?;
    if receipt.review_id != prior.review_id || receipt.packet_hash != prior.packet_hash {
        return Err("prior delegated receipt does not match the original review packet".to_owned());
    }
    if receipt.target != request.target || receipt.execution_profile != request.execution_profile {
        return Err(
            "finding-resolution delegated target differs from the original delivery target"
                .to_owned(),
        );
    }
    let Session::Resume { id } = &request.session else {
        return Err(
            "a finding-resolution review requires the existing reviewer Session".to_owned(),
        );
    };
    if receipt.session_id != *id {
        return Err(
            "finding-resolution Session differs from the original delivery Session".to_owned(),
        );
    }
    Ok(Some(CapturedHostReceipt {
        path,
        bytes,
        host_request_id: receipt.host_request_id,
        execution_isolation: receipt.execution_isolation,
    }))
}

pub(super) fn verify_completed_delivery_bytes(
    bytes: &[u8],
    review: &VerifiedReview,
) -> Result<VerifiedDeliveryRoute, String> {
    let receipt = parse_delivery_receipt(bytes, "delegated external review delivery receipt")?;
    if receipt.review_id != review.review_id || receipt.packet_hash != review.packet_hash {
        return Err(
            "delegated external review delivery receipt does not match the reviewed packet"
                .to_owned(),
        );
    }
    Ok(VerifiedDeliveryRoute::Delegated {
        host: receipt.target.host().to_owned(),
        session_id: receipt.session_id,
    })
}
pub(super) fn parse_delivery_receipt(
    bytes: &[u8],
    label: &str,
) -> Result<DelegatedDeliveryReceipt, String> {
    let receipt: DelegatedDeliveryReceipt =
        serde_json::from_slice(bytes).map_err(|error| format!("invalid {label}: {error}"))?;
    if !matches!(
        receipt.schema.as_str(),
        DELEGATED_DELIVERY_RECEIPT_SCHEMA | DELEGATED_DELIVERY_RECEIPT_SCHEMA_V1_ALPHA2
    ) {
        return Err(format!(
            "unsupported delegated delivery receipt schema `{}`; expected `{DELEGATED_DELIVERY_RECEIPT_SCHEMA}` or `{DELEGATED_DELIVERY_RECEIPT_SCHEMA_V1_ALPHA2}`",
            receipt.schema
        ));
    }
    require_sha256(&receipt.review_id, "delivery ReviewId")?;
    require_sha256(&receipt.packet_hash, "delivery packet hash")?;
    validate_host(receipt.target.host())?;
    require_execution_profile(&receipt.execution_profile)?;
    match (
        receipt.schema.as_str(),
        receipt.execution_isolation.as_deref(),
    ) {
        (DELEGATED_DELIVERY_RECEIPT_SCHEMA, None) => {},
        (DELEGATED_DELIVERY_RECEIPT_SCHEMA_V1_ALPHA2, Some(isolation))
            if receipt.target.host() == "grok"
                && matches!(
                    isolation,
                    grok_outer_sandbox::NATIVE_SANDBOX_REVIEW_PROFILE
                        | grok_outer_sandbox::OUTER_SANDBOX_REVIEW_PROFILE
                ) => {},
        (DELEGATED_DELIVERY_RECEIPT_SCHEMA, Some(_)) => {
            return Err(
                "delegated delivery receipt v1alpha1 must not name execution isolation".to_owned(),
            );
        },
        (DELEGATED_DELIVERY_RECEIPT_SCHEMA_V1_ALPHA2, _) => {
            return Err(
                "delegated delivery receipt v1alpha2 requires an exact Grok execution isolation"
                    .to_owned(),
            );
        },
        _ => unreachable!("validated delegated receipt schema"),
    }
    compact_token(
        &receipt.session_id,
        MAX_SESSION_ID_BYTES,
        "delivery session id",
    )?;
    compact_token(
        &receipt.host_request_id,
        MAX_HOST_REQUEST_ID_BYTES,
        "delivery host request id",
    )?;
    if receipt.host_request_count != 1 {
        return Err("delegated delivery receipt must record exactly one host request".to_owned());
    }
    Ok(receipt)
}
