use std::path::Path;

use super::{
    CONTINUATION_REQUEST_SCHEMA_V1_ALPHA3, CONTINUATION_REQUEST_SCHEMA_V1_ALPHA4,
    DELEGATED_CONTINUATION_REQUEST_SCHEMA_V1_ALPHA3,
    DELEGATED_CONTINUATION_REQUEST_SCHEMA_V1_ALPHA4, DELEGATED_REQUEST_SCHEMA_V1_ALPHA3,
    DELEGATED_REQUEST_SCHEMA_V1_ALPHA4, DeliveryPolicy, REQUEST_LIMIT, REQUEST_SCHEMA_V1_ALPHA3,
    REQUEST_SCHEMA_V1_ALPHA4,
    artifact::{compact_path, require_exact_file_hash, require_sha256},
    model::{
        AdmittedContinuationRequest, AdmittedRequest, CONTINUATION_REQUEST_SCHEMA,
        CONTINUATION_REQUEST_SCHEMA_V1_ALPHA2, ContinuationRequest,
        DELEGATED_CONTINUATION_REQUEST_SCHEMA, DELEGATED_CONTINUATION_REQUEST_SCHEMA_V1_ALPHA2,
        DELEGATED_REQUEST_SCHEMA, DELEGATED_REQUEST_SCHEMA_V1_ALPHA2, DelegatedContinuationRequest,
        DelegatedRequest, DeliveryRequest, REQUEST_SCHEMA, REQUEST_SCHEMA_V1_ALPHA2, Request,
    },
    workspace::shared_path,
};
use crate::{
    bounded_file,
    review_egress::{AuthorizedDelivery, AuthorizedHostDelivery},
    review_target_admission::{self, Admission, ReviewTarget},
};

#[derive(Debug)]
pub(super) struct AdmissionReference {
    pub(super) path: String,
    pub(super) hash: String,
}
pub(super) fn read_request(path: &Path) -> Result<DeliveryRequest, String> {
    read_request_with_output_policy(path).map(|(request, _)| request)
}

pub(super) fn read_request_with_output_policy(
    path: &Path,
) -> Result<(DeliveryRequest, DeliveryPolicy), String> {
    let bytes = bounded_file::read_regular(path, REQUEST_LIMIT, "Slice review delivery request")?;
    let header: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "invalid Slice review delivery request {}: {error}",
            path.display()
        )
    })?;
    let schema = header
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "Slice review delivery request has no string schema".to_owned())?;
    match schema {
        REQUEST_SCHEMA => {
            let request: Request = serde_json::from_slice(&bytes)
                .map_err(|error| format!("invalid original review delivery request: {error}"))?;
            debug_assert_eq!(request.schema, REQUEST_SCHEMA);
            compact_path(&request.egress_request_path, "egress_request_path")?;
            compact_path(&request.output_directory, "output_directory")?;
            require_sha256(&request.egress_request_hash, "egress_request_hash")?;
            Ok((
                DeliveryRequest::Original(request),
                DeliveryPolicy::default(),
            ))
        },
        REQUEST_SCHEMA_V1_ALPHA2 | REQUEST_SCHEMA_V1_ALPHA3 | REQUEST_SCHEMA_V1_ALPHA4 => {
            let mut request: AdmittedRequest = serde_json::from_slice(&bytes).map_err(|error| {
                format!("invalid admitted original review delivery request: {error}")
            })?;
            let policy = DeliveryPolicy {
                prepare_output: matches!(
                    request.schema.as_str(),
                    REQUEST_SCHEMA_V1_ALPHA3 | REQUEST_SCHEMA_V1_ALPHA4
                ),
                bind_usage: request.schema == REQUEST_SCHEMA_V1_ALPHA4,
            };
            request.schema = REQUEST_SCHEMA_V1_ALPHA2.to_owned();
            compact_path(&request.egress_request_path, "egress_request_path")?;
            compact_path(&request.admission_request_path, "admission_request_path")?;
            compact_path(&request.output_directory, "output_directory")?;
            require_sha256(&request.egress_request_hash, "egress_request_hash")?;
            require_sha256(&request.admission_request_hash, "admission_request_hash")?;
            Ok((DeliveryRequest::AdmittedOriginal(request), policy))
        },
        CONTINUATION_REQUEST_SCHEMA => {
            let request: ContinuationRequest = serde_json::from_slice(&bytes).map_err(|error| {
                format!("invalid continuation review delivery request: {error}")
            })?;
            debug_assert_eq!(request.schema, CONTINUATION_REQUEST_SCHEMA);
            compact_path(&request.preflight_request_path, "preflight_request_path")?;
            compact_path(&request.output_directory, "output_directory")?;
            require_sha256(&request.preflight_request_hash, "preflight_request_hash")?;
            Ok((
                DeliveryRequest::Continuation(request),
                DeliveryPolicy::default(),
            ))
        },
        CONTINUATION_REQUEST_SCHEMA_V1_ALPHA2
        | CONTINUATION_REQUEST_SCHEMA_V1_ALPHA3
        | CONTINUATION_REQUEST_SCHEMA_V1_ALPHA4 => {
            let mut request: AdmittedContinuationRequest =
                serde_json::from_slice(&bytes).map_err(|error| {
                    format!("invalid admitted continuation review delivery request: {error}")
                })?;
            let policy = DeliveryPolicy {
                prepare_output: matches!(
                    request.schema.as_str(),
                    CONTINUATION_REQUEST_SCHEMA_V1_ALPHA3 | CONTINUATION_REQUEST_SCHEMA_V1_ALPHA4
                ),
                bind_usage: request.schema == CONTINUATION_REQUEST_SCHEMA_V1_ALPHA4,
            };
            request.schema = CONTINUATION_REQUEST_SCHEMA_V1_ALPHA2.to_owned();
            compact_path(&request.preflight_request_path, "preflight_request_path")?;
            compact_path(&request.admission_request_path, "admission_request_path")?;
            compact_path(&request.output_directory, "output_directory")?;
            require_sha256(&request.preflight_request_hash, "preflight_request_hash")?;
            require_sha256(&request.admission_request_hash, "admission_request_hash")?;
            Ok((DeliveryRequest::AdmittedContinuation(request), policy))
        },
        DELEGATED_REQUEST_SCHEMA
        | DELEGATED_REQUEST_SCHEMA_V1_ALPHA2
        | DELEGATED_REQUEST_SCHEMA_V1_ALPHA3
        | DELEGATED_REQUEST_SCHEMA_V1_ALPHA4 => {
            let mut request: DelegatedRequest = serde_json::from_slice(&bytes)
                .map_err(|error| format!("invalid delegated review delivery request: {error}"))?;
            let policy = DeliveryPolicy {
                prepare_output: matches!(
                    request.schema.as_str(),
                    DELEGATED_REQUEST_SCHEMA_V1_ALPHA3 | DELEGATED_REQUEST_SCHEMA_V1_ALPHA4
                ),
                bind_usage: request.schema == DELEGATED_REQUEST_SCHEMA_V1_ALPHA4,
            };
            if policy.prepare_output {
                request.schema = DELEGATED_REQUEST_SCHEMA_V1_ALPHA2.to_owned();
            }
            compact_path(&request.egress_request_path, "egress_request_path")?;
            compact_path(&request.admission_request_path, "admission_request_path")?;
            compact_path(&request.output_directory, "output_directory")?;
            require_sha256(&request.egress_request_hash, "egress_request_hash")?;
            require_sha256(&request.admission_request_hash, "admission_request_hash")?;
            Ok((DeliveryRequest::Delegated(request), policy))
        },
        DELEGATED_CONTINUATION_REQUEST_SCHEMA
        | DELEGATED_CONTINUATION_REQUEST_SCHEMA_V1_ALPHA2
        | DELEGATED_CONTINUATION_REQUEST_SCHEMA_V1_ALPHA3
        | DELEGATED_CONTINUATION_REQUEST_SCHEMA_V1_ALPHA4 => {
            let mut request: DelegatedContinuationRequest = serde_json::from_slice(&bytes)
                .map_err(|error| {
                    format!("invalid delegated continuation delivery request: {error}")
                })?;
            let policy = DeliveryPolicy {
                prepare_output: matches!(
                    request.schema.as_str(),
                    DELEGATED_CONTINUATION_REQUEST_SCHEMA_V1_ALPHA3
                        | DELEGATED_CONTINUATION_REQUEST_SCHEMA_V1_ALPHA4
                ),
                bind_usage: request.schema == DELEGATED_CONTINUATION_REQUEST_SCHEMA_V1_ALPHA4,
            };
            if policy.prepare_output {
                request.schema = DELEGATED_CONTINUATION_REQUEST_SCHEMA_V1_ALPHA2.to_owned();
            }
            compact_path(&request.preflight_request_path, "preflight_request_path")?;
            compact_path(&request.admission_request_path, "admission_request_path")?;
            compact_path(&request.output_directory, "output_directory")?;
            require_sha256(&request.preflight_request_hash, "preflight_request_hash")?;
            require_sha256(&request.admission_request_hash, "admission_request_hash")?;
            Ok((DeliveryRequest::DelegatedContinuation(request), policy))
        },
        other => Err(format!(
            "unsupported Slice review delivery request schema `{other}`; expected a supported original, continuation, or delegated delivery schema from v1alpha1 through v1alpha4"
        )),
    }
}

pub(super) fn evaluate_host_admission(
    repository: &Path,
    request_path: &str,
    request_hash: &str,
    delivery: &AuthorizedHostDelivery,
    require_state_readiness: bool,
) -> Result<Admission, String> {
    let path = shared_path(repository, request_path)?;
    require_exact_file_hash(
        &path,
        request_hash,
        REQUEST_LIMIT,
        "delegated external review target admission request",
    )?;
    let admission = review_target_admission::evaluate(&path)?;
    let expected = ReviewTarget::DelegatedHost {
        host: delivery.host.clone(),
    };
    if admission.target != expected {
        return Err(
            "review-target admission differs from the authorized delegated host".to_owned(),
        );
    }
    if !admission.admitted() {
        return Err(format!(
            "delegated external review target admission stopped before claim: {}",
            admission.availability_detail()
        ));
    }
    if require_state_readiness {
        if !admission.has_delegated_host_state_readiness() {
            return Err(
                "delegated delivery v1alpha2 requires target admission v1alpha3 host-state readiness"
                    .to_owned(),
            );
        }
    } else if !admission.supports_frozen_delegated_delivery() {
        return Err(
            "delegated delivery v1alpha1 requires frozen target admission v1alpha2 eligibility"
                .to_owned(),
        );
    }
    Ok(admission)
}

pub(super) fn evaluate_admission(
    repository: &Path,
    reference: &AdmissionReference,
    delivery: &AuthorizedDelivery,
) -> Result<Admission, String> {
    let path = shared_path(repository, &reference.path)?;
    require_exact_file_hash(
        &path,
        &reference.hash,
        REQUEST_LIMIT,
        "external review target admission request",
    )?;
    let admission = review_target_admission::evaluate(&path)?;
    let expected = ReviewTarget::managed(
        delivery.provider.clone(),
        delivery.account.clone(),
        delivery.model.clone(),
    );
    if admission.target != expected {
        return Err("review-target admission differs from the authorized managed route".to_owned());
    }
    if !admission.admitted() {
        return Err(format!(
            "external review target admission stopped before claim: {}",
            admission.availability_detail()
        ));
    }
    Ok(admission)
}

pub(super) fn require_original_fresh(delivery: &AuthorizedDelivery) -> Result<(), String> {
    if delivery.review_kind != "original" || !delivery.fresh_session {
        Err(
            "review-deliver v1alpha1 supports only one original packet in a fresh Session"
                .to_owned(),
        )
    } else {
        Ok(())
    }
}

pub(super) fn managed_model_reference(delivery: &AuthorizedDelivery) -> Result<String, String> {
    if [
        delivery.provider.as_str(),
        delivery.account.as_str(),
        delivery.model.as_str(),
    ]
    .into_iter()
    .any(|part| part.contains(':'))
    {
        return Err(
            "review-deliver v1alpha1 managed route components must not contain `:`".to_owned(),
        );
    }
    Ok(format!(
        "{}:{}:{}",
        delivery.provider, delivery.account, delivery.model
    ))
}
