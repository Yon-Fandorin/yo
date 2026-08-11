use serde::{Deserialize, Serialize};

pub(super) use crate::review_protocol::{
    Artifact, ArtifactWithTokens, DeliveryProfile, EvidenceRequest, NamedArtifact,
    NamedSemanticInput, PacketRecord, TOKENIZER_COMPILER, TOKENIZER_PROFILE,
};

pub(super) const REQUEST_SCHEMA: &str = "yo.slice-review-packet-request/v1";
pub(super) const PLAN_SCHEMA: &str = "yo.slice-review-plan/v1";
pub(super) const MANIFEST_SCHEMA: &str = "yo.slice-review-manifest/v1";
pub(super) const RESULT_SCHEMA: &str = "yo.slice-review-packet-result/v1";
pub(super) const PREFLIGHT_RESULT_SCHEMA: &str = "yo.slice-review-packet-preflight-result/v1";
pub(super) const READINESS_RESULT_SCHEMA: &str = "yo.slice-review-request-readiness-result/v1";
pub(super) const SECTION_TOKEN_ACCOUNTING: &str = "independently-tokenized-non-additive/v1";
pub(super) const DELIVERY_PROFILE: &str = "yo.slice-review-markdown/v1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub(super) schema: String,
    pub(super) context_request_path: String,
    pub(super) required_knowledge_ids: Vec<String>,
    pub(super) slice_contract_path: String,
    pub(super) repository_authority_paths: Vec<String>,
    pub(super) validation_evidence: Vec<EvidenceRequest>,
    pub(super) review_lenses: Vec<String>,
    pub(super) review_questions: Vec<String>,
    pub(super) delivery_profile: String,
    pub(super) tokenizer_profile: String,
    pub(super) max_managed_payload_tokens: usize,
}

#[derive(Debug, Deserialize)]
pub(super) struct ContextResult {
    pub(super) schema: String,
    pub(super) ok: bool,
    pub(super) operation: String,
    pub(super) authority: String,
    pub(super) trusted_commit: String,
    pub(super) build_id: String,
    pub(super) context: Artifact,
    pub(super) manifest: Artifact,
}

#[derive(Debug, Deserialize)]
pub(super) struct ContextManifest {
    pub(super) schema: String,
    pub(super) build_id: String,
    pub(super) plan: ContextPlan,
    pub(super) context: Artifact,
}

#[derive(Debug, Deserialize)]
pub(super) struct ContextPlan {
    pub(super) checkpoint: CheckpointIdentity,
    pub(super) units: Vec<ContextUnit>,
    pub(super) tokenizer_profile: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ContextUnit {
    pub(super) id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct CheckpointIdentity {
    pub(super) id: String,
    pub(super) hash: String,
    pub(super) authority_basis_commit: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct ReviewPlan {
    pub(super) schema: String,
    pub(super) base_commit: String,
    pub(super) candidate_commit: String,
    pub(super) diff_hash: String,
    pub(super) trusted_commit: String,
    pub(super) active_checkpoint: CheckpointIdentity,
    pub(super) context_build_id: String,
    pub(super) context_request: SemanticInput,
    pub(super) context: SemanticInput,
    pub(super) context_manifest: SemanticInput,
    pub(super) required_knowledge_ids: Vec<String>,
    pub(super) repository_authorities: Vec<SemanticInput>,
    pub(super) slice_contract: SemanticInput,
    pub(super) validation_evidence: Vec<NamedSemanticInput>,
    pub(super) review_lenses: Vec<String>,
    pub(super) review_questions: Vec<String>,
    pub(super) delivery_profile: DeliveryProfile,
    pub(super) tokenizer_profile: String,
    pub(super) tokenizer_compiler: String,
    pub(super) max_managed_payload_tokens: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct SemanticInput {
    pub(super) path: String,
    pub(super) hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct Manifest {
    pub(super) schema: String,
    pub(super) review_id: String,
    pub(super) plan: ReviewPlan,
    pub(super) inputs: ManifestInputs,
    pub(super) packet: PacketRecord,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct ManifestInputs {
    pub(super) context_request: Artifact,
    pub(super) context: Artifact,
    pub(super) context_manifest: Artifact,
    pub(super) repository_authorities: Vec<Artifact>,
    pub(super) slice_contract: Artifact,
    pub(super) validation_evidence: Vec<NamedArtifact>,
    pub(super) diff: Artifact,
}

#[derive(Debug, Serialize)]
pub(super) struct ResultRecord {
    pub(super) schema: &'static str,
    pub(super) ok: bool,
    pub(super) operation: &'static str,
    pub(super) status: &'static str,
    pub(super) review_id: String,
    pub(super) trusted_commit: String,
    pub(super) candidate_commit: String,
    pub(super) packet: ArtifactWithTokens,
    pub(super) manifest: Artifact,
    pub(super) max_managed_payload_tokens: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct PreflightResultRecord {
    pub(super) schema: &'static str,
    pub(super) ok: bool,
    pub(super) operation: &'static str,
    pub(super) status: &'static str,
    pub(super) artifacts_published: bool,
    pub(super) review_id: String,
    pub(super) trusted_commit: String,
    pub(super) candidate_commit: String,
    pub(super) packet: PreflightPacket,
    pub(super) section_token_accounting: &'static str,
    pub(super) sections: Vec<PreflightSection>,
}

#[derive(Debug, Serialize)]
pub(super) struct PreflightPacket {
    pub(super) bytes: usize,
    pub(super) managed_payload_tokens: usize,
    pub(super) max_managed_payload_tokens: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct PreflightSection {
    pub(super) kind: String,
    pub(super) name: String,
    pub(super) path: String,
    pub(super) hash: String,
    pub(super) content_bytes: usize,
    pub(super) content_tokens_independent: usize,
    pub(super) rendered_bytes: usize,
    pub(super) rendered_tokens_independent: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct ReadinessResultRecord {
    pub(super) schema: &'static str,
    pub(super) ok: bool,
    pub(super) operation: &'static str,
    pub(super) status: &'static str,
    pub(super) artifacts_published: bool,
    pub(super) slice: String,
    pub(super) base_commit: String,
    pub(super) trusted_commit: String,
    pub(super) candidate_commit: String,
    pub(super) request: Artifact,
    pub(super) slice_contract: Artifact,
    pub(super) context_request: Artifact,
    pub(super) required_knowledge_id_count: usize,
    pub(super) repository_authority_count: usize,
    pub(super) validation_evidence_count: usize,
    pub(super) review_lens_count: usize,
    pub(super) review_question_count: usize,
}
