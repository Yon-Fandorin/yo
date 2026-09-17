use std::{fs, path::Path};

use super::{
    super::super::{
        REQUEST_LIMIT,
        admission::evaluate_host_admission,
        artifact::{canonical_json, require_exact_file_hash},
        delegated,
        delegated_session::{observe_host_continuation, observe_host_session},
        model::{
            DELEGATED_CLAIM_SCHEMA, DELEGATED_CLAIM_SCHEMA_V1_ALPHA2,
            DELEGATED_CLAIM_SCHEMA_V1_ALPHA3, DELEGATED_CONTINUATION_CLAIM_SCHEMA,
            DELEGATED_CONTINUATION_CLAIM_SCHEMA_V1_ALPHA2,
            DELEGATED_CONTINUATION_CLAIM_SCHEMA_V1_ALPHA3,
            DELEGATED_CONTINUATION_REQUEST_SCHEMA_V1_ALPHA2, DELEGATED_DELIVERY_RECEIPT_SCHEMA,
            DELEGATED_DELIVERY_RECEIPT_SCHEMA_V1_ALPHA2, DELEGATED_REQUEST_SCHEMA_V1_ALPHA2,
            DelegatedContinuationRequest, DelegatedDeliveryReceipt, DelegatedRequest,
        },
        workspace::{output_directory, shared_path},
    },
    observation, request,
};
use crate::{bounded_file, review_egress, review_protocol::digest};

const FINALIZATION_SCHEMA: &str = "yo.external-review-delegated-delivery-finalization/v1alpha1";

pub(super) struct Finalized {
    pub(super) created: bool,
    pub(super) request_id: String,
    pub(super) review_id: String,
    pub(super) candidate_commit: String,
    pub(super) session_id: String,
    pub(super) host_request_id: String,
    pub(super) delivery_path: String,
    pub(super) delivery_hash: String,
}

pub(super) fn finalize_original(
    repository: &Path,
    request: DelegatedRequest,
) -> Result<Finalized, String> {
    let egress_path = shared_path(repository, &request.egress_request_path)?;
    require_exact_file_hash(
        &egress_path,
        &request.egress_request_hash,
        REQUEST_LIMIT,
        "delegated Slice review egress request",
    )?;
    let delivery = review_egress::authorize_host_delivery(repository, &egress_path)?;
    if delivery.review_kind != "original" || !delivery.fresh_session {
        return Err(
            "delegated original finalization requires one fresh original review".to_owned(),
        );
    }
    let strong = request.schema == DELEGATED_REQUEST_SCHEMA_V1_ALPHA2;
    let admission = evaluate_host_admission(
        repository,
        &request.admission_request_path,
        &request.admission_request_hash,
        &delivery,
        strong,
    )?;
    let execution_isolation = admission.delegated_execution_isolation();
    let output = output_directory(repository, &request.output_directory)?;
    let claim = observation::read_json(&output.join("claim.json"), "delegated delivery claim")?;
    request::validate_claim(
        &claim,
        &delivery,
        &request.admission_request_hash,
        if execution_isolation.is_some() {
            DELEGATED_CLAIM_SCHEMA_V1_ALPHA3
        } else if strong {
            DELEGATED_CLAIM_SCHEMA_V1_ALPHA2
        } else {
            DELEGATED_CLAIM_SCHEMA
        },
        false,
        None,
        execution_isolation,
    )?;
    observation::validate_process_and_artifacts(&output, &delivery.request_id, false)?;
    let observation =
        observe_host_session(&output.join("sessions"), &delivery.packet_bytes, &delivery);
    if let Some(failure) = observation.failure {
        return Err(format!(
            "delegated delivery is not recoverable without another host request: {failure}"
        ));
    }
    let session_id = observation
        .session_id
        .ok_or_else(|| "recovered delegated Session has no identity".to_owned())?;
    let host_request_id = observation
        .host_request_id
        .ok_or_else(|| "recovered delegated request has no identity".to_owned())?;
    if observation.host_request_count != 1 {
        return Err(
            "recovered delegated delivery must contain exactly one host request".to_owned(),
        );
    }
    publish_recovery(
        &output,
        &delivery,
        &session_id,
        &host_request_id,
        None,
        execution_isolation,
    )
}

pub(super) fn finalize_continuation(
    repository: &Path,
    request: DelegatedContinuationRequest,
) -> Result<Finalized, String> {
    let preflight_path = shared_path(repository, &request.preflight_request_path)?;
    require_exact_file_hash(
        &preflight_path,
        &request.preflight_request_hash,
        REQUEST_LIMIT,
        "delegated continuation preflight request",
    )?;
    let preflight_bytes = bounded_file::read_regular(
        &preflight_path,
        REQUEST_LIMIT,
        "delegated continuation preflight request",
    )?;
    let preflight = request::read_continuation_preflight(&preflight_bytes)?;
    let egress_path = shared_path(repository, &preflight.egress_request_path)?;
    require_exact_file_hash(
        &egress_path,
        &preflight.egress_request_hash,
        REQUEST_LIMIT,
        "delegated Slice review egress request",
    )?;
    let delivery = review_egress::authorize_host_delivery(repository, &egress_path)?;
    if delivery.review_kind != "finding_resolution" || delivery.fresh_session {
        return Err(
            "delegated continuation finalization requires one resumed finding resolution"
                .to_owned(),
        );
    }
    let strong = request.schema == DELEGATED_CONTINUATION_REQUEST_SCHEMA_V1_ALPHA2;
    let admission = evaluate_host_admission(
        repository,
        &request.admission_request_path,
        &request.admission_request_hash,
        &delivery,
        strong,
    )?;
    let execution_isolation = admission.delegated_execution_isolation();
    let output = output_directory(repository, &request.output_directory)?;
    let claim = observation::read_json(&output.join("claim.json"), "delegated continuation claim")?;
    let prior_anchor = required_u64(&claim, "continuation_anchor_sequence")?;
    let binding_epoch = required_u64(&claim, "binding_epoch")?;
    let preflight_id = digest(&preflight_bytes);
    request::validate_claim(
        &claim,
        &delivery,
        &request.admission_request_hash,
        if execution_isolation.is_some() {
            DELEGATED_CONTINUATION_CLAIM_SCHEMA_V1_ALPHA3
        } else if strong {
            DELEGATED_CONTINUATION_CLAIM_SCHEMA_V1_ALPHA2
        } else {
            DELEGATED_CONTINUATION_CLAIM_SCHEMA
        },
        true,
        Some(&preflight_id),
        execution_isolation,
    )?;
    observation::validate_process_and_artifacts(&output, &delivery.request_id, true)?;
    let session_root = fs::canonicalize(&preflight.session_repository_path).map_err(|error| {
        format!(
            "cannot resolve delegated continuation Session repository {}: {error}",
            preflight.session_repository_path
        )
    })?;
    let observation = observe_host_continuation(
        &session_root,
        &delivery.packet_bytes,
        &delivery,
        prior_anchor,
        binding_epoch,
    );
    if let Some(failure) = observation.failure {
        return Err(format!(
            "delegated continuation is not recoverable without another host request: {failure}"
        ));
    }
    if observation.host_request_count != 1 {
        return Err(
            "recovered delegated continuation must contain exactly one new host request".to_owned(),
        );
    }
    let host_request_id = observation
        .host_request_id
        .ok_or_else(|| "recovered delegated continuation has no request identity".to_owned())?;
    let anchor = observation
        .continuation_anchor_sequence
        .ok_or_else(|| "recovered delegated continuation has no new Anchor".to_owned())?;
    let session_id = delivery
        .session_id
        .clone()
        .ok_or_else(|| "delegated continuation authorization has no Session".to_owned())?;
    publish_recovery(
        &output,
        &delivery,
        &session_id,
        &host_request_id,
        Some(anchor),
        execution_isolation,
    )
}

fn publish_recovery(
    output: &Path,
    delivery: &review_egress::AuthorizedHostDelivery,
    session_id: &str,
    host_request_id: &str,
    continuation_anchor_sequence: Option<u64>,
    execution_isolation: Option<&str>,
) -> Result<Finalized, String> {
    let receipt = DelegatedDeliveryReceipt {
        schema: if execution_isolation.is_some() {
            DELEGATED_DELIVERY_RECEIPT_SCHEMA_V1_ALPHA2
        } else {
            DELEGATED_DELIVERY_RECEIPT_SCHEMA
        },
        review_id: &delivery.review_id,
        packet_hash: &delivery.packet_hash,
        target: delegated::target(delivery),
        execution_profile: &delivery.execution_profile,
        execution_isolation,
        session_id,
        host_request_id,
        host_request_count: 1,
    };
    let delivery_bytes = canonical_json(&receipt)?;
    let delivery_path = output.join("delivery.json");
    let created = bounded_file::publish_new_or_exact(
        &delivery_path,
        &delivery_bytes,
        REQUEST_LIMIT,
        "recovered delegated delivery receipt",
    )?;
    let outcome_path = output.join("outcome.json");
    let outcome_bytes =
        bounded_file::read_regular(&outcome_path, REQUEST_LIMIT, "delegated delivery outcome")?;
    let finalization = canonical_json(&serde_json::json!({
        "schema": FINALIZATION_SCHEMA,
        "request_id": delivery.request_id,
        "outcome_hash": digest(&outcome_bytes),
        "delivery_receipt_hash": digest(&delivery_bytes),
        "session_id": session_id,
        "host_request_id": host_request_id,
        "continuation_anchor_sequence": continuation_anchor_sequence,
        "provider_requests": 0,
        "host_requests": 0
    }))?;
    bounded_file::publish_new_or_exact(
        &output.join("finalization.json"),
        &finalization,
        REQUEST_LIMIT,
        "delegated delivery finalization",
    )?;
    Ok(Finalized {
        created,
        request_id: delivery.request_id.clone(),
        review_id: delivery.review_id.clone(),
        candidate_commit: delivery.candidate_commit.clone(),
        session_id: session_id.to_owned(),
        host_request_id: host_request_id.to_owned(),
        delivery_path: delivery_path.to_string_lossy().into_owned(),
        delivery_hash: digest(&delivery_bytes),
    })
}

fn required_u64(value: &serde_json::Value, field: &str) -> Result<u64, String> {
    value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("delegated continuation claim has no integer `{field}`"))
}
