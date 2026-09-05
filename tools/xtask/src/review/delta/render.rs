use serde::Serialize;

use super::{
    Inputs, METADATA_SUFFIX, PAYLOAD_SUFFIX, PREAMBLE, SECTION_PREFIX, SECTION_SUFFIX,
    WireContract,
    model::{
        DeliveryProfile, Manifest, ManifestInputs, NamedArtifact, NamedSemanticInput, PacketRecord,
        ReviewDeltaPlan, TOKENIZER_COMPILER, TOKENIZER_PROFILE,
    },
};
use crate::review_protocol::{NamedCaptured, artifact, digest};

pub(super) fn build_plan_for(inputs: &Inputs, contract: WireContract) -> ReviewDeltaPlan {
    ReviewDeltaPlan {
        schema: contract.plan_schema.to_owned(),
        prior_review_id: inputs.prior.review_id.clone(),
        prior_manifest_hash: inputs.prior_manifest.hash.clone(),
        prior_packet_hash: inputs.prior_packet.hash.clone(),
        prior_findings: artifact(&inputs.prior_findings),
        prior_candidate_commit: inputs.prior.candidate_commit.clone(),
        replacement_candidate_commit: inputs.replacement_candidate.clone(),
        delta_hash: inputs.delta.hash.clone(),
        trusted_commit: inputs.prior.trusted_commit.clone(),
        slice_contract: artifact(&inputs.slice_contract),
        finding_dispositions: inputs.findings.clone(),
        reused_validation_evidence: inputs
            .reused_validation
            .iter()
            .map(named_semantic_input)
            .collect(),
        affected_validation_evidence: inputs
            .affected_validation
            .iter()
            .map(named_semantic_input)
            .collect(),
        review_lenses: inputs.prior.review_lenses.clone(),
        review_questions: inputs.prior.review_questions.clone(),
        delivery_profile: delivery_profile_for(contract),
        tokenizer_profile: TOKENIZER_PROFILE.to_owned(),
        tokenizer_compiler: TOKENIZER_COMPILER.to_owned(),
        max_managed_payload_tokens: inputs.max_tokens,
    }
}

pub(super) fn render_packet(
    review_delta_id: &str,
    plan: &ReviewDeltaPlan,
    inputs: &Inputs,
) -> Result<Vec<u8>, String> {
    let mut packet = PREAMBLE.as_bytes().to_vec();
    let plan_bytes = serde_json::to_vec_pretty(plan).expect("closed delta plan serializes");
    append_section(
        &mut packet,
        "review_delta_plan",
        review_delta_id,
        "",
        &plan_bytes,
    )?;
    let findings =
        serde_json::to_vec_pretty(&inputs.findings).expect("finding dispositions serialize");
    append_section(
        &mut packet,
        "prior_findings",
        "exact-prior-finding-set",
        &inputs.prior_findings.path,
        &inputs.prior_findings.bytes,
    )?;
    append_section(
        &mut packet,
        "finding_dispositions",
        "exact-finding-dispositions",
        "",
        &findings,
    )?;
    let reused = serde_json::to_vec_pretty(
        &inputs
            .reused_validation
            .iter()
            .map(named_artifact)
            .collect::<Vec<_>>(),
    )
    .expect("reused evidence records serialize");
    append_section(
        &mut packet,
        "reused_validation_evidence",
        "unchanged-green-evidence",
        "",
        &reused,
    )?;
    for evidence in &inputs.affected_validation {
        append_section(
            &mut packet,
            "affected_validation_evidence",
            &evidence.name,
            &evidence.artifact.path,
            &evidence.artifact.bytes,
        )?;
    }
    append_section(
        &mut packet,
        "git_delta",
        "prior-to-replacement-candidate",
        &inputs.delta.path,
        &inputs.delta.bytes,
    )?;
    packet.extend_from_slice(PAYLOAD_SUFFIX.as_bytes());
    Ok(packet)
}

#[derive(Serialize)]
struct SectionMetadata<'a> {
    kind: &'a str,
    name: &'a str,
    path: &'a str,
    hash: String,
    bytes: usize,
}

fn append_section(
    output: &mut Vec<u8>,
    kind: &str,
    name: &str,
    path: &str,
    bytes: &[u8],
) -> Result<(), String> {
    std::str::from_utf8(bytes)
        .map_err(|_| format!("review delta section `{name}` is not UTF-8 model-visible text"))?;
    let metadata = serde_json::to_vec(&SectionMetadata {
        kind,
        name,
        path,
        hash: digest(bytes),
        bytes: bytes.len(),
    })
    .expect("section metadata serializes");
    output.extend_from_slice(SECTION_PREFIX.as_bytes());
    output.extend_from_slice(&metadata);
    output.extend_from_slice(METADATA_SUFFIX.as_bytes());
    output.extend_from_slice(bytes);
    output.extend_from_slice(SECTION_SUFFIX.as_bytes());
    Ok(())
}

pub(super) fn build_manifest_for(
    review_delta_id: String,
    plan: ReviewDeltaPlan,
    inputs: &Inputs,
    packet_hash: String,
    managed_payload_tokens: usize,
    contract: WireContract,
) -> Manifest {
    Manifest {
        schema: contract.manifest_schema.to_owned(),
        review_delta_id,
        plan,
        inputs: ManifestInputs {
            prior_manifest: artifact(&inputs.prior_manifest),
            prior_packet: artifact(&inputs.prior_packet),
            prior_findings: artifact(&inputs.prior_findings),
            slice_contract: artifact(&inputs.slice_contract),
            reused_validation_evidence: inputs
                .reused_validation
                .iter()
                .map(named_artifact)
                .collect(),
            affected_validation_evidence: inputs
                .affected_validation
                .iter()
                .map(named_artifact)
                .collect(),
            delta: artifact(&inputs.delta),
        },
        packet: PacketRecord {
            path: "packet.md".to_owned(),
            hash: packet_hash,
            managed_payload_tokens,
            max_managed_payload_tokens: inputs.max_tokens,
        },
    }
}

pub(super) fn delivery_profile_for(contract: WireContract) -> DeliveryProfile {
    DeliveryProfile {
        id: contract.delivery_profile.to_owned(),
        preamble: PREAMBLE.to_owned(),
        section_prefix: SECTION_PREFIX.to_owned(),
        metadata_suffix: METADATA_SUFFIX.to_owned(),
        section_suffix: SECTION_SUFFIX.to_owned(),
        payload_suffix: PAYLOAD_SUFFIX.to_owned(),
    }
}

pub(super) fn delivery_profile_bytes_for(contract: WireContract) -> Vec<u8> {
    serde_json::to_vec(&delivery_profile_for(contract)).expect("closed delivery profile serializes")
}

pub(super) fn count_tokens(bytes: &[u8]) -> Result<usize, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "canonical review delta packet is not UTF-8".to_owned())?;
    Ok(tiktoken_rs::o200k_base_singleton()
        .encode_ordinary(text)
        .len())
}

pub(super) fn require_budget(actual: usize, maximum: usize) -> Result<(), String> {
    if actual <= maximum {
        Ok(())
    } else {
        Err(format!(
            "managed delta payload requires {actual} tokens but the budget is {maximum}; no content was truncated"
        ))
    }
}

pub(super) fn named_artifact(input: &NamedCaptured) -> NamedArtifact {
    NamedArtifact {
        name: input.name.clone(),
        artifact: artifact(&input.artifact),
    }
}

pub(super) fn named_semantic_input(input: &NamedCaptured) -> NamedSemanticInput {
    NamedSemanticInput {
        name: input.name.clone(),
        path: input.artifact.path.clone(),
        hash: input.artifact.hash.clone(),
    }
}
