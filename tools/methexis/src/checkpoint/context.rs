//! Context-resolution view of trusted Checkpoint authority.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use super::{
    AuthorityFailure, DEFAULT_TRUSTED_REF, OperationFailure, evaluate, git, hash_bytes, records,
};
use crate::{
    check::{Diagnostic, DiagnosticPhase, Foundation, load_foundation},
    review::ApprovalEvidence,
    source::{self, FreshnessGuard, UnitFreshness},
};

pub(crate) struct ContextAuthority {
    pub(crate) trusted_commit: String,
    pub(crate) checkpoint_id: String,
    pub(crate) checkpoint_hash: String,
    pub(crate) authority_basis_commit: String,
    pub(crate) foundation: Foundation,
    pub(crate) unit_paths: BTreeMap<String, String>,
    pub(crate) active: BTreeSet<String>,
    pub(crate) freshness: BTreeMap<String, UnitFreshness>,
    pub(crate) approval_evidence: BTreeMap<String, ApprovalEvidence>,
    freshness_guard: FreshnessGuard,
    active_record_hash: String,
}

pub(crate) fn resolve(repository_root: &Path) -> Result<ContextAuthority, AuthorityFailure> {
    const OPERATION: &str = "resolve_context";
    let evaluation = evaluate(repository_root, None)?.ok_or_else(|| {
        authority_failure(
            None,
            false,
            "trusted_authority_unavailable",
            "local develop does not provide trusted Methexis authority",
        )
    })?;
    let active = evaluation.active_checkpoint.ok_or_else(|| {
        authority_failure(
            Some(evaluation.trusted_commit.clone()),
            false,
            "active_checkpoint_missing",
            "trusted authority has no active Checkpoint",
        )
    })?;
    let snapshot = git::resolve_exact(repository_root, &evaluation.trusted_commit, OPERATION)
        .map_err(operation_failure)?;
    let foundation = load_foundation(&snapshot.root).map_err(|diagnostics| AuthorityFailure {
        diagnostics,
        trusted_commit: Some(evaluation.trusted_commit.clone()),
        retryable: false,
    })?;
    let unit_paths = foundation
        .units
        .iter()
        .map(|unit| {
            (
                unit.metadata.id.clone(),
                unit.path
                    .strip_prefix(&snapshot.root)
                    .unwrap_or(&unit.path)
                    .to_string_lossy()
                    .replace('\\', "/"),
            )
        })
        .collect();
    Ok(ContextAuthority {
        trusted_commit: evaluation.trusted_commit,
        checkpoint_id: active.id,
        checkpoint_hash: active.hash,
        authority_basis_commit: active.authority_basis_commit,
        foundation,
        unit_paths,
        active: evaluation.active,
        freshness: evaluation.freshness,
        approval_evidence: evaluation.approval_evidence,
        freshness_guard: evaluation.freshness_guard,
        active_record_hash: active.active_record_hash,
    })
}

pub(crate) fn final_revalidate(
    repository_root: &Path,
    authority: &ContextAuthority,
) -> Result<(), AuthorityFailure> {
    const OPERATION: &str = "resolve_context";
    source::final_revalidate(repository_root, &authority.freshness_guard).map_err(|failure| {
        AuthorityFailure {
            diagnostics: vec![Diagnostic {
                phase: DiagnosticPhase::Global,
                path: "methexis/sources".to_owned(),
                code: "source_changed_during_resolution".to_owned(),
                message: failure.message,
                line: None,
                column: None,
                affected_ids: failure.affected_ids,
            }],
            trusted_commit: Some(authority.trusted_commit.clone()),
            retryable: true,
        }
    })?;
    let snapshot = git::resolve(repository_root, DEFAULT_TRUSTED_REF, OPERATION)
        .map_err(|error| concurrent_authority_failure(authority, error.error.message))?;
    let active_path = snapshot.root.join("methexis/active-checkpoint.yaml");
    let (active, bytes) = records::read_active(&active_path, OPERATION)
        .map_err(|error| concurrent_authority_failure(authority, error.error.message))?;
    if snapshot.commit != authority.trusted_commit
        || active.checkpoint_id != authority.checkpoint_id
        || active.checkpoint_hash != authority.checkpoint_hash
        || hash_bytes(&bytes) != authority.active_record_hash
    {
        return Err(authority_failure(
            Some(authority.trusted_commit.clone()),
            true,
            "authority_changed_during_resolution",
            "trusted ref or active Checkpoint changed during context resolution",
        ));
    }
    Ok(())
}

fn concurrent_authority_failure(
    authority: &ContextAuthority,
    detail: impl Into<String>,
) -> AuthorityFailure {
    authority_failure(
        Some(authority.trusted_commit.clone()),
        true,
        "authority_changed_during_resolution",
        &format!(
            "trusted ref or active Checkpoint changed during context resolution: {}",
            detail.into()
        ),
    )
}

fn operation_failure(error: OperationFailure) -> AuthorityFailure {
    AuthorityFailure {
        diagnostics: vec![Diagnostic {
            phase: DiagnosticPhase::Global,
            path: "refs/heads/develop".to_owned(),
            code: error.error.code,
            message: error.error.message,
            line: None,
            column: None,
            affected_ids: error.error.affected_ids,
        }],
        trusted_commit: error.trusted_commit,
        retryable: false,
    }
}

fn authority_failure(
    trusted_commit: Option<String>,
    retryable: bool,
    code: &str,
    message: &str,
) -> AuthorityFailure {
    AuthorityFailure {
        diagnostics: vec![Diagnostic {
            phase: DiagnosticPhase::Global,
            path: "refs/heads/develop".to_owned(),
            code: code.to_owned(),
            message: message.to_owned(),
            line: None,
            column: None,
            affected_ids: Vec::new(),
        }],
        trusted_commit,
        retryable,
    }
}
