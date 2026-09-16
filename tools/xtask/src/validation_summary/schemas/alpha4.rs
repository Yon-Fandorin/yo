use std::path::Path;

use serde::Deserialize;

use super::{
    super::{
        command::{verify_command_and_log, verify_recorded_command_and_log},
        identity::{verify_common, verify_exact_execution},
        model::{ResourceLease, ReuseContext, VerifiedSummary},
        parse::parse,
        reuse::{
            CONTEXT_REUSE, REVIEWED_DESCENDANT_REUSE, verify_current_reuse_context,
            verify_resource_lease, verify_reuse_context_format,
        },
    },
    ALPHA4_SCHEMA,
};
use crate::{git, review_protocol};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Alpha4Summary {
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
    reuse_policy: String,
    #[serde(default)]
    reuse_context: Option<ReuseContext>,
    resource_lease: ResourceLease,
}

pub(in crate::validation_summary) fn verify(
    repository: &Path,
    bytes: &[u8],
    expected_name: &str,
    expected_argv: &[String],
    candidate: &str,
    requested_reuse: bool,
) -> Result<VerifiedSummary, String> {
    let summary: Alpha4Summary = parse(bytes)?;
    verify_common(
        &summary.schema,
        ALPHA4_SCHEMA,
        &summary.name,
        expected_name,
        &summary.status,
        summary.exit_code,
    )?;
    verify_resource_lease(&summary.resource_lease)?;
    review_protocol::require_commit(&summary.head_commit, "validation summary head_commit")?;
    if summary.worktree_state != "clean" {
        return Err("worktree_state must be `clean`".to_owned());
    }
    if summary.reused {
        return Err("an execution summary must record `reused:false`".to_owned());
    }
    match &summary.reuse_context {
        Some(context) => {
            if summary.reuse_policy != CONTEXT_REUSE {
                return Err(format!("reuse_policy must be `{CONTEXT_REUSE}`"));
            }
            verify_reuse_context_format(context)?;
            if requested_reuse {
                verify_current_reuse_context(context)?;
            }
        },
        None if summary.reuse_policy == REVIEWED_DESCENDANT_REUSE => {},
        None => {
            return Err(format!(
                "reuse_policy must be `{REVIEWED_DESCENDANT_REUSE}` without reuse_context"
            ));
        },
    }
    if requested_reuse {
        if summary.status != "passed" {
            return Err("only passed validation evidence can be reused".to_owned());
        }
        if !git::trusted_succeeds_in(
            repository,
            &[
                "merge-base",
                "--is-ancestor",
                &summary.head_commit,
                candidate,
            ],
        )? {
            return Err(format!(
                "validation summary head_commit {} is not an ancestor of candidate {candidate}",
                summary.head_commit
            ));
        }
    } else if summary.head_commit != candidate {
        return Err(format!(
            "head_commit {} does not match candidate {candidate}",
            summary.head_commit
        ));
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
    let summary: Alpha4Summary = parse(bytes)?;
    verify_identity(&summary, expected_name, candidate)?;
    verify_resource_lease(&summary.resource_lease)?;
    verify_recorded_command_and_log(
        summary.command_argv_count,
        &summary.command_argv_hash,
        &summary.log_hash,
    )
}

pub(in crate::validation_summary) fn reuse_context(
    bytes: &[u8],
) -> Result<Option<ReuseContext>, String> {
    let summary: Alpha4Summary = parse(bytes)?;
    verify_resource_lease(&summary.resource_lease)?;
    Ok(summary.reuse_context)
}

fn verify_identity(
    summary: &Alpha4Summary,
    expected_name: &str,
    candidate: &str,
) -> Result<(), String> {
    verify_common(
        &summary.schema,
        ALPHA4_SCHEMA,
        &summary.name,
        expected_name,
        &summary.status,
        summary.exit_code,
    )?;
    match &summary.reuse_context {
        Some(context) => {
            if summary.reuse_policy != CONTEXT_REUSE {
                return Err(format!("reuse_policy must be `{CONTEXT_REUSE}`"));
            }
            verify_reuse_context_format(context)?;
        },
        None if summary.reuse_policy == REVIEWED_DESCENDANT_REUSE => {},
        None => {
            return Err(format!(
                "reuse_policy must be `{REVIEWED_DESCENDANT_REUSE}` without reuse_context"
            ));
        },
    }
    verify_exact_execution(
        &summary.head_commit,
        &summary.worktree_state,
        summary.reused,
        candidate,
    )
}
