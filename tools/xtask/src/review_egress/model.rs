use serde::{Deserialize, Serialize};

pub(super) const REQUEST_SCHEMA: &str = "yo.slice-review-egress-request/v1";
pub(super) const AUTHORIZATION_SCHEMA: &str = "yo.external-review-standing-authorization/v1";
pub(super) const DELIVERY_RECEIPT_SCHEMA: &str = "yo.external-review-delivery-receipt/v1";
pub(super) const RESULT_SCHEMA: &str = "yo.slice-review-egress-result/v1";
pub(super) const DELEGATED_REQUEST_SCHEMA: &str =
    "yo.slice-review-delegated-egress-request/v1alpha1";
pub(super) const DELEGATED_AUTHORIZATION_SCHEMA: &str =
    "yo.external-review-delegated-authorization/v1alpha1";
pub(super) const DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA2: &str =
    "yo.external-review-delegated-authorization/v1alpha2";
pub(super) const DELEGATED_AUTHORIZATION_SCHEMA_V1_ALPHA3: &str =
    "yo.external-review-delegated-authorization/v1alpha3";
pub(super) const DELEGATED_REVIEW_CHAIN_PROFILE: &str =
    "yo.external-review-chain/bounded-multihop/v1alpha1";
pub(super) const DELEGATED_DELIVERY_RECEIPT_SCHEMA: &str =
    "yo.external-review-delegated-delivery-receipt/v1alpha1";
pub(super) const DELEGATED_RESULT_SCHEMA: &str = "yo.slice-review-delegated-egress-result/v1alpha1";
pub(super) const DELEGATED_EXECUTION_PROFILE: &str = "yo.delegated-review-execution/v1alpha1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub(super) schema: String,
    pub(super) manifest_path: String,
    pub(super) manifest_hash: String,
    pub(super) authorization_hash: String,
    pub(super) route: Route,
    pub(super) session: Session,
    #[serde(default)]
    pub(super) prior_delivery: Option<Artifact>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DelegatedRequest {
    pub(super) schema: String,
    pub(super) manifest_path: String,
    pub(super) manifest_hash: String,
    pub(super) authorization_hash: String,
    pub(super) target: DelegatedTarget,
    pub(super) execution_profile: String,
    pub(super) session: Session,
    #[serde(default)]
    pub(super) prior_delivery: Option<Artifact>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum DelegatedTarget {
    DelegatedHost { host: String },
}

impl DelegatedTarget {
    pub(super) fn host(&self) -> &str {
        match self {
            Self::DelegatedHost { host } => host,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Route {
    pub(super) provider: String,
    pub(super) account: String,
    pub(super) model: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Session {
    Fresh,
    Resume { id: String },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Authorization {
    pub(super) schema: String,
    pub(super) authority: String,
    pub(super) status: String,
    pub(super) routes: Vec<AuthorizedRoute>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DelegatedAuthorization {
    pub(super) schema: String,
    pub(super) authority: String,
    pub(super) status: String,
    pub(super) targets: Vec<AuthorizedDelegatedTarget>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DelegatedAuthorizationV1Alpha2 {
    pub(super) schema: String,
    pub(super) authority: String,
    pub(super) status: String,
    pub(super) targets: Vec<AuthorizedDelegatedTargetV1Alpha2>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DelegatedAuthorizationV1Alpha3 {
    pub(super) schema: String,
    pub(super) authority: String,
    pub(super) status: String,
    pub(super) review_chain_profile: String,
    pub(super) targets: Vec<AuthorizedDelegatedTargetV1Alpha3>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum DelegatedAuthorizationDocument {
    Alpha1(DelegatedAuthorization),
    Alpha2(DelegatedAuthorizationV1Alpha2),
    Alpha3(DelegatedAuthorizationV1Alpha3),
}

impl DelegatedAuthorizationDocument {
    pub(super) fn authority(&self) -> &str {
        match self {
            Self::Alpha1(value) => &value.authority,
            Self::Alpha2(value) => &value.authority,
            Self::Alpha3(value) => &value.authority,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AuthorizedDelegatedTarget {
    pub(super) host: String,
    pub(super) execution_profile: String,
    pub(super) max_packet_bytes: usize,
    pub(super) max_managed_payload_tokens: usize,
    pub(super) allow_original_fresh: bool,
    pub(super) allow_finding_resolution_resume: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AuthorizedDelegatedTargetV1Alpha2 {
    pub(super) host: String,
    pub(super) execution_profile: String,
    pub(super) max_packet_bytes: usize,
    pub(super) max_managed_payload_tokens: usize,
    pub(super) max_original_fresh_requests: usize,
    pub(super) max_finding_resolution_resume_requests: usize,
    pub(super) max_total_requests: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AuthorizedDelegatedTargetV1Alpha3 {
    pub(super) host: String,
    pub(super) execution_profile: String,
    pub(super) max_packet_bytes: usize,
    pub(super) max_managed_payload_tokens: usize,
    pub(super) max_original_fresh_requests: usize,
    pub(super) max_finding_resolution_resume_requests: usize,
    pub(super) max_total_requests: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AuthorizedRoute {
    pub(super) provider: String,
    pub(super) account: String,
    pub(super) model: String,
    pub(super) max_packet_bytes: usize,
    pub(super) max_managed_payload_tokens: usize,
    pub(super) allow_original_fresh: bool,
    pub(super) allow_finding_resolution_resume: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct ManifestHeader {
    pub(super) schema: String,
    #[serde(default)]
    pub(super) review_id: Option<String>,
    #[serde(default)]
    pub(super) review_delta_id: Option<String>,
    pub(super) packet: PacketRecord,
    #[serde(default)]
    pub(super) inputs: Option<ManifestInputs>,
}

#[derive(Debug, Deserialize)]
pub(super) struct PacketRecord {
    pub(super) hash: String,
    pub(super) managed_payload_tokens: usize,
}

#[derive(Debug, Deserialize)]
pub(super) struct ManifestInputs {
    #[serde(default)]
    pub(super) prior_manifest: Option<Artifact>,
}

#[derive(Debug, Deserialize)]
pub(super) struct Artifact {
    pub(super) path: String,
    pub(super) hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeliveryReceipt {
    pub(super) schema: String,
    pub(super) review_id: String,
    pub(super) packet_hash: String,
    pub(super) route: Route,
    pub(super) session_id: String,
    pub(super) provider_request_id: String,
    pub(super) provider_request_count: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DelegatedDeliveryReceipt {
    pub(super) schema: String,
    pub(super) review_id: String,
    pub(super) packet_hash: String,
    pub(super) target: DelegatedTarget,
    pub(super) execution_profile: String,
    pub(super) session_id: String,
    pub(super) host_request_id: String,
    pub(super) host_request_count: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct ResultDocument {
    pub(super) schema: &'static str,
    pub(super) ok: bool,
    pub(super) status: &'static str,
    pub(super) next_action: &'static str,
    pub(super) request_id: String,
    pub(super) authorization_id: String,
    pub(super) authority: String,
    pub(super) review_kind: ReviewKind,
    pub(super) review_id: String,
    pub(super) candidate_commit: String,
    pub(super) packet: PacketResult,
    pub(super) route: Route,
    pub(super) session: Session,
    pub(super) limits: DeliveryLimits,
}

#[derive(Debug, Serialize)]
pub(super) struct DelegatedResultDocument {
    pub(super) schema: &'static str,
    pub(super) ok: bool,
    pub(super) status: &'static str,
    pub(super) next_action: &'static str,
    pub(super) request_id: String,
    pub(super) authorization_id: String,
    pub(super) authority: String,
    pub(super) review_kind: ReviewKind,
    pub(super) review_id: String,
    pub(super) candidate_commit: String,
    pub(super) packet: PacketResult,
    pub(super) target: DelegatedTarget,
    pub(super) execution_profile: String,
    pub(super) session: Session,
    pub(super) limits: DelegatedDeliveryLimits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ReviewKind {
    Original,
    FindingResolution,
}

#[derive(Debug, Serialize)]
pub(super) struct PacketResult {
    pub(super) path: String,
    pub(super) hash: String,
    pub(super) bytes: usize,
    pub(super) managed_payload_tokens: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct DeliveryLimits {
    pub(super) provider_requests: usize,
    pub(super) additional_provider_requests: usize,
    pub(super) retries: usize,
    pub(super) steer: usize,
    pub(super) fallback: usize,
    pub(super) second_provider: bool,
    pub(super) tool_execution: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct DelegatedDeliveryLimits {
    pub(super) host_requests: usize,
    pub(super) additional_host_requests: usize,
    pub(super) retries: usize,
    pub(super) steer: usize,
    pub(super) fallback: usize,
    pub(super) target_switch: bool,
}
