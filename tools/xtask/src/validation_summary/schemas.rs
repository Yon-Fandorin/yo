use super::model::VerifiedSummary;
use crate::review_packet::external_operation;

mod alpha1;
mod alpha2;
mod alpha3;
mod alpha4;
mod legacy;

pub(super) const LEGACY_SCHEMA: &str = "yo.validation-run-summary/v1";
pub(super) const ALPHA1_SCHEMA: &str = "yo.validation-run-summary/v1alpha1";
pub(super) const ALPHA2_SCHEMA: &str = "yo.validation-run-summary/v1alpha2";
pub(super) const ALPHA3_SCHEMA: &str = "yo.validation-run-summary/v1alpha3";
pub(super) const ALPHA4_SCHEMA: &str = "yo.validation-run-summary/v1alpha4";

pub(super) use alpha1::{
    verify as verify_alpha1, verify_review_input as verify_alpha1_review_input,
};
pub(super) use alpha2::{
    verify as verify_alpha2, verify_review_input as verify_alpha2_review_input,
};
pub(super) use alpha3::{
    reuse_context as alpha3_reuse_context, verify as verify_alpha3,
    verify_review_input as verify_alpha3_review_input,
};
pub(super) use alpha4::{
    reuse_context as alpha4_reuse_context, verify as verify_alpha4,
    verify_review_input as verify_alpha4_review_input,
};
pub(super) use legacy::{
    verify as verify_legacy, verify_review_input as verify_legacy_review_input,
};

pub(super) fn verify_external_operation(
    bytes: &[u8],
    expected_name: &str,
    expected_argv: &[String],
    candidate: &str,
    requested_reuse: bool,
) -> Result<VerifiedSummary, String> {
    external_operation::validate_for_gate(
        expected_name,
        bytes,
        candidate,
        expected_argv,
        requested_reuse,
    )?;
    Ok(VerifiedSummary {
        status: "passed".to_owned(),
        log_path: None,
    })
}

pub(super) fn verify_external_review_input(
    bytes: &[u8],
    expected_name: &str,
    candidate: &str,
) -> Result<(), String> {
    external_operation::validate(expected_name, bytes, candidate)
}
