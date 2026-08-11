use serde::Serialize;

use super::{
    METADATA_SUFFIX, PAYLOAD_SUFFIX, PREAMBLE, SECTION_PREFIX, SECTION_SUFFIX, capture::Inputs,
    model::ReviewPlan,
};
use crate::review_protocol::digest;

pub(super) fn render_packet(
    review_id: &str,
    plan: &ReviewPlan,
    inputs: &Inputs,
) -> Result<Vec<u8>, String> {
    let mut packet = PREAMBLE.as_bytes().to_vec();
    let plan_bytes = serde_json::to_vec_pretty(plan).expect("closed review plan serializes");
    append_section(&mut packet, "review_plan", review_id, "", &plan_bytes)?;
    append_section(
        &mut packet,
        "context_request",
        "context-request",
        &inputs.context.request.path,
        &inputs.context.request.bytes,
    )?;
    append_section(
        &mut packet,
        "context_manifest",
        &inputs.context.result.build_id,
        &inputs.context.manifest.path,
        &inputs.context.manifest.bytes,
    )?;
    append_section(
        &mut packet,
        "context",
        &inputs.context.result.build_id,
        &inputs.context.context.path,
        &inputs.context.context.bytes,
    )?;
    for authority in &inputs.authorities {
        append_section(
            &mut packet,
            "repository_authority",
            &authority.path,
            &authority.path,
            &authority.bytes,
        )?;
    }
    append_section(
        &mut packet,
        "slice_contract",
        "slice-contract",
        &inputs.slice_contract.path,
        &inputs.slice_contract.bytes,
    )?;
    for evidence in &inputs.validation {
        append_section(
            &mut packet,
            "validation_evidence",
            &evidence.name,
            &evidence.artifact.path,
            &evidence.artifact.bytes,
        )?;
    }
    let instructions = serde_json::to_vec_pretty(&serde_json::json!({
        "review_lenses": inputs.lenses,
        "review_questions": inputs.questions,
    }))
    .expect("review instructions serialize");
    append_section(
        &mut packet,
        "review_instructions",
        "requested-review",
        "",
        &instructions,
    )?;
    append_section(
        &mut packet,
        "git_diff",
        "base-to-candidate",
        &inputs.diff.path,
        &inputs.diff.bytes,
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

pub(super) fn append_section(
    output: &mut Vec<u8>,
    kind: &str,
    name: &str,
    path: &str,
    bytes: &[u8],
) -> Result<(), String> {
    std::str::from_utf8(bytes)
        .map_err(|_| format!("review section `{name}` is not UTF-8 model-visible text"))?;
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

pub(super) fn count_tokens(bytes: &[u8]) -> Result<usize, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "canonical review packet is not UTF-8".to_owned())?;
    Ok(tiktoken_rs::o200k_base_singleton()
        .encode_ordinary(text)
        .len())
}

pub(super) fn require_budget(actual: usize, maximum: usize) -> Result<(), String> {
    if actual <= maximum {
        Ok(())
    } else {
        Err(format!(
            "managed payload requires {actual} tokens but the budget is {maximum}; no content was truncated"
        ))
    }
}
