use serde::{Deserialize, Serialize};

pub(super) const REQUEST_SCHEMA: &str = "yo.slice-review-delivery-request/v1alpha1";
pub(super) const REQUEST_SCHEMA_V1_ALPHA2: &str = "yo.slice-review-delivery-request/v1alpha2";
pub(super) const CONTINUATION_REQUEST_SCHEMA: &str =
    "yo.slice-review-continuation-delivery-request/v1alpha1";
pub(super) const CONTINUATION_REQUEST_SCHEMA_V1_ALPHA2: &str =
    "yo.slice-review-continuation-delivery-request/v1alpha2";
pub(super) const CLAIM_SCHEMA: &str = "yo.external-review-delivery-claim/v1alpha1";
pub(super) const CLAIM_SCHEMA_V1_ALPHA2: &str = "yo.external-review-delivery-claim/v1alpha2";
pub(super) const CONTINUATION_CLAIM_SCHEMA: &str =
    "yo.external-review-continuation-delivery-claim/v1alpha1";
pub(super) const CONTINUATION_CLAIM_SCHEMA_V1_ALPHA2: &str =
    "yo.external-review-continuation-delivery-claim/v1alpha2";
pub(super) const OUTCOME_SCHEMA: &str = "yo.external-review-delivery-outcome/v1alpha1";
pub(super) const CONTINUATION_OUTCOME_SCHEMA: &str =
    "yo.external-review-continuation-delivery-outcome/v1alpha1";
pub(super) const RESULT_SCHEMA: &str = "yo.slice-review-delivery-result/v1alpha1";
pub(super) const RESULT_SCHEMA_V1_ALPHA2: &str = "yo.slice-review-delivery-result/v1alpha2";
pub(super) const RESULT_SCHEMA_V1_ALPHA3: &str = "yo.slice-review-delivery-result/v1alpha3";
pub(super) const CONTINUATION_RESULT_SCHEMA: &str =
    "yo.slice-review-continuation-delivery-result/v1alpha1";
pub(super) const CONTINUATION_RESULT_SCHEMA_V1_ALPHA2: &str =
    "yo.slice-review-continuation-delivery-result/v1alpha2";
pub(super) const CONTINUATION_RESULT_SCHEMA_V1_ALPHA3: &str =
    "yo.slice-review-continuation-delivery-result/v1alpha3";
pub(super) const DELIVERY_RECEIPT_SCHEMA: &str = "yo.external-review-delivery-receipt/v1";
pub(super) const DELEGATED_REQUEST_SCHEMA: &str =
    "yo.slice-review-delegated-delivery-request/v1alpha1";
pub(super) const DELEGATED_REQUEST_SCHEMA_V1_ALPHA2: &str =
    "yo.slice-review-delegated-delivery-request/v1alpha2";
pub(super) const DELEGATED_CONTINUATION_REQUEST_SCHEMA: &str =
    "yo.slice-review-delegated-continuation-delivery-request/v1alpha1";
pub(super) const DELEGATED_CONTINUATION_REQUEST_SCHEMA_V1_ALPHA2: &str =
    "yo.slice-review-delegated-continuation-delivery-request/v1alpha2";
pub(super) const DELEGATED_CLAIM_SCHEMA: &str =
    "yo.external-review-delegated-delivery-claim/v1alpha1";
pub(super) const DELEGATED_CLAIM_SCHEMA_V1_ALPHA2: &str =
    "yo.external-review-delegated-delivery-claim/v1alpha2";
pub(super) const DELEGATED_CLAIM_SCHEMA_V1_ALPHA3: &str =
    "yo.external-review-delegated-delivery-claim/v1alpha3";
pub(super) const DELEGATED_CONTINUATION_CLAIM_SCHEMA: &str =
    "yo.external-review-delegated-continuation-delivery-claim/v1alpha1";
pub(super) const DELEGATED_CONTINUATION_CLAIM_SCHEMA_V1_ALPHA2: &str =
    "yo.external-review-delegated-continuation-delivery-claim/v1alpha2";
pub(super) const DELEGATED_CONTINUATION_CLAIM_SCHEMA_V1_ALPHA3: &str =
    "yo.external-review-delegated-continuation-delivery-claim/v1alpha3";
pub(super) const DELEGATED_OUTCOME_SCHEMA: &str =
    "yo.external-review-delegated-delivery-outcome/v1alpha1";
pub(super) const DELEGATED_CONTINUATION_OUTCOME_SCHEMA: &str =
    "yo.external-review-delegated-continuation-delivery-outcome/v1alpha1";
pub(super) const DELEGATED_RESULT_SCHEMA: &str =
    "yo.slice-review-delegated-delivery-result/v1alpha1";
pub(super) const DELEGATED_RESULT_SCHEMA_V1_ALPHA2: &str =
    "yo.slice-review-delegated-delivery-result/v1alpha2";
pub(super) const DELEGATED_RESULT_SCHEMA_V1_ALPHA3: &str =
    "yo.slice-review-delegated-delivery-result/v1alpha3";
pub(super) const DELEGATED_CONTINUATION_RESULT_SCHEMA: &str =
    "yo.slice-review-delegated-continuation-delivery-result/v1alpha1";
pub(super) const DELEGATED_CONTINUATION_RESULT_SCHEMA_V1_ALPHA2: &str =
    "yo.slice-review-delegated-continuation-delivery-result/v1alpha2";
pub(super) const DELEGATED_CONTINUATION_RESULT_SCHEMA_V1_ALPHA3: &str =
    "yo.slice-review-delegated-continuation-delivery-result/v1alpha3";
pub(super) const DELEGATED_DELIVERY_RECEIPT_SCHEMA: &str =
    "yo.external-review-delegated-delivery-receipt/v1alpha1";
pub(super) const DELEGATED_DELIVERY_RECEIPT_SCHEMA_V1_ALPHA2: &str =
    "yo.external-review-delegated-delivery-receipt/v1alpha2";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub(super) schema: String,
    pub(super) egress_request_path: String,
    pub(super) egress_request_hash: String,
    pub(super) output_directory: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AdmittedRequest {
    pub(super) schema: String,
    pub(super) egress_request_path: String,
    pub(super) egress_request_hash: String,
    pub(super) admission_request_path: String,
    pub(super) admission_request_hash: String,
    pub(super) output_directory: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ContinuationRequest {
    pub(super) schema: String,
    pub(super) preflight_request_path: String,
    pub(super) preflight_request_hash: String,
    pub(super) output_directory: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AdmittedContinuationRequest {
    pub(super) schema: String,
    pub(super) preflight_request_path: String,
    pub(super) preflight_request_hash: String,
    pub(super) admission_request_path: String,
    pub(super) admission_request_hash: String,
    pub(super) output_directory: String,
}

#[derive(Debug)]
pub(super) enum DeliveryRequest {
    Original(Request),
    AdmittedOriginal(AdmittedRequest),
    Continuation(ContinuationRequest),
    AdmittedContinuation(AdmittedContinuationRequest),
    Delegated(DelegatedRequest),
    DelegatedContinuation(DelegatedContinuationRequest),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DelegatedRequest {
    pub(super) schema: String,
    pub(super) egress_request_path: String,
    pub(super) egress_request_hash: String,
    pub(super) admission_request_path: String,
    pub(super) admission_request_hash: String,
    pub(super) output_directory: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DelegatedContinuationRequest {
    pub(super) schema: String,
    pub(super) preflight_request_path: String,
    pub(super) preflight_request_hash: String,
    pub(super) admission_request_path: String,
    pub(super) admission_request_hash: String,
    pub(super) output_directory: String,
}

#[derive(Debug, Serialize)]
pub(super) struct Route<'a> {
    pub(super) provider: &'a str,
    pub(super) account: &'a str,
    pub(super) model: &'a str,
}

#[derive(Debug, Serialize)]
pub(super) struct Artifact {
    pub(super) path: String,
    pub(super) hash: String,
    pub(super) bytes: usize,
    pub(super) published: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct Claim<'a> {
    pub(super) schema: &'static str,
    pub(super) request_id: &'a str,
    pub(super) authorization_id: &'a str,
    pub(super) authority: &'a str,
    pub(super) review_id: &'a str,
    pub(super) candidate_commit: &'a str,
    pub(super) integration_commit: &'a str,
    pub(super) packet_hash: &'a str,
    pub(super) packet_bytes: usize,
    pub(super) managed_payload_tokens: usize,
    pub(super) route: Route<'a>,
    pub(super) session_mode: &'static str,
    pub(super) provider_request_limit: usize,
    pub(super) retries: usize,
    pub(super) steer: usize,
    pub(super) fallback: usize,
    pub(super) second_provider: bool,
    pub(super) tool_execution: bool,
    pub(super) yo_binary_hash: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) admission_request_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) target: Option<&'a crate::review_target_admission::ReviewTarget>,
}

#[derive(Debug, Serialize)]
pub(super) struct ContinuationClaim<'a> {
    pub(super) schema: &'static str,
    pub(super) request_id: &'a str,
    pub(super) preflight_request_id: &'a str,
    pub(super) authorization_id: &'a str,
    pub(super) authority: &'a str,
    pub(super) review_id: &'a str,
    pub(super) candidate_commit: &'a str,
    pub(super) integration_commit: &'a str,
    pub(super) packet_hash: &'a str,
    pub(super) packet_bytes: usize,
    pub(super) managed_payload_tokens: usize,
    pub(super) route: Route<'a>,
    pub(super) session_mode: &'static str,
    pub(super) session_id: &'a str,
    pub(super) prior_provider_request_id: &'a str,
    pub(super) continuation_anchor_sequence: u64,
    pub(super) binding_epoch: u64,
    pub(super) provider_request_limit: usize,
    pub(super) retries: usize,
    pub(super) steer: usize,
    pub(super) fallback: usize,
    pub(super) second_provider: bool,
    pub(super) tool_execution: bool,
    pub(super) yo_binary_hash: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) admission_request_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) target: Option<&'a crate::review_target_admission::ReviewTarget>,
}

#[derive(Debug, Serialize)]
pub(super) struct ProcessOutcome {
    pub(super) exit_code: Option<i32>,
    #[cfg(unix)]
    pub(super) signal: Option<i32>,
}

#[derive(Debug, Serialize)]
pub(super) struct DeliveryOutcome {
    pub(super) schema: &'static str,
    pub(super) request_id: String,
    pub(super) status: &'static str,
    pub(super) process: ProcessOutcome,
    pub(super) session_id: Option<String>,
    pub(super) durable_provider_request_count: usize,
    pub(super) provider_request_id: Option<String>,
    pub(super) review_result: Artifact,
    pub(super) diagnostic: Artifact,
    pub(super) failure: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ContinuationDeliveryOutcome {
    pub(super) schema: &'static str,
    pub(super) request_id: String,
    pub(super) preflight_request_id: String,
    pub(super) status: &'static str,
    pub(super) process: ProcessOutcome,
    pub(super) session_id: String,
    pub(super) durable_provider_request_count: usize,
    pub(super) provider_request_id: Option<String>,
    pub(super) continuation_anchor_sequence: Option<u64>,
    pub(super) review_result: Artifact,
    pub(super) diagnostic: Artifact,
    pub(super) failure: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct DeliveryReceipt<'a> {
    pub(super) schema: &'static str,
    pub(super) review_id: &'a str,
    pub(super) packet_hash: &'a str,
    pub(super) route: Route<'a>,
    pub(super) session_id: &'a str,
    pub(super) provider_request_id: &'a str,
    pub(super) provider_request_count: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct ResultDocument {
    pub(super) schema: &'static str,
    pub(super) ok: bool,
    pub(super) status: &'static str,
    pub(super) next_action: &'static str,
    pub(super) request_id: String,
    pub(super) review_id: String,
    pub(super) candidate_commit: String,
    pub(super) integration_commit: String,
    pub(super) session_id: String,
    pub(super) provider_request_id: String,
    pub(super) review_result: Artifact,
    pub(super) diagnostic: Artifact,
    pub(super) outcome: Artifact,
    pub(super) delivery_receipt: Artifact,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) provider_usage: Option<Artifact>,
}

#[derive(Debug, Serialize)]
pub(super) struct ContinuationResultDocument {
    pub(super) schema: &'static str,
    pub(super) ok: bool,
    pub(super) status: &'static str,
    pub(super) next_action: &'static str,
    pub(super) request_id: String,
    pub(super) preflight_request_id: String,
    pub(super) review_id: String,
    pub(super) candidate_commit: String,
    pub(super) integration_commit: String,
    pub(super) session_id: String,
    pub(super) provider_request_id: String,
    pub(super) continuation_anchor_sequence: u64,
    pub(super) review_result: Artifact,
    pub(super) diagnostic: Artifact,
    pub(super) outcome: Artifact,
    pub(super) delivery_receipt: Artifact,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) provider_usage: Option<Artifact>,
}

#[derive(Debug, Serialize)]
pub(super) struct DelegatedTarget<'a> {
    pub(super) kind: &'static str,
    pub(super) host: &'a str,
}

#[derive(Debug, Serialize)]
pub(super) struct DelegatedClaim<'a> {
    pub(super) schema: &'static str,
    pub(super) request_id: &'a str,
    pub(super) authorization_id: &'a str,
    pub(super) authority: &'a str,
    pub(super) review_id: &'a str,
    pub(super) candidate_commit: &'a str,
    pub(super) integration_commit: &'a str,
    pub(super) packet_hash: &'a str,
    pub(super) packet_bytes: usize,
    pub(super) managed_payload_tokens: usize,
    pub(super) target: DelegatedTarget<'a>,
    pub(super) execution_profile: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) execution_isolation: Option<&'a str>,
    pub(super) session_mode: &'static str,
    pub(super) host_request_limit: usize,
    pub(super) retries: usize,
    pub(super) steer: usize,
    pub(super) fallback: usize,
    pub(super) target_switch: bool,
    pub(super) yo_binary_hash: &'a str,
    pub(super) admission_request_id: &'a str,
}

#[derive(Debug, Serialize)]
pub(super) struct DelegatedContinuationClaim<'a> {
    pub(super) schema: &'static str,
    pub(super) request_id: &'a str,
    pub(super) preflight_request_id: &'a str,
    pub(super) authorization_id: &'a str,
    pub(super) authority: &'a str,
    pub(super) review_id: &'a str,
    pub(super) candidate_commit: &'a str,
    pub(super) integration_commit: &'a str,
    pub(super) packet_hash: &'a str,
    pub(super) packet_bytes: usize,
    pub(super) managed_payload_tokens: usize,
    pub(super) target: DelegatedTarget<'a>,
    pub(super) execution_profile: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) execution_isolation: Option<&'a str>,
    pub(super) session_mode: &'static str,
    pub(super) session_id: &'a str,
    pub(super) prior_host_request_id: &'a str,
    pub(super) continuation_anchor_sequence: u64,
    pub(super) binding_epoch: u64,
    pub(super) host_request_limit: usize,
    pub(super) retries: usize,
    pub(super) steer: usize,
    pub(super) fallback: usize,
    pub(super) target_switch: bool,
    pub(super) yo_binary_hash: &'a str,
    pub(super) admission_request_id: &'a str,
}

#[derive(Debug, Serialize)]
pub(super) struct DelegatedDeliveryOutcome {
    pub(super) schema: &'static str,
    pub(super) request_id: String,
    pub(super) status: &'static str,
    pub(super) process: ProcessOutcome,
    pub(super) session_id: Option<String>,
    pub(super) durable_host_request_count: usize,
    pub(super) host_request_id: Option<String>,
    pub(super) review_result: Artifact,
    pub(super) diagnostic: Artifact,
    pub(super) failure: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct DelegatedContinuationDeliveryOutcome {
    pub(super) schema: &'static str,
    pub(super) request_id: String,
    pub(super) preflight_request_id: String,
    pub(super) status: &'static str,
    pub(super) process: ProcessOutcome,
    pub(super) session_id: String,
    pub(super) durable_host_request_count: usize,
    pub(super) host_request_id: Option<String>,
    pub(super) continuation_anchor_sequence: Option<u64>,
    pub(super) review_result: Artifact,
    pub(super) diagnostic: Artifact,
    pub(super) failure: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct DelegatedDeliveryReceipt<'a> {
    pub(super) schema: &'static str,
    pub(super) review_id: &'a str,
    pub(super) packet_hash: &'a str,
    pub(super) target: DelegatedTarget<'a>,
    pub(super) execution_profile: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) execution_isolation: Option<&'a str>,
    pub(super) session_id: &'a str,
    pub(super) host_request_id: &'a str,
    pub(super) host_request_count: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct DelegatedResultDocument {
    pub(super) schema: &'static str,
    pub(super) ok: bool,
    pub(super) status: &'static str,
    pub(super) next_action: &'static str,
    pub(super) request_id: String,
    pub(super) review_id: String,
    pub(super) candidate_commit: String,
    pub(super) integration_commit: String,
    pub(super) session_id: String,
    pub(super) host_request_id: String,
    pub(super) review_result: Artifact,
    pub(super) diagnostic: Artifact,
    pub(super) outcome: Artifact,
    pub(super) delivery_receipt: Artifact,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) provider_usage: Option<Artifact>,
}

#[derive(Debug, Serialize)]
pub(super) struct DelegatedContinuationResultDocument {
    pub(super) schema: &'static str,
    pub(super) ok: bool,
    pub(super) status: &'static str,
    pub(super) next_action: &'static str,
    pub(super) request_id: String,
    pub(super) preflight_request_id: String,
    pub(super) review_id: String,
    pub(super) candidate_commit: String,
    pub(super) integration_commit: String,
    pub(super) session_id: String,
    pub(super) host_request_id: String,
    pub(super) continuation_anchor_sequence: u64,
    pub(super) review_result: Artifact,
    pub(super) diagnostic: Artifact,
    pub(super) outcome: Artifact,
    pub(super) delivery_receipt: Artifact,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) provider_usage: Option<Artifact>,
}
