use super::{
    METADATA_SUFFIX, PAYLOAD_SUFFIX, PREAMBLE, SECTION_PREFIX, SECTION_SUFFIX,
    capture::Inputs,
    model::{
        DELIVERY_PROFILE, DeliveryProfile, MANIFEST_SCHEMA, Manifest, ManifestInputs,
        NamedArtifact, NamedSemanticInput, PLAN_SCHEMA, PacketRecord, ReviewPlan, SemanticInput,
        TOKENIZER_COMPILER, TOKENIZER_PROFILE,
    },
};
use crate::review_protocol::artifact;

pub(super) fn build_plan(inputs: &Inputs) -> ReviewPlan {
    ReviewPlan {
        schema: PLAN_SCHEMA.to_owned(),
        base_commit: inputs.base_commit.clone(),
        candidate_commit: inputs.candidate_commit.clone(),
        diff_hash: inputs.diff.hash.clone(),
        trusted_commit: inputs.context.result.trusted_commit.clone(),
        active_checkpoint: inputs.context.active_checkpoint.clone(),
        context_build_id: inputs.context.result.build_id.clone(),
        context_request: semantic_input(&inputs.context.request),
        context: semantic_input(&inputs.context.context),
        context_manifest: semantic_input(&inputs.context.manifest),
        required_knowledge_ids: inputs.required_knowledge_ids.clone(),
        repository_authorities: inputs
            .authorities
            .iter()
            .map(|input| SemanticInput {
                path: input.path.clone(),
                hash: input.hash.clone(),
            })
            .collect(),
        slice_contract: semantic_input(&inputs.slice_contract),
        validation_evidence: inputs
            .validation
            .iter()
            .map(|input| NamedSemanticInput {
                name: input.name.clone(),
                path: input.artifact.path.clone(),
                hash: input.artifact.hash.clone(),
            })
            .collect(),
        review_lenses: inputs.lenses.clone(),
        review_questions: inputs.questions.clone(),
        delivery_profile: delivery_profile(),
        tokenizer_profile: TOKENIZER_PROFILE.to_owned(),
        tokenizer_compiler: TOKENIZER_COMPILER.to_owned(),
        max_managed_payload_tokens: inputs.max_tokens,
    }
}

pub(super) fn build_manifest(
    review_id: String,
    plan: ReviewPlan,
    inputs: &Inputs,
    packet_hash: String,
    managed_payload_tokens: usize,
) -> Manifest {
    Manifest {
        schema: MANIFEST_SCHEMA.to_owned(),
        review_id,
        plan,
        inputs: ManifestInputs {
            context_request: artifact(&inputs.context.request),
            context: artifact(&inputs.context.context),
            context_manifest: artifact(&inputs.context.manifest),
            repository_authorities: inputs.authorities.iter().map(artifact).collect(),
            slice_contract: artifact(&inputs.slice_contract),
            validation_evidence: inputs
                .validation
                .iter()
                .map(|input| NamedArtifact {
                    name: input.name.clone(),
                    artifact: artifact(&input.artifact),
                })
                .collect(),
            diff: artifact(&inputs.diff),
        },
        packet: PacketRecord {
            path: "packet.md".to_owned(),
            hash: packet_hash,
            managed_payload_tokens,
            max_managed_payload_tokens: inputs.max_tokens,
        },
    }
}

pub(super) fn semantic_input(input: &crate::review_protocol::Captured) -> SemanticInput {
    SemanticInput {
        path: input.path.clone(),
        hash: input.hash.clone(),
    }
}

pub(super) fn delivery_profile() -> DeliveryProfile {
    DeliveryProfile {
        id: DELIVERY_PROFILE.to_owned(),
        preamble: PREAMBLE.to_owned(),
        section_prefix: SECTION_PREFIX.to_owned(),
        metadata_suffix: METADATA_SUFFIX.to_owned(),
        section_suffix: SECTION_SUFFIX.to_owned(),
        payload_suffix: PAYLOAD_SUFFIX.to_owned(),
    }
}

pub(super) fn delivery_profile_bytes() -> Vec<u8> {
    serde_json::to_vec(&delivery_profile()).expect("closed delivery profile serializes")
}
