use serde::{Deserialize, Serialize};

pub(super) use crate::review_protocol::{
    Artifact, ArtifactWithTokens, DeliveryProfile, EvidenceRequest, NamedArtifact,
    NamedSemanticInput, PacketRecord, TOKENIZER_COMPILER, TOKENIZER_PROFILE,
};

pub(super) const REQUEST_SCHEMA: &str = "yo.slice-review-delta-request/v1";
pub(super) const PLAN_SCHEMA: &str = "yo.slice-review-delta-plan/v1";
pub(super) const MANIFEST_SCHEMA: &str = "yo.slice-review-delta-manifest/v1";
pub(super) const RESULT_SCHEMA: &str = "yo.slice-review-delta-result/v1";
pub(super) const PRIOR_FINDINGS_SCHEMA: &str = "yo.slice-review-findings/v1";
pub(super) const DELIVERY_PROFILE: &str = "yo.slice-review-delta-markdown/v1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub(super) schema: String,
    pub(super) prior_manifest_path: String,
    pub(super) prior_manifest_hash: String,
    pub(super) prior_findings_path: String,
    pub(super) prior_findings_hash: String,
    pub(super) finding_dispositions: Vec<FindingDisposition>,
    pub(super) reused_validation_evidence: Vec<String>,
    pub(super) affected_validation_evidence: Vec<EvidenceRequest>,
    pub(super) delivery_profile: String,
    pub(super) tokenizer_profile: String,
    pub(super) max_managed_payload_tokens: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FindingDisposition {
    pub(super) finding_id: String,
    pub(super) disposition: Disposition,
    pub(super) summary: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Disposition {
    Resolved,
    NotReproduced,
    AcceptedLimit,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PriorFindings {
    pub(super) schema: String,
    pub(super) review_id: String,
    pub(super) candidate_commit: String,
    pub(super) findings: Vec<PriorFinding>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PriorFinding {
    pub(super) finding_id: String,
    pub(super) summary: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct ReviewDeltaPlan {
    pub(super) schema: String,
    pub(super) prior_review_id: String,
    pub(super) prior_manifest_hash: String,
    pub(super) prior_packet_hash: String,
    pub(super) prior_findings: Artifact,
    pub(super) prior_candidate_commit: String,
    pub(super) replacement_candidate_commit: String,
    pub(super) delta_hash: String,
    pub(super) trusted_commit: String,
    pub(super) slice_contract: Artifact,
    pub(super) finding_dispositions: Vec<FindingDisposition>,
    pub(super) reused_validation_evidence: Vec<NamedSemanticInput>,
    pub(super) affected_validation_evidence: Vec<NamedSemanticInput>,
    pub(super) review_lenses: Vec<String>,
    pub(super) review_questions: Vec<String>,
    pub(super) delivery_profile: DeliveryProfile,
    pub(super) tokenizer_profile: String,
    pub(super) tokenizer_compiler: String,
    pub(super) max_managed_payload_tokens: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct Manifest {
    pub(super) schema: String,
    pub(super) review_delta_id: String,
    pub(super) plan: ReviewDeltaPlan,
    pub(super) inputs: ManifestInputs,
    pub(super) packet: PacketRecord,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct ManifestInputs {
    pub(super) prior_manifest: Artifact,
    pub(super) prior_packet: Artifact,
    pub(super) prior_findings: Artifact,
    pub(super) slice_contract: Artifact,
    pub(super) reused_validation_evidence: Vec<NamedArtifact>,
    pub(super) affected_validation_evidence: Vec<NamedArtifact>,
    pub(super) delta: Artifact,
}

#[derive(Debug, Serialize)]
pub(super) struct ResultRecord {
    pub(super) schema: &'static str,
    pub(super) ok: bool,
    pub(super) operation: &'static str,
    pub(super) status: &'static str,
    pub(super) review_delta_id: String,
    pub(super) prior_review_id: String,
    pub(super) prior_candidate_commit: String,
    pub(super) replacement_candidate_commit: String,
    pub(super) packet: ArtifactWithTokens,
    pub(super) manifest: Artifact,
    pub(super) max_managed_payload_tokens: usize,
}
