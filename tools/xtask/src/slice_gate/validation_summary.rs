use std::path::Path;

use serde::Deserialize;

use super::{canonical_sha256, model::LegacyValidationSummary};
use crate::{git, review_packet::external_operation, review_protocol};

const LEGACY_SCHEMA: &str = "yo.validation-run-summary/v1";
const ALPHA1_SCHEMA: &str = "yo.validation-run-summary/v1alpha1";
const ALPHA2_SCHEMA: &str = "yo.validation-run-summary/v1alpha2";
const REVIEWED_DESCENDANT_REUSE: &str = "reviewed-descendant/v1";
const ARGV_DOMAIN: &[u8] = b"yo.validation-run-argv/v1alpha1\0";

pub(super) struct VerifiedSummary {
    pub(super) status: String,
    pub(super) log_path: Option<String>,
}

#[derive(Deserialize)]
struct SchemaEnvelope {
    schema: String,
}

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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Alpha2Summary {
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
}

pub(super) fn verify(
    repository: &Path,
    bytes: &[u8],
    expected_name: &str,
    expected_argv: &[String],
    candidate: &str,
    requested_reuse: bool,
) -> Result<VerifiedSummary, String> {
    let envelope: SchemaEnvelope = serde_json::from_slice(bytes)
        .map_err(|error| format!("cannot read summary schema: {error}"))?;
    match envelope.schema.as_str() {
        LEGACY_SCHEMA => verify_legacy(bytes, expected_name),
        ALPHA1_SCHEMA => verify_alpha1(
            bytes,
            expected_name,
            expected_argv,
            candidate,
            requested_reuse,
        ),
        ALPHA2_SCHEMA => verify_alpha2(
            repository,
            bytes,
            expected_name,
            expected_argv,
            candidate,
            requested_reuse,
        ),
        external_operation::SCHEMA => verify_external_operation(
            bytes,
            expected_name,
            expected_argv,
            candidate,
            requested_reuse,
        ),
        other => Err(format!(
            "unsupported schema `{other}`; expected `{LEGACY_SCHEMA}`, `{ALPHA1_SCHEMA}`, `{ALPHA2_SCHEMA}`, or `{}`",
            external_operation::SCHEMA
        )),
    }
}

fn verify_legacy(bytes: &[u8], expected_name: &str) -> Result<VerifiedSummary, String> {
    let summary: LegacyValidationSummary =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if summary.schema != LEGACY_SCHEMA
        || summary.name != expected_name
        || !matches!(summary.status.as_str(), "passed" | "failed")
        || (summary.status == "passed") != (summary.exit_code == 0)
    {
        return Err("inconsistent summary fields".to_owned());
    }
    let _ = (summary.elapsed_seconds, summary.log_bytes);
    Ok(VerifiedSummary {
        status: summary.status,
        log_path: Some(summary.log_path),
    })
}

fn verify_alpha1(
    bytes: &[u8],
    expected_name: &str,
    expected_argv: &[String],
    candidate: &str,
    requested_reuse: bool,
) -> Result<VerifiedSummary, String> {
    let summary: AlphaSummary = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if summary.schema != ALPHA1_SCHEMA
        || summary.name != expected_name
        || !matches!(summary.status.as_str(), "passed" | "failed")
        || (summary.status == "passed") != (summary.exit_code == 0)
    {
        return Err("inconsistent summary fields".to_owned());
    }
    review_protocol::require_commit(&summary.head_commit, "validation summary head_commit")?;
    if summary.head_commit != candidate {
        return Err(format!(
            "head_commit {} does not match candidate {candidate}",
            summary.head_commit
        ));
    }
    if summary.worktree_state != "clean" {
        return Err("worktree_state must be `clean`".to_owned());
    }
    if summary.reused || requested_reuse {
        return Err("v1alpha1 does not permit reused validation evidence".to_owned());
    }
    if summary.command_argv_count != expected_argv.len() {
        return Err("command_argv_count does not match the gate request".to_owned());
    }
    canonical_sha256(&summary.command_argv_hash, "validation command argv hash")?;
    let expected_hash = argv_hash(expected_argv);
    if summary.command_argv_hash != expected_hash {
        return Err(format!(
            "command_argv_hash does not match the gate request; expected {expected_hash}"
        ));
    }
    canonical_sha256(&summary.log_hash, "validation log hash")?;
    let _ = (summary.elapsed_seconds, summary.log_bytes);
    Ok(VerifiedSummary {
        status: summary.status,
        log_path: Some(summary.log_path),
    })
}

fn verify_external_operation(
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

fn verify_alpha2(
    repository: &Path,
    bytes: &[u8],
    expected_name: &str,
    expected_argv: &[String],
    candidate: &str,
    requested_reuse: bool,
) -> Result<VerifiedSummary, String> {
    let summary: Alpha2Summary =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if summary.schema != ALPHA2_SCHEMA
        || summary.name != expected_name
        || !matches!(summary.status.as_str(), "passed" | "failed")
        || (summary.status == "passed") != (summary.exit_code == 0)
        || summary.reuse_policy != REVIEWED_DESCENDANT_REUSE
    {
        return Err("inconsistent summary fields".to_owned());
    }
    review_protocol::require_commit(&summary.head_commit, "validation summary head_commit")?;
    if summary.worktree_state != "clean" {
        return Err("worktree_state must be `clean`".to_owned());
    }
    if summary.reused {
        return Err("an execution summary must record `reused:false`".to_owned());
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

fn verify_command_and_log(
    expected_argv: &[String],
    command_argv_count: usize,
    command_argv_hash: &str,
    log_hash: &str,
) -> Result<(), String> {
    if command_argv_count != expected_argv.len() {
        return Err("command_argv_count does not match the gate request".to_owned());
    }
    canonical_sha256(command_argv_hash, "validation command argv hash")?;
    let expected_hash = argv_hash(expected_argv);
    if command_argv_hash != expected_hash {
        return Err(format!(
            "command_argv_hash does not match the gate request; expected {expected_hash}"
        ));
    }
    canonical_sha256(log_hash, "validation log hash")?;
    Ok(())
}

pub(super) fn argv_hash(argv: &[String]) -> String {
    let mut framed = Vec::with_capacity(
        ARGV_DOMAIN.len() + argv.iter().map(|value| value.len() + 24).sum::<usize>(),
    );
    framed.extend_from_slice(ARGV_DOMAIN);
    for value in argv {
        framed.extend_from_slice(value.len().to_string().as_bytes());
        framed.push(b':');
        framed.extend_from_slice(value.as_bytes());
        framed.push(0);
    }
    review_protocol::digest(&framed)
}

#[cfg(test)]
mod tests {
    use super::argv_hash;

    // 각 argv의 byte 길이와 NUL 경계를 framing하여 단순 byte 연결에서 생기는
    // 인자 경계 충돌을 서로 다른 command identity로 유지한다.
    #[test]
    fn argv_hash_is_boundary_aware() {
        assert_ne!(
            argv_hash(&["ab".to_owned(), "c".to_owned()]),
            argv_hash(&["a".to_owned(), "bc".to_owned()])
        );
    }

    // Rust gate의 framing이 shell runner의 canonical test vector와 같아 한쪽만
    // 바뀐 summary를 valid evidence로 오인하지 않게 한다.
    #[test]
    fn argv_hash_matches_the_bounded_runner_framing() {
        let argv = [
            "bash".to_owned(),
            "-c".to_owned(),
            "printf \"visible only in the full log\\n\"; printf \"diagnostic\\n\" >&2".to_owned(),
        ];

        assert_eq!(
            argv_hash(&argv),
            "sha256:b2feeb2dc7a19ae550541f96076627745b156652ed171a1f7bc182cbdee19b74"
        );
    }
}
