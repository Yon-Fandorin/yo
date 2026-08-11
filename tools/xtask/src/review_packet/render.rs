use serde::Serialize;

use super::{
    METADATA_SUFFIX, PAYLOAD_SUFFIX, PREAMBLE, SECTION_PREFIX, SECTION_SUFFIX,
    capture::Inputs,
    model::{PreflightSection, ReviewPlan},
};
use crate::review_protocol::digest;

pub(super) struct RenderedPacket {
    pub(super) bytes: Vec<u8>,
    pub(super) sections: Vec<PreflightSection>,
}

pub(super) fn render_packet(
    review_id: &str,
    plan: &ReviewPlan,
    inputs: &Inputs,
) -> Result<Vec<u8>, String> {
    Ok(render_packet_inner(review_id, plan, inputs, false)?.bytes)
}

pub(super) fn render_packet_with_measurements(
    review_id: &str,
    plan: &ReviewPlan,
    inputs: &Inputs,
) -> Result<RenderedPacket, String> {
    render_packet_inner(review_id, plan, inputs, true)
}

fn render_packet_inner(
    review_id: &str,
    plan: &ReviewPlan,
    inputs: &Inputs,
    measure: bool,
) -> Result<RenderedPacket, String> {
    let mut packet = PREAMBLE.as_bytes().to_vec();
    let mut sections = Vec::new();
    let plan_bytes = serde_json::to_vec_pretty(plan).expect("closed review plan serializes");
    append_review_section(
        &mut packet,
        &mut sections,
        measure,
        "review_plan",
        review_id,
        "",
        &plan_bytes,
    )?;
    append_review_section(
        &mut packet,
        &mut sections,
        measure,
        "context_request",
        "context-request",
        &inputs.context.request.path,
        &inputs.context.request.bytes,
    )?;
    append_review_section(
        &mut packet,
        &mut sections,
        measure,
        "context_manifest",
        &inputs.context.result.build_id,
        &inputs.context.manifest.path,
        &inputs.context.manifest.bytes,
    )?;
    append_review_section(
        &mut packet,
        &mut sections,
        measure,
        "context",
        &inputs.context.result.build_id,
        &inputs.context.context.path,
        &inputs.context.context.bytes,
    )?;
    for authority in &inputs.authorities {
        append_review_section(
            &mut packet,
            &mut sections,
            measure,
            "repository_authority",
            &authority.path,
            &authority.path,
            &authority.bytes,
        )?;
    }
    append_review_section(
        &mut packet,
        &mut sections,
        measure,
        "slice_contract",
        "slice-contract",
        &inputs.slice_contract.path,
        &inputs.slice_contract.bytes,
    )?;
    for evidence in &inputs.validation {
        append_review_section(
            &mut packet,
            &mut sections,
            measure,
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
    append_review_section(
        &mut packet,
        &mut sections,
        measure,
        "review_instructions",
        "requested-review",
        "",
        &instructions,
    )?;
    append_review_section(
        &mut packet,
        &mut sections,
        measure,
        "git_diff",
        "base-to-candidate",
        &inputs.diff.path,
        &inputs.diff.bytes,
    )?;
    packet.extend_from_slice(PAYLOAD_SUFFIX.as_bytes());
    Ok(RenderedPacket {
        bytes: packet,
        sections,
    })
}

fn append_review_section(
    output: &mut Vec<u8>,
    sections: &mut Vec<PreflightSection>,
    measure: bool,
    kind: &str,
    name: &str,
    path: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let start = output.len();
    append_section(output, kind, name, path, bytes)?;
    if measure {
        sections.push(PreflightSection {
            kind: kind.to_owned(),
            name: name.to_owned(),
            path: path.to_owned(),
            hash: digest(bytes),
            content_bytes: bytes.len(),
            content_tokens_independent: count_tokens(bytes)?,
            rendered_bytes: output.len() - start,
            rendered_tokens_independent: count_tokens(&output[start..])?,
        });
    }
    Ok(())
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
