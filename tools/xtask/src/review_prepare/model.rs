use serde::{Deserialize, Serialize};

pub(super) const REQUEST_SCHEMA: &str = "yo.slice-review-prepare-request/v1alpha1";
pub(super) const RESULT_SCHEMA: &str = "yo.slice-review-prepare-result/v1alpha1";
pub(super) const REQUEST_SCHEMA_V1_ALPHA2: &str = "yo.slice-review-prepare-request/v1alpha2";
pub(super) const RESULT_SCHEMA_V1_ALPHA2: &str = "yo.slice-review-prepare-result/v1alpha2";
pub(super) const REQUEST_SCHEMA_V1_ALPHA3: &str = "yo.slice-review-prepare-request/v1alpha3";
pub(super) const RESULT_SCHEMA_V1_ALPHA3: &str = "yo.slice-review-prepare-result/v1alpha3";
pub(super) const REQUEST_SCHEMA_V1_ALPHA4: &str = "yo.slice-review-prepare-request/v1alpha4";
pub(super) const RESULT_SCHEMA_V1_ALPHA4: &str = "yo.slice-review-prepare-result/v1alpha4";
pub(super) const REQUEST_SCHEMA_V1_ALPHA5: &str = "yo.slice-review-prepare-request/v1alpha5";
pub(super) const RESULT_SCHEMA_V1_ALPHA5: &str = "yo.slice-review-prepare-result/v1alpha5";
pub(super) const REQUEST_SCHEMA_V1_ALPHA6: &str = "yo.slice-review-prepare-request/v1alpha6";
pub(super) const RESULT_SCHEMA_V1_ALPHA6: &str = "yo.slice-review-prepare-result/v1alpha6";
pub(super) const CONTEXT_SCHEMA: &str = "methexis.context-request/v1alpha1";
pub(super) const REVIEW_SCHEMA: &str = "yo.slice-review-packet-request/v1";
pub(super) const DELIVERY_PROFILE: &str = "yo.slice-review-markdown/v1alpha2";
pub(super) const TOKENIZER_PROFILE: &str = "o200k_base/v1";
pub(super) const MANAGED_EGRESS_SCHEMA: &str = "yo.slice-review-egress-request/v1";
pub(super) const DELEGATED_EGRESS_SCHEMA: &str =
    "yo.slice-review-delegated-egress-request/v1alpha1";
pub(super) const MANAGED_ADMISSION_SCHEMA: &str =
    "yo.external-review-target-admission-request/v1alpha1";
pub(super) const DELEGATED_ADMISSION_SCHEMA: &str =
    "yo.external-review-target-admission-request/v1alpha3";
pub(super) const DELEGATED_PROFILE_ADMISSION_SCHEMA: &str =
    "yo.external-review-target-admission-request/v1alpha4";
pub(super) const MANAGED_DELIVERY_SCHEMA: &str = "yo.slice-review-delivery-request/v1alpha2";
pub(super) const DELEGATED_DELIVERY_SCHEMA: &str =
    "yo.slice-review-delegated-delivery-request/v1alpha2";
pub(super) const MANAGED_USAGE_DELIVERY_SCHEMA: &str = "yo.slice-review-delivery-request/v1alpha4";
pub(super) const DELEGATED_USAGE_DELIVERY_SCHEMA: &str =
    "yo.slice-review-delegated-delivery-request/v1alpha4";
pub(super) const DELEGATED_EXECUTION_PROFILE: &str = "yo.delegated-review-execution/v1alpha1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub(super) schema: String,
    pub(super) slice: String,
    pub(super) knowledge_ids: Vec<String>,
    pub(super) context_max_tokens: usize,
    pub(super) repository_authority_paths: Vec<String>,
    #[serde(default)]
    pub(super) repository_authority_policy: Option<String>,
    pub(super) validation_evidence: Vec<Evidence>,
    pub(super) review_lenses: Vec<String>,
    pub(super) review_questions: Vec<String>,
    pub(super) max_managed_payload_tokens: usize,
    pub(super) target: Target,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Evidence {
    pub(super) name: String,
    pub(super) path: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Target {
    ManagedModel {
        provider: String,
        account: String,
        model: String,
        connection_repository_path: String,
        #[serde(default)]
        session_repository_path: Option<String>,
    },
    DelegatedHost {
        host: String,
        #[serde(default)]
        session_repository_path: Option<String>,
    },
}

#[derive(Serialize)]
pub(super) struct ContextRequest<'a> {
    pub(super) schema: &'static str,
    pub(super) anchors: Vec<ContextAnchor<'a>>,
    pub(super) tokenizer_profile: &'static str,
    pub(super) max_tokens: usize,
}

#[derive(Serialize)]
pub(super) struct ContextAnchor<'a> {
    pub(super) kind: &'static str,
    pub(super) value: &'a str,
}

#[derive(Serialize)]
pub(super) struct ReviewRequest<'a> {
    pub(super) schema: &'static str,
    pub(super) context_request_path: &'a str,
    pub(super) required_knowledge_ids: &'a [String],
    pub(super) slice_contract_path: String,
    pub(super) repository_authority_paths: &'a [String],
    pub(super) validation_evidence: &'a [Evidence],
    pub(super) review_lenses: &'a [String],
    pub(super) review_questions: &'a [String],
    pub(super) delivery_profile: &'static str,
    pub(super) tokenizer_profile: &'static str,
    pub(super) max_managed_payload_tokens: usize,
}

#[derive(Serialize)]
pub(super) struct ManagedEgress<'a> {
    pub(super) schema: &'static str,
    pub(super) manifest_path: &'a str,
    pub(super) manifest_hash: &'a str,
    pub(super) authorization_hash: &'a str,
    pub(super) route: ManagedRoute<'a>,
    pub(super) session: FreshSession,
}

#[derive(Serialize)]
pub(super) struct ManagedRoute<'a> {
    pub(super) provider: &'a str,
    pub(super) account: &'a str,
    pub(super) model: &'a str,
}

#[derive(Serialize)]
pub(super) struct DelegatedEgress<'a> {
    pub(super) schema: &'static str,
    pub(super) manifest_path: &'a str,
    pub(super) manifest_hash: &'a str,
    pub(super) authorization_hash: &'a str,
    pub(super) target: DelegatedTarget<'a>,
    pub(super) execution_profile: &'static str,
    pub(super) session: FreshSession,
}

#[derive(Serialize)]
pub(super) struct DelegatedTarget<'a> {
    pub(super) kind: &'static str,
    pub(super) host: &'a str,
}

#[derive(Serialize)]
pub(super) struct FreshSession {
    pub(super) mode: &'static str,
}

#[derive(Serialize)]
pub(super) struct ManagedAdmission<'a> {
    pub(super) schema: &'static str,
    pub(super) target: ManagedAdmissionTarget<'a>,
    pub(super) connection_repository_path: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) session_repository_path: Option<&'a str>,
}

#[derive(Serialize)]
pub(super) struct ManagedAdmissionTarget<'a> {
    pub(super) kind: &'static str,
    pub(super) provider: &'a str,
    pub(super) account: &'a str,
    pub(super) model: &'a str,
}

#[derive(Serialize)]
pub(super) struct DelegatedAdmission<'a> {
    pub(super) schema: &'static str,
    pub(super) target: DelegatedTarget<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) session_repository_path: Option<&'a str>,
}

#[derive(Serialize)]
pub(super) struct DeliveryRequest<'a> {
    pub(super) schema: &'static str,
    pub(super) egress_request_path: &'a str,
    pub(super) egress_request_hash: &'a str,
    pub(super) admission_request_path: &'a str,
    pub(super) admission_request_hash: &'a str,
    pub(super) output_directory: &'a str,
}
