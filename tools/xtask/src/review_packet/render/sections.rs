use serde::Serialize;

#[cfg(test)]
use super::super::{METADATA_SUFFIX, SECTION_PREFIX, SECTION_SUFFIX};
use super::super::{
    capture::Inputs,
    model::{
        DELIVERY_PROFILE_V1_ALPHA2, DELIVERY_PROFILE_V1_ALPHA3, PreflightSection, ReviewPlan,
        SENTINEL_ESCAPE_PROFILE,
    },
};
use crate::review_protocol::{DeliveryProfile, digest};

pub(super) fn append_plan(
    packet: &mut Vec<u8>,
    sections: &mut Vec<PreflightSection>,
    measure: bool,
    profile: &DeliveryProfile,
    review_id: &str,
    plan: &ReviewPlan,
) -> Result<(), String> {
    let plan_bytes = serde_json::to_vec_pretty(plan).expect("closed review plan serializes");
    SectionSink::new(packet, sections, measure, profile).append(
        "review_plan",
        review_id,
        "",
        &plan_bytes,
    )
}

pub(super) enum ContextPaths {
    Physical,
    Logical,
}

pub(super) fn append_context(
    packet: &mut Vec<u8>,
    sections: &mut Vec<PreflightSection>,
    measure: bool,
    profile: &DeliveryProfile,
    inputs: &Inputs,
    paths: ContextPaths,
) -> Result<(), String> {
    let (request_path, manifest_path, context_path) = match paths {
        ContextPaths::Physical => (
            inputs.context.request.path.as_str(),
            inputs.context.manifest.path.as_str(),
            inputs.context.context.path.as_str(),
        ),
        ContextPaths::Logical => (
            "context-request.json",
            "context-manifest.json",
            "context.md",
        ),
    };
    let mut sink = SectionSink::new(packet, sections, measure, profile);
    sink.append(
        "context_request",
        "context-request",
        request_path,
        &inputs.context.request.bytes,
    )?;
    sink.append(
        "context_manifest",
        &inputs.context.result.build_id,
        manifest_path,
        &inputs.context.manifest.bytes,
    )?;
    sink.append(
        "context",
        &inputs.context.result.build_id,
        context_path,
        &inputs.context.context.bytes,
    )?;
    if let Some(proposal) = &inputs.prospective {
        sink.append(
            "prospective_activation_request",
            "activation-request",
            &proposal.activation_request.path,
            &proposal.activation_request.bytes,
        )?;
        sink.append(
            "prospective_checkpoint",
            "proposed-checkpoint",
            &proposal.proposed_checkpoint.path,
            &proposal.proposed_checkpoint.bytes,
        )?;
        sink.append(
            "prospective_active_record",
            "proposed-active-record",
            &proposal.proposed_active_record.path,
            &proposal.proposed_active_record.bytes,
        )?;
    }
    Ok(())
}

pub(super) fn append_authorities(
    packet: &mut Vec<u8>,
    sections: &mut Vec<PreflightSection>,
    measure: bool,
    profile: &DeliveryProfile,
    inputs: &Inputs,
) -> Result<(), String> {
    let mut sink = SectionSink::new(packet, sections, measure, profile);
    for authority in &inputs.authorities {
        sink.append(
            "repository_authority",
            &authority.path,
            &authority.path,
            &authority.bytes,
        )?;
    }
    Ok(())
}

pub(super) fn append_candidate_suffix(
    packet: &mut Vec<u8>,
    sections: &mut Vec<PreflightSection>,
    measure: bool,
    profile: &DeliveryProfile,
    inputs: &Inputs,
) -> Result<(), String> {
    let mut sink = SectionSink::new(packet, sections, measure, profile);
    sink.append(
        "slice_contract",
        "slice-contract",
        &inputs.slice_contract.path,
        &inputs.slice_contract.bytes,
    )?;
    for evidence in &inputs.validation {
        sink.append(
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
    sink.append("review_instructions", "requested-review", "", &instructions)?;
    sink.append(
        "git_diff",
        "base-to-candidate",
        &inputs.diff.path,
        &inputs.diff.bytes,
    )
}

struct SectionSink<'a> {
    output: &'a mut Vec<u8>,
    sections: &'a mut Vec<PreflightSection>,
    measure: bool,
    profile: &'a DeliveryProfile,
}

impl<'a> SectionSink<'a> {
    fn new(
        output: &'a mut Vec<u8>,
        sections: &'a mut Vec<PreflightSection>,
        measure: bool,
        profile: &'a DeliveryProfile,
    ) -> Self {
        Self {
            output,
            sections,
            measure,
            profile,
        }
    }

    fn append(&mut self, kind: &str, name: &str, path: &str, bytes: &[u8]) -> Result<(), String> {
        let start = self.output.len();
        append_section_with_profile(self.output, self.profile, kind, name, path, bytes)?;
        if self.measure {
            self.sections.push(PreflightSection {
                kind: kind.to_owned(),
                name: name.to_owned(),
                path: path.to_owned(),
                hash: digest(bytes),
                content_bytes: bytes.len(),
                content_tokens_independent: count_tokens(bytes)?,
                rendered_bytes: self.output.len() - start,
                rendered_tokens_independent: count_tokens(&self.output[start..])?,
            });
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct SectionMetadata<'a> {
    kind: &'a str,
    name: &'a str,
    path: &'a str,
    hash: String,
    bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    encoding: Option<&'a str>,
}

#[cfg(test)]
pub(in crate::review_packet) fn append_section(
    output: &mut Vec<u8>,
    kind: &str,
    name: &str,
    path: &str,
    bytes: &[u8],
) -> Result<(), String> {
    append_section_bytes(
        output,
        SECTION_PREFIX,
        METADATA_SUFFIX,
        SECTION_SUFFIX,
        kind,
        name,
        path,
        bytes,
        bytes,
        None,
    )
}

fn append_section_with_profile(
    output: &mut Vec<u8>,
    profile: &DeliveryProfile,
    kind: &str,
    name: &str,
    path: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let encoded;
    let (rendered, encoding) = if matches!(
        profile.id.as_str(),
        DELIVERY_PROFILE_V1_ALPHA2 | DELIVERY_PROFILE_V1_ALPHA3
    ) {
        encoded = encode_section(bytes);
        (encoded.as_slice(), Some(SENTINEL_ESCAPE_PROFILE))
    } else {
        (bytes, None)
    };
    append_section_bytes(
        output,
        &profile.section_prefix,
        &profile.metadata_suffix,
        &profile.section_suffix,
        kind,
        name,
        path,
        bytes,
        rendered,
        encoding,
    )
}

#[allow(clippy::too_many_arguments)]
fn append_section_bytes(
    output: &mut Vec<u8>,
    section_prefix: &str,
    metadata_suffix: &str,
    section_suffix: &str,
    kind: &str,
    name: &str,
    path: &str,
    bytes: &[u8],
    rendered: &[u8],
    encoding: Option<&str>,
) -> Result<(), String> {
    std::str::from_utf8(bytes)
        .map_err(|_| format!("review section `{name}` is not UTF-8 model-visible text"))?;
    let metadata = serde_json::to_vec(&SectionMetadata {
        kind,
        name,
        path,
        hash: digest(bytes),
        bytes: bytes.len(),
        encoding,
    })
    .expect("section metadata serializes");
    output.extend_from_slice(section_prefix.as_bytes());
    output.extend_from_slice(&metadata);
    output.extend_from_slice(metadata_suffix.as_bytes());
    output.extend_from_slice(rendered);
    output.extend_from_slice(section_suffix.as_bytes());
    Ok(())
}

pub(in crate::review_packet) fn encode_section(bytes: &[u8]) -> Vec<u8> {
    const SENTINEL_PREFIX: &[u8] = b"<<<YO-REVIEW-";
    let mut encoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            encoded.extend_from_slice(b"\\\\");
            index += 1;
        } else if bytes[index..].starts_with(SENTINEL_PREFIX) {
            encoded.extend_from_slice(b"\\x3c");
            index += 1;
        } else {
            encoded.push(bytes[index]);
            index += 1;
        }
    }
    encoded
}

#[cfg(test)]
pub(in crate::review_packet) fn decode_section(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"\\\\") {
            decoded.push(b'\\');
            index += 2;
        } else if bytes[index..].starts_with(b"\\x3c") {
            decoded.push(b'<');
            index += 4;
        } else if bytes[index] == b'\\' {
            return Err("invalid sentinel-safe section escape".to_owned());
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    Ok(decoded)
}

pub(in crate::review_packet) fn count_tokens(bytes: &[u8]) -> Result<usize, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "canonical review packet is not UTF-8".to_owned())?;
    Ok(tiktoken_rs::o200k_base_singleton()
        .encode_ordinary(text)
        .len())
}

pub(in crate::review_packet) fn require_budget(
    actual: usize,
    maximum: usize,
) -> Result<(), String> {
    if actual <= maximum {
        Ok(())
    } else {
        Err(format!(
            "managed payload requires {actual} tokens but the budget is {maximum}; no content was truncated"
        ))
    }
}
