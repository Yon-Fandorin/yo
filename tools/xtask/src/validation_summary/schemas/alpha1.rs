use serde::Deserialize;

use super::{
    super::{
        command::{verify_command_and_log, verify_recorded_command_and_log},
        identity::{verify_common, verify_exact_execution},
        model::VerifiedSummary,
        parse::parse,
    },
    ALPHA1_SCHEMA,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AlphaSummary {
    schema: String,
    name: String,
    status: String,
    exit_code: i32,
    elapsed_seconds: u64,
    log_bytes: u64,
    log_path: String,
    log_hash: String,
    head_commit: String,
    worktree_state: String,
    command_argv_count: usize,
    command_argv_hash: String,
    reused: bool,
}

pub(in crate::validation_summary) fn verify(
    bytes: &[u8],
    expected_name: &str,
    expected_argv: &[String],
    candidate: &str,
    requested_reuse: bool,
) -> Result<VerifiedSummary, String> {
    let summary: AlphaSummary = parse(bytes)?;
    verify_identity(&summary, expected_name, candidate)?;
    if requested_reuse {
        return Err("v1alpha1 does not permit reused validation evidence".to_owned());
    }
    verify_command_and_log(
        expected_argv,
        summary.command_argv_count,
        &summary.command_argv_hash,
        &summary.log_hash,
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
    candidate: &str,
) -> Result<(), String> {
    let summary: AlphaSummary = parse(bytes)?;
    verify_identity(&summary, expected_name, candidate)?;
    verify_recorded_command_and_log(
        summary.command_argv_count,
        &summary.command_argv_hash,
        &summary.log_hash,
    )
}

fn verify_identity(
    summary: &AlphaSummary,
    expected_name: &str,
    candidate: &str,
) -> Result<(), String> {
    verify_common(
        &summary.schema,
        ALPHA1_SCHEMA,
        &summary.name,
        expected_name,
        &summary.status,
        summary.exit_code,
    )?;
    verify_exact_execution(
        &summary.head_commit,
        &summary.worktree_state,
        summary.reused,
        candidate,
    )
}
