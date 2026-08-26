use std::path::Path;

use serde::Deserialize;

use crate::{git, review_packet::external_operation, review_protocol};

const LEGACY_SCHEMA: &str = "yo.validation-run-summary/v1";
const ALPHA1_SCHEMA: &str = "yo.validation-run-summary/v1alpha1";
const ALPHA2_SCHEMA: &str = "yo.validation-run-summary/v1alpha2";
const REVIEWED_DESCENDANT_REUSE: &str = "reviewed-descendant/v1";
const ARGV_DOMAIN: &[u8] = b"yo.validation-run-argv/v1alpha1\0";

pub(crate) struct VerifiedSummary {
    pub(crate) status: String,
    pub(crate) log_path: Option<String>,
}

#[derive(Deserialize)]
struct SchemaEnvelope {
    schema: String,
}

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

pub(crate) fn verify(
    repository: &Path,
    bytes: &[u8],
    expected_name: &str,
    expected_argv: &[String],
    candidate: &str,
    requested_reuse: bool,
) -> Result<VerifiedSummary, String> {
    let envelope = schema(bytes)?;
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
        other => unsupported_schema(other),
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
    let envelope = schema(bytes)?;
    match envelope.schema.as_str() {
        LEGACY_SCHEMA => verify_legacy(bytes, expected_name).map(drop),
        ALPHA1_SCHEMA => {
            let summary: AlphaSummary = parse(bytes)?;
            verify_alpha_identity(&summary, expected_name, candidate)?;
            verify_recorded_command_and_log(
                summary.command_argv_count,
                &summary.command_argv_hash,
                &summary.log_hash,
            )
        },
        ALPHA2_SCHEMA => {
            let summary: Alpha2Summary = parse(bytes)?;
            verify_alpha2_identity(&summary, expected_name, candidate)?;
            verify_recorded_command_and_log(
                summary.command_argv_count,
                &summary.command_argv_hash,
                &summary.log_hash,
            )
        },
        external_operation::SCHEMA => external_operation::validate(expected_name, bytes, candidate),
        other => unsupported_schema(other),
    }
}

fn schema(bytes: &[u8]) -> Result<SchemaEnvelope, String> {
    serde_json::from_slice(bytes).map_err(|error| format!("cannot read summary schema: {error}"))
}

fn parse<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, String> {
    serde_json::from_slice(bytes).map_err(|error| error.to_string())
}

fn unsupported_schema<T>(schema: &str) -> Result<T, String> {
    Err(format!(
        "unsupported schema `{schema}`; expected `{LEGACY_SCHEMA}`, `{ALPHA1_SCHEMA}`, `{ALPHA2_SCHEMA}`, or `{}`",
        external_operation::SCHEMA
    ))
}

fn verify_legacy(bytes: &[u8], expected_name: &str) -> Result<VerifiedSummary, String> {
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

fn verify_alpha1(
    bytes: &[u8],
    expected_name: &str,
    expected_argv: &[String],
    candidate: &str,
    requested_reuse: bool,
) -> Result<VerifiedSummary, String> {
    let summary: AlphaSummary = parse(bytes)?;
    verify_alpha_identity(&summary, expected_name, candidate)?;
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

fn verify_alpha_identity(
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

fn verify_alpha2_identity(
    summary: &Alpha2Summary,
    expected_name: &str,
    candidate: &str,
) -> Result<(), String> {
    verify_common(
        &summary.schema,
        ALPHA2_SCHEMA,
        &summary.name,
        expected_name,
        &summary.status,
        summary.exit_code,
    )?;
    if summary.reuse_policy != REVIEWED_DESCENDANT_REUSE {
        return Err(format!(
            "reuse_policy must be `{REVIEWED_DESCENDANT_REUSE}`"
        ));
    }
    verify_exact_execution(
        &summary.head_commit,
        &summary.worktree_state,
        summary.reused,
        candidate,
    )
}

fn verify_common(
    schema: &str,
    expected_schema: &str,
    name: &str,
    expected_name: &str,
    status: &str,
    exit_code: i32,
) -> Result<(), String> {
    if schema != expected_schema {
        return Err(format!("schema must be `{expected_schema}`"));
    }
    if name != expected_name {
        return Err(format!(
            "summary name `{name}` does not match requested evidence name `{expected_name}`"
        ));
    }
    if !matches!(status, "passed" | "failed") || (status == "passed") != (exit_code == 0) {
        return Err("status and exit_code are inconsistent".to_owned());
    }
    Ok(())
}

fn verify_exact_execution(
    head_commit: &str,
    worktree_state: &str,
    reused: bool,
    candidate: &str,
) -> Result<(), String> {
    review_protocol::require_commit(head_commit, "validation summary head_commit")?;
    if head_commit != candidate {
        return Err(format!(
            "head_commit {head_commit} does not match candidate {candidate}"
        ));
    }
    if worktree_state != "clean" {
        return Err("worktree_state must be `clean`".to_owned());
    }
    if reused {
        return Err("an execution summary must record `reused:false`".to_owned());
    }
    Ok(())
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
    let summary: Alpha2Summary = parse(bytes)?;
    verify_common(
        &summary.schema,
        ALPHA2_SCHEMA,
        &summary.name,
        expected_name,
        &summary.status,
        summary.exit_code,
    )?;
    if summary.reuse_policy != REVIEWED_DESCENDANT_REUSE {
        return Err(format!(
            "reuse_policy must be `{REVIEWED_DESCENDANT_REUSE}`"
        ));
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

fn verify_recorded_command_and_log(
    command_argv_count: usize,
    command_argv_hash: &str,
    log_hash: &str,
) -> Result<(), String> {
    if command_argv_count == 0 {
        return Err("command_argv_count must be greater than zero".to_owned());
    }
    canonical_sha256(command_argv_hash, "validation command argv hash")?;
    canonical_sha256(log_hash, "validation log hash")
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
    canonical_sha256(log_hash, "validation log hash")
}

fn canonical_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.strip_prefix("sha256:").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    }) {
        Ok(())
    } else {
        Err(format!("{label} must be canonical SHA-256"))
    }
}

pub(crate) fn argv_hash(argv: &[String]) -> String {
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
    use serde_json::json;

    use super::{argv_hash, verify_review_input};

    fn alpha2(name: &str, candidate: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "schema": "yo.validation-run-summary/v1alpha2",
            "name": name,
            "status": "passed",
            "exit_code": 0,
            "elapsed_seconds": 1,
            "log_bytes": 1,
            "log_path": ".local-exclude/test.log",
            "log_hash": format!("sha256:{}", "1".repeat(64)),
            "head_commit": candidate,
            "worktree_state": "clean",
            "command_argv_count": 2,
            "command_argv_hash": argv_hash(&["cargo".to_owned(), "test".to_owned()]),
            "reused": false,
            "reuse_policy": "reviewed-descendant/v1"
        }))
        .unwrap()
    }

    // 패킷 준비 단계가 runner 내부 이름과 exact candidate를 묶어 늦은 gate 실패를 막는다.
    #[test]
    fn review_input_binds_name_and_exact_candidate_before_publication() {
        let candidate = "a".repeat(40);
        let bytes = alpha2("yo-cli", &candidate);
        let repository = std::path::Path::new(".");

        verify_review_input(repository, &bytes, "yo-cli", &candidate).unwrap();
        assert!(
            verify_review_input(repository, &bytes, "yo-cli-tests", &candidate)
                .unwrap_err()
                .contains("does not match requested evidence name")
        );
        assert!(
            verify_review_input(repository, &bytes, "yo-cli", &"b".repeat(40))
                .unwrap_err()
                .contains("does not match candidate")
        );
    }

    // 실행 정체성을 증명하지 못하는 dirty, reused, 빈 argv 기록은 리뷰 입력이 될 수 없다.
    #[test]
    fn review_input_rejects_dirty_reused_or_unidentified_execution() {
        let candidate = "a".repeat(40);
        let repository = std::path::Path::new(".");
        for (field, replacement, message) in [
            ("worktree_state", json!("dirty"), "worktree_state"),
            ("reused", json!(true), "reused:false"),
            ("command_argv_count", json!(0), "greater than zero"),
        ] {
            let mut summary: serde_json::Value =
                serde_json::from_slice(&alpha2("yo-cli", &candidate)).unwrap();
            summary[field] = replacement;
            let bytes = serde_json::to_vec(&summary).unwrap();
            assert!(
                verify_review_input(repository, &bytes, "yo-cli", &candidate)
                    .unwrap_err()
                    .contains(message)
            );
        }
    }

    // 각 argv의 byte 길이와 NUL 경계를 framing하여 단순 연결의 인자 경계 충돌을 막는다.
    #[test]
    fn argv_hash_is_boundary_aware() {
        assert_ne!(
            argv_hash(&["ab".to_owned(), "c".to_owned()]),
            argv_hash(&["a".to_owned(), "bc".to_owned()])
        );
    }

    // Rust 검증기의 framing을 bounded shell runner의 canonical test vector와 맞춘다.
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
