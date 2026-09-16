use serde::Deserialize;

use super::{
    super::{identity::verify_common, model::VerifiedSummary, parse::parse},
    LEGACY_SCHEMA,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacySummary {
    schema: String,
    name: String,
    status: String,
    exit_code: i32,
    elapsed_seconds: u64,
    log_bytes: u64,
    log_path: String,
}

pub(in crate::validation_summary) fn verify(
    bytes: &[u8],
    expected_name: &str,
) -> Result<VerifiedSummary, String> {
    let summary: LegacySummary = parse(bytes)?;
    verify_common(
        &summary.schema,
        LEGACY_SCHEMA,
        &summary.name,
        expected_name,
        &summary.status,
        summary.exit_code,
    )?;
    let _ = (summary.elapsed_seconds, summary.log_bytes);
    Ok(VerifiedSummary {
        status: summary.status,
        log_path: Some(summary.log_path),
    })
}

pub(in crate::validation_summary) fn verify_review_input(
    bytes: &[u8],
    expected_name: &str,
) -> Result<(), String> {
    verify(bytes, expected_name).map(drop)
}
