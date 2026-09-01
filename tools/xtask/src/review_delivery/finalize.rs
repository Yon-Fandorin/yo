use std::{fs, path::Path};

use serde::Deserialize;

use super::{
    DIAGNOSTIC_LIMIT, REQUEST_LIMIT, REVIEW_RESULT_LIMIT, canonical_json,
    delegated_session::{observe_host_continuation, observe_host_session},
    evaluate_host_admission,
    model::{
        DELEGATED_CLAIM_SCHEMA, DELEGATED_CLAIM_SCHEMA_V1_ALPHA2, DELEGATED_CLAIM_SCHEMA_V1_ALPHA3,
        DELEGATED_CONTINUATION_CLAIM_SCHEMA, DELEGATED_CONTINUATION_CLAIM_SCHEMA_V1_ALPHA2,
        DELEGATED_CONTINUATION_CLAIM_SCHEMA_V1_ALPHA3,
        DELEGATED_CONTINUATION_REQUEST_SCHEMA_V1_ALPHA2, DELEGATED_DELIVERY_RECEIPT_SCHEMA,
        DELEGATED_DELIVERY_RECEIPT_SCHEMA_V1_ALPHA2, DELEGATED_REQUEST_SCHEMA_V1_ALPHA2,
        DelegatedContinuationRequest, DelegatedDeliveryReceipt, DelegatedRequest, DeliveryRequest,
    },
    output_directory, read_request, require_exact_file_hash, shared_path,
};
use crate::{bounded_file, review_egress, review_protocol::digest};

const REQUEST_SCHEMA: &str = "yo.slice-review-delegated-delivery-finalize-request/v1alpha1";
const RESULT_SCHEMA: &str = "yo.slice-review-delegated-delivery-finalize-result/v1alpha1";
const FINALIZATION_SCHEMA: &str = "yo.external-review-delegated-delivery-finalization/v1alpha1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: String,
    delivery_request_path: String,
    delivery_request_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContinuationPreflightRequest {
    schema: String,
    egress_request_path: String,
    egress_request_hash: String,
    session_repository_path: String,
}

#[derive(Debug, Deserialize)]
struct StoredArtifact {
    path: String,
    hash: String,
    bytes: usize,
    published: bool,
}

pub(super) fn run(repository: &Path, request_path: &Path) -> Result<(), String> {
    let request_bytes = bounded_file::read_regular(
        request_path,
        REQUEST_LIMIT,
        "delegated delivery finalization request",
    )?;
    let request: Request = serde_json::from_slice(&request_bytes).map_err(|error| {
        format!(
            "invalid delegated delivery finalization request {}: {error}",
            request_path.display()
        )
    })?;
    if request.schema != REQUEST_SCHEMA {
        return Err(format!(
            "unsupported delegated delivery finalization schema `{}`; expected `{REQUEST_SCHEMA}`",
            request.schema
        ));
    }
    super::compact_path(&request.delivery_request_path, "delivery_request_path")?;
    super::require_sha256(&request.delivery_request_hash, "delivery_request_hash")?;
    let delivery_request_path = shared_path(repository, &request.delivery_request_path)?;
    require_exact_file_hash(
        &delivery_request_path,
        &request.delivery_request_hash,
        REQUEST_LIMIT,
        "delegated delivery request",
    )?;

    let finalized = match read_request(&delivery_request_path)? {
        DeliveryRequest::Delegated(request) => finalize_original(repository, request)?,
        DeliveryRequest::DelegatedContinuation(request) => {
            finalize_continuation(repository, request)?
        },
        _ => {
            return Err(
                "delivery finalization accepts only a delegated original or continuation request"
                    .to_owned(),
            );
        },
    };

    require_exact_file_hash(
        &delivery_request_path,
        &request.delivery_request_hash,
        REQUEST_LIMIT,
        "delegated delivery request",
    )?;
    if bounded_file::read_regular(
        request_path,
        REQUEST_LIMIT,
        "delegated delivery finalization request",
    )? != request_bytes
    {
        return Err("delegated delivery finalization request changed during recovery".to_owned());
    }
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "schema": RESULT_SCHEMA,
            "ok": true,
            "status": if finalized.created { "finalized" } else { "reused" },
            "next_action": "interpret_review",
            "request_id": finalized.request_id,
            "review_id": finalized.review_id,
            "candidate_commit": finalized.candidate_commit,
            "session_id": finalized.session_id,
            "host_request_id": finalized.host_request_id,
            "delivery_receipt_path": finalized.delivery_path,
            "delivery_receipt_hash": finalized.delivery_hash,
            "provider_requests": 0,
            "host_requests": 0
        }))
        .map_err(|error| format!("cannot encode delegated finalization result: {error}"))?
    );
    Ok(())
}

struct Finalized {
    created: bool,
    request_id: String,
    review_id: String,
    candidate_commit: String,
    session_id: String,
    host_request_id: String,
    delivery_path: String,
    delivery_hash: String,
}

fn finalize_original(repository: &Path, request: DelegatedRequest) -> Result<Finalized, String> {
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
    let claim = read_json(&output.join("claim.json"), "delegated delivery claim")?;
    validate_claim(
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
    validate_process_and_artifacts(&output, &delivery.request_id, false)?;
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

fn finalize_continuation(
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
    let preflight: ContinuationPreflightRequest = serde_json::from_slice(&preflight_bytes)
        .map_err(|error| format!("invalid delegated continuation preflight request: {error}"))?;
    if preflight.schema != "yo.slice-review-delegated-continuation-preflight-request/v1alpha1" {
        return Err("unsupported delegated continuation preflight schema".to_owned());
    }
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
    let claim = read_json(&output.join("claim.json"), "delegated continuation claim")?;
    let prior_anchor = required_u64(&claim, "continuation_anchor_sequence")?;
    let binding_epoch = required_u64(&claim, "binding_epoch")?;
    let preflight_id = digest(&preflight_bytes);
    validate_claim(
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
    validate_process_and_artifacts(&output, &delivery.request_id, true)?;
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

fn validate_claim(
    claim: &serde_json::Value,
    delivery: &review_egress::AuthorizedHostDelivery,
    admission_hash: &str,
    expected_schema: &str,
    continuation: bool,
    preflight_id: Option<&str>,
    execution_isolation: Option<&str>,
) -> Result<(), String> {
    require_claim_schema(claim, expected_schema)?;
    for (field, expected) in [
        ("request_id", delivery.request_id.as_str()),
        ("authorization_id", delivery.authorization_id.as_str()),
        ("authority", delivery.authority.as_str()),
        ("review_id", delivery.review_id.as_str()),
        ("candidate_commit", delivery.candidate_commit.as_str()),
        ("integration_commit", delivery.trusted_commit.as_str()),
        ("packet_hash", delivery.packet_hash.as_str()),
        ("execution_profile", delivery.execution_profile.as_str()),
        ("admission_request_id", admission_hash),
    ] {
        if claim.get(field).and_then(serde_json::Value::as_str) != Some(expected) {
            return Err(format!("delegated delivery claim changed `{field}`"));
        }
    }
    if claim
        .get("execution_isolation")
        .and_then(serde_json::Value::as_str)
        != execution_isolation
    {
        return Err("delegated delivery claim changed `execution_isolation`".to_owned());
    }
    if claim
        .pointer("/target/kind")
        .and_then(serde_json::Value::as_str)
        != Some("delegated_host")
        || claim
            .pointer("/target/host")
            .and_then(serde_json::Value::as_str)
            != Some(delivery.host.as_str())
        || claim
            .get("packet_bytes")
            .and_then(serde_json::Value::as_u64)
            != Some(delivery.packet_bytes.len() as u64)
        || claim
            .get("host_request_limit")
            .and_then(serde_json::Value::as_u64)
            != Some(1)
        || ["retries", "steer", "fallback"]
            .iter()
            .any(|field| claim.get(*field).and_then(serde_json::Value::as_u64) != Some(0))
        || claim
            .get("target_switch")
            .and_then(serde_json::Value::as_bool)
            != Some(false)
    {
        return Err("delegated delivery claim limits or identity changed".to_owned());
    }
    let expected_mode = if continuation { "resume" } else { "fresh" };
    if claim
        .get("session_mode")
        .and_then(serde_json::Value::as_str)
        != Some(expected_mode)
    {
        return Err("delegated delivery claim changed Session mode".to_owned());
    }
    if continuation
        && (claim
            .get("preflight_request_id")
            .and_then(serde_json::Value::as_str)
            != preflight_id
            || claim.get("session_id").and_then(serde_json::Value::as_str)
                != delivery.session_id.as_deref()
            || claim
                .get("prior_host_request_id")
                .and_then(serde_json::Value::as_str)
                != delivery.prior_host_request_id.as_deref())
    {
        return Err("delegated continuation claim changed its prior boundary".to_owned());
    }
    Ok(())
}

fn require_claim_schema(claim: &serde_json::Value, expected: &str) -> Result<(), String> {
    if claim.get("schema").and_then(serde_json::Value::as_str) == Some(expected) {
        Ok(())
    } else {
        Err(format!(
            "delegated delivery claim schema must equal `{expected}`"
        ))
    }
}

fn validate_process_and_artifacts(
    output: &Path,
    request_id: &str,
    continuation: bool,
) -> Result<(), String> {
    let outcome = read_json(&output.join("outcome.json"), "delegated delivery outcome")?;
    let expected_schema = if continuation {
        "yo.external-review-delegated-continuation-delivery-outcome/v1alpha1"
    } else {
        "yo.external-review-delegated-delivery-outcome/v1alpha1"
    };
    if outcome.get("schema").and_then(serde_json::Value::as_str) != Some(expected_schema)
        || outcome
            .get("request_id")
            .and_then(serde_json::Value::as_str)
            != Some(request_id)
        || outcome
            .pointer("/process/exit_code")
            .and_then(serde_json::Value::as_i64)
            != Some(0)
        || outcome
            .pointer("/process/signal")
            .is_some_and(|value| !value.is_null())
    {
        return Err(
            "delegated delivery finalization requires an immutable successful process outcome"
                .to_owned(),
        );
    }
    verify_artifact(
        output,
        outcome
            .get("review_result")
            .ok_or_else(|| "delegated outcome has no review_result".to_owned())?,
        "review.txt",
        REVIEW_RESULT_LIMIT,
        "delegated review result",
    )?;
    verify_artifact(
        output,
        outcome
            .get("diagnostic")
            .ok_or_else(|| "delegated outcome has no diagnostic".to_owned())?,
        "diagnostic.txt",
        DIAGNOSTIC_LIMIT,
        "delegated review diagnostic",
    )?;
    Ok(())
}

fn verify_artifact(
    output: &Path,
    value: &serde_json::Value,
    file: &str,
    limit: usize,
    label: &str,
) -> Result<(), String> {
    let artifact: StoredArtifact = serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid {label} artifact: {error}"))?;
    let expected_path = output.join(file);
    if !artifact.published || artifact.path != expected_path.to_string_lossy() {
        return Err(format!(
            "{label} was not published at its exact output path"
        ));
    }
    let bytes = bounded_file::read_regular(&expected_path, limit, label)?;
    if artifact.bytes != bytes.len() || artifact.hash != digest(&bytes) {
        return Err(format!("{label} bytes differ from the immutable outcome"));
    }
    Ok(())
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
        target: super::delegated::target(delivery),
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

fn read_json(path: &Path, label: &str) -> Result<serde_json::Value, String> {
    let bytes = bounded_file::read_regular(path, REQUEST_LIMIT, label)?;
    serde_json::from_slice(&bytes).map_err(|error| format!("invalid {label}: {error}"))
}

fn required_u64(value: &serde_json::Value, field: &str) -> Result<u64, String> {
    value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("delegated continuation claim has no integer `{field}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 복구 요청은 전송 입력을 exact hash로만 가리키며 실행 파일·재시도 옵션을
    // 표현할 수 없어 provider/host request를 추가하는 입력 공간이 없습니다.
    #[test]
    fn finalization_request_has_no_delivery_effect_fields() {
        let valid = serde_json::json!({
            "schema": REQUEST_SCHEMA,
            "delivery_request_path": "/tmp/delivery-request.json",
            "delivery_request_hash": format!("sha256:{}", "a".repeat(64))
        });
        serde_json::from_value::<Request>(valid.clone()).unwrap();
        let mut extra = valid;
        extra["retry"] = true.into();
        assert!(serde_json::from_value::<Request>(extra).is_err());
    }

    // 복구는 필드가 우연히 같은 미지의 claim을 수용하지 않고 delivery request가
    // 소유하는 frozen claim schema를 exact wire boundary로 먼저 확인합니다.
    #[test]
    fn finalization_requires_the_exact_claim_schema() {
        let exact = serde_json::json!({"schema": DELEGATED_CLAIM_SCHEMA_V1_ALPHA2});
        require_claim_schema(&exact, DELEGATED_CLAIM_SCHEMA_V1_ALPHA2).unwrap();
        assert!(
            require_claim_schema(&exact, DELEGATED_CLAIM_SCHEMA)
                .unwrap_err()
                .contains(DELEGATED_CLAIM_SCHEMA)
        );
        let unknown = serde_json::json!({"schema": "yo.unknown-claim/v1alpha1"});
        assert!(require_claim_schema(&unknown, DELEGATED_CLAIM_SCHEMA).is_err());
    }
}
