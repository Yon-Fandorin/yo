use super::{
    capture::Inputs,
    model::{
        DELIVERY_PROFILE_V1, DELIVERY_PROFILE_V1_ALPHA1, DELIVERY_PROFILE_V1_ALPHA2,
        DELIVERY_PROFILE_V1_ALPHA3, DeliveryProfile, InputPrefixRecord, MANIFEST_SCHEMA_V1,
        MANIFEST_SCHEMA_V1_ALPHA1, MANIFEST_SCHEMA_V1_ALPHA2, MANIFEST_SCHEMA_V1_ALPHA3, Manifest,
        ManifestInputs, NamedArtifact, NamedSemanticInput, PLAN_SCHEMA, PLAN_SCHEMA_V1_ALPHA3,
        PacketRecord, ProspectiveActivationPlan, ProspectiveManifestInputs, ReviewPlan,
        SemanticInput, TOKENIZER_COMPILER, TOKENIZER_PROFILE,
    },
};
use crate::review_protocol::artifact;

pub(super) fn build_plan(inputs: &Inputs) -> ReviewPlan {
    let prospective = inputs.prospective.as_ref();
    ReviewPlan {
        schema: if prospective.is_some() {
            PLAN_SCHEMA_V1_ALPHA3
        } else {
            PLAN_SCHEMA
        }
        .to_owned(),
        base_commit: inputs.base_commit.clone(),
        candidate_commit: inputs.candidate_commit.clone(),
        diff_hash: inputs.diff.hash.clone(),
        trusted_commit: inputs.context.result.trusted_commit.clone(),
        authority_mode: prospective.map(|_| "prospective".to_owned()),
        active_checkpoint: prospective
            .is_none()
            .then(|| inputs.context.active_checkpoint.clone()),
        prospective_checkpoint: prospective
            .is_some()
            .then(|| inputs.context.active_checkpoint.clone()),
        prospective_activation: prospective.map(|proposal| ProspectiveActivationPlan {
            activation_request: semantic_input(&proposal.activation_request),
            proposed_checkpoint: semantic_input(&proposal.proposed_checkpoint),
            proposed_active_record: semantic_input(&proposal.proposed_active_record),
            predecessor_active_record_hash: proposal.predecessor_active_record_hash.clone(),
        }),
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
        delivery_profile: serde_json::from_slice(&inputs.delivery_profile_bytes)
            .expect("captured delivery profile bytes deserialize"),
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
    input_prefix: Option<InputPrefixRecord>,
) -> Manifest {
    let schema = match plan.delivery_profile.id.as_str() {
        DELIVERY_PROFILE_V1 => MANIFEST_SCHEMA_V1,
        DELIVERY_PROFILE_V1_ALPHA1 => MANIFEST_SCHEMA_V1_ALPHA1,
        DELIVERY_PROFILE_V1_ALPHA2 => MANIFEST_SCHEMA_V1_ALPHA2,
        DELIVERY_PROFILE_V1_ALPHA3 => MANIFEST_SCHEMA_V1_ALPHA3,
        _ => unreachable!("validated original review delivery profile"),
    };
    Manifest {
        schema: schema.to_owned(),
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
            prospective_activation: inputs.prospective.as_ref().map(|proposal| {
                ProspectiveManifestInputs {
                    activation_request: artifact(&proposal.activation_request),
                    proposed_checkpoint: artifact(&proposal.proposed_checkpoint),
                    proposed_active_record: artifact(&proposal.proposed_active_record),
                }
            }),
        },
        packet: PacketRecord {
            path: "packet.md".to_owned(),
            hash: packet_hash,
            managed_payload_tokens,
            max_managed_payload_tokens: inputs.max_tokens,
        },
        input_prefix,
    }
}

pub(super) fn semantic_input(input: &crate::review_protocol::Captured) -> SemanticInput {
    SemanticInput {
        path: input.path.clone(),
        hash: input.hash.clone(),
    }
}

pub(super) fn delivery_profile_for_id(id: &str) -> Result<DeliveryProfile, String> {
    let preamble = match id {
        DELIVERY_PROFILE_V1 => super::PREAMBLE,
        DELIVERY_PROFILE_V1_ALPHA1 => super::PREAMBLE_V1_ALPHA1,
        DELIVERY_PROFILE_V1_ALPHA2 => super::PREAMBLE_V1_ALPHA2,
        DELIVERY_PROFILE_V1_ALPHA3 => super::PREAMBLE_V1_ALPHA3,
        _ => {
            return Err(format!(
                "unsupported original review delivery profile `{id}`"
            ));
        },
    };
    Ok(DeliveryProfile {
        id: id.to_owned(),
        preamble: preamble.to_owned(),
        section_prefix: super::SECTION_PREFIX.to_owned(),
        metadata_suffix: super::METADATA_SUFFIX.to_owned(),
        section_suffix: super::SECTION_SUFFIX.to_owned(),
        payload_suffix: super::PAYLOAD_SUFFIX.to_owned(),
    })
}

#[cfg(test)]
fn delivery_profile_v1() -> DeliveryProfile {
    delivery_profile_for_id(DELIVERY_PROFILE_V1)
        .expect("frozen original review v1 delivery profile is supported")
}

pub(super) fn delivery_profile_bytes_for_id(id: &str) -> Result<Vec<u8>, String> {
    Ok(serde_json::to_vec(&delivery_profile_for_id(id)?)
        .expect("closed delivery profile serializes"))
}

#[cfg(test)]
pub(super) fn delivery_profile_v1_bytes() -> Vec<u8> {
    serde_json::to_vec(&delivery_profile_v1()).expect("closed v1 delivery profile serializes")
}
