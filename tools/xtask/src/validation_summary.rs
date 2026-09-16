use std::path::Path;

use crate::review_packet::external_operation;

mod command;
mod identity;
mod model;
mod parse;
mod reuse;
mod schemas;

pub(crate) use model::VerifiedSummary;
pub(crate) use reuse::current_reusable_context;

pub(crate) fn argv_hash(argv: &[String]) -> String {
    command::argv_hash(argv)
}

pub(crate) fn current_toolchain_hash() -> Result<String, String> {
    reuse::current_toolchain_hash()
}

pub(crate) fn verify(
    repository: &Path,
    bytes: &[u8],
    expected_name: &str,
    expected_argv: &[String],
    candidate: &str,
    requested_reuse: bool,
) -> Result<VerifiedSummary, String> {
    let envelope = parse::schema(bytes)?;
    match envelope.schema.as_str() {
        schemas::LEGACY_SCHEMA => schemas::verify_legacy(bytes, expected_name),
        schemas::ALPHA1_SCHEMA => schemas::verify_alpha1(
            bytes,
            expected_name,
            expected_argv,
            candidate,
            requested_reuse,
        ),
        schemas::ALPHA2_SCHEMA => schemas::verify_alpha2(
            repository,
            bytes,
            expected_name,
            expected_argv,
            candidate,
            requested_reuse,
        ),
        schemas::ALPHA3_SCHEMA => schemas::verify_alpha3(
            repository,
            bytes,
            expected_name,
            expected_argv,
            candidate,
            requested_reuse,
        ),
        schemas::ALPHA4_SCHEMA => schemas::verify_alpha4(
            repository,
            bytes,
            expected_name,
            expected_argv,
            candidate,
            requested_reuse,
        ),
        external_operation::SCHEMA => schemas::verify_external_operation(
            bytes,
            expected_name,
            expected_argv,
            candidate,
            requested_reuse,
        ),
        other => parse::unsupported_schema(other),
    }
}

/// Verify everything an immutable review-packet request knows about validation evidence.
/// Exact argv values remain gate-owned because review-packet requests intentionally carry
/// only the evidence name and path.
pub(crate) fn verify_review_input(
    _repository: &Path,
    bytes: &[u8],
    expected_name: &str,
    candidate: &str,
) -> Result<(), String> {
    let envelope = parse::schema(bytes)?;
    match envelope.schema.as_str() {
        schemas::LEGACY_SCHEMA => schemas::verify_legacy_review_input(bytes, expected_name),
        schemas::ALPHA1_SCHEMA => {
            schemas::verify_alpha1_review_input(bytes, expected_name, candidate)
        },
        schemas::ALPHA2_SCHEMA => {
            schemas::verify_alpha2_review_input(bytes, expected_name, candidate)
        },
        schemas::ALPHA3_SCHEMA => {
            schemas::verify_alpha3_review_input(bytes, expected_name, candidate)
        },
        schemas::ALPHA4_SCHEMA => {
            schemas::verify_alpha4_review_input(bytes, expected_name, candidate)
        },
        external_operation::SCHEMA => {
            schemas::verify_external_review_input(bytes, expected_name, candidate)
        },
        other => parse::unsupported_schema(other),
    }
}

#[cfg(test)]
mod tests;
