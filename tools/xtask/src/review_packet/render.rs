mod sections;

use sections::{
    ContextPaths, append_authorities, append_candidate_suffix, append_context, append_plan,
};
#[cfg(test)]
pub(super) use sections::{append_section, decode_section, encode_section};
pub(super) use sections::{count_tokens, require_budget};

use super::{
    capture::Inputs,
    model::{
        DELIVERY_PROFILE_V1, DELIVERY_PROFILE_V1_ALPHA1, DELIVERY_PROFILE_V1_ALPHA2,
        INPUT_PREFIX_PROFILE, InputPrefixRecord, PreflightSection, ReviewPlan, TOKENIZER_PROFILE,
    },
};
use crate::review_protocol::digest;

pub(super) struct RenderedPacket {
    pub(super) bytes: Vec<u8>,
    pub(super) sections: Vec<PreflightSection>,
    pub(super) input_prefix: Option<InputPrefixRecord>,
}

#[cfg(test)]
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

pub(super) fn render_packet_with_metadata(
    review_id: &str,
    plan: &ReviewPlan,
    inputs: &Inputs,
) -> Result<RenderedPacket, String> {
    render_packet_inner(review_id, plan, inputs, false)
}

fn render_packet_inner(
    review_id: &str,
    plan: &ReviewPlan,
    inputs: &Inputs,
    measure: bool,
) -> Result<RenderedPacket, String> {
    match plan.delivery_profile.id.as_str() {
        DELIVERY_PROFILE_V1 => render_v1(review_id, plan, inputs, measure),
        DELIVERY_PROFILE_V1_ALPHA1 | DELIVERY_PROFILE_V1_ALPHA2 => {
            render_prefixed(review_id, plan, inputs, measure)
        },
        profile => Err(format!(
            "unsupported original review delivery profile `{profile}`"
        )),
    }
}

fn render_v1(
    review_id: &str,
    plan: &ReviewPlan,
    inputs: &Inputs,
    measure: bool,
) -> Result<RenderedPacket, String> {
    let profile = &plan.delivery_profile;
    let mut packet = profile.preamble.as_bytes().to_vec();
    let mut sections = Vec::new();
    append_plan(
        &mut packet,
        &mut sections,
        measure,
        profile,
        review_id,
        plan,
    )?;
    append_context(
        &mut packet,
        &mut sections,
        measure,
        profile,
        inputs,
        ContextPaths::Physical,
    )?;
    append_authorities(&mut packet, &mut sections, measure, profile, inputs)?;
    append_candidate_suffix(&mut packet, &mut sections, measure, profile, inputs)?;
    packet.extend_from_slice(profile.payload_suffix.as_bytes());
    Ok(RenderedPacket {
        bytes: packet,
        sections,
        input_prefix: None,
    })
}

fn render_prefixed(
    review_id: &str,
    plan: &ReviewPlan,
    inputs: &Inputs,
    measure: bool,
) -> Result<RenderedPacket, String> {
    let profile = &plan.delivery_profile;
    let mut packet = profile.preamble.as_bytes().to_vec();
    let mut sections = Vec::new();
    append_context(
        &mut packet,
        &mut sections,
        measure,
        profile,
        inputs,
        ContextPaths::Logical,
    )?;
    append_authorities(&mut packet, &mut sections, measure, profile, inputs)?;
    let input_prefix = input_prefix(&packet)?;
    append_plan(
        &mut packet,
        &mut sections,
        measure,
        profile,
        review_id,
        plan,
    )?;
    append_candidate_suffix(&mut packet, &mut sections, measure, profile, inputs)?;
    packet.extend_from_slice(profile.payload_suffix.as_bytes());
    Ok(RenderedPacket {
        bytes: packet,
        sections,
        input_prefix: Some(input_prefix),
    })
}

fn input_prefix(bytes: &[u8]) -> Result<InputPrefixRecord, String> {
    Ok(InputPrefixRecord {
        boundary_profile: INPUT_PREFIX_PROFILE.to_owned(),
        bytes: bytes.len(),
        hash: digest(bytes),
        tokenizer_profile: TOKENIZER_PROFILE.to_owned(),
        standalone_tokens: count_tokens(bytes)?,
    })
}
