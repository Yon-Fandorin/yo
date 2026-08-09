//! Trusted approval and active-Checkpoint derivation for Fast Check.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use super::{
    DEFAULT_TRUSTED_REF, git, hash_bytes,
    records::{read_active, read_checkpoint},
    validation,
};
use crate::{
    check::{Diagnostic, DiagnosticPhase, load_foundation},
    model::Source,
    review::{ApprovalEvidence, validate_records},
    source::{self, UnitFreshness},
};

pub(crate) struct AuthorityEvaluation {
    pub(crate) trusted_commit: String,
    pub(crate) approvals: BTreeMap<String, String>,
    pub(crate) approval_evidence: BTreeMap<String, ApprovalEvidence>,
    pub(crate) active: BTreeSet<String>,
    pub(crate) freshness: BTreeMap<String, UnitFreshness>,
    pub(crate) freshness_guard: source::FreshnessGuard,
    pub(crate) checkpoint: &'static str,
    pub(crate) active_checkpoint: Option<ActiveCheckpoint>,
}

pub(crate) struct ActiveCheckpoint {
    pub(crate) id: String,
    pub(crate) hash: String,
    pub(crate) active_record_hash: String,
    pub(crate) authority_basis_commit: String,
}

#[derive(Debug)]
pub(crate) struct AuthorityFailure {
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) trusted_commit: Option<String>,
    pub(crate) retryable: bool,
}

impl From<Vec<Diagnostic>> for AuthorityFailure {
    fn from(diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            diagnostics,
            trusted_commit: None,
            retryable: false,
        }
    }
}

impl AuthorityFailure {
    pub(crate) fn from_source(trusted_commit: &str, failure: source::FreshnessFailure) -> Self {
        let negative_records = failure.code.starts_with("negative_records_");
        let retryable = matches!(
            failure.code,
            "source_changed_during_validation" | "negative_records_changed_during_validation"
        );
        Self {
            diagnostics: vec![Diagnostic {
                phase: DiagnosticPhase::Global,
                path: if negative_records {
                    "methexis/negative-records.yaml"
                } else {
                    "methexis/sources"
                }
                .to_owned(),
                code: failure.code.to_owned(),
                message: failure.message,
                line: None,
                column: None,
                affected_ids: failure.affected_ids,
            }],
            trusted_commit: Some(trusted_commit.to_owned()),
            retryable,
        }
    }
}

pub(crate) fn evaluate(
    repository_root: &Path,
    provided_working_sources: Option<&[Source]>,
) -> Result<Option<AuthorityEvaluation>, AuthorityFailure> {
    const OPERATION: &str = "check_authority";
    if !repository_root.join(".git").exists() {
        return Ok(None);
    }
    let snapshot = match git::resolve(repository_root, DEFAULT_TRUSTED_REF, OPERATION) {
        Ok(snapshot) => snapshot,
        Err(error) if error.code() == "trusted_corpus_missing" => return Ok(None),
        Err(error) => return Err(vec![diagnostic(error)].into()),
    };
    let with_commit = |diagnostics| AuthorityFailure {
        diagnostics,
        trusted_commit: Some(snapshot.commit.clone()),
        retryable: false,
    };
    let foundation = load_foundation(&snapshot.root).map_err(&with_commit)?;
    let review = validate_records(&snapshot.root, &foundation);
    if !review.diagnostics.is_empty() {
        return Err(with_commit(review.diagnostics));
    }
    let approvals = foundation
        .units
        .iter()
        .filter(|unit| {
            review
                .states
                .get(&unit.metadata.id)
                .is_some_and(|state| state.evidence == "matching_proposal")
        })
        .map(|unit| (unit.metadata.id.clone(), unit.revision.clone()))
        .collect::<BTreeMap<_, _>>();
    let approval_evidence = review.evidence;

    let active_path = snapshot.root.join("methexis/active-checkpoint.yaml");
    if !active_path.exists() {
        return Ok(Some(AuthorityEvaluation {
            trusted_commit: snapshot.commit.clone(),
            approvals,
            approval_evidence,
            active: BTreeSet::new(),
            freshness: BTreeMap::new(),
            freshness_guard: source::FreshnessGuard::empty(),
            checkpoint: "inactive",
            active_checkpoint: None,
        }));
    }
    let (active_record, active_bytes) = read_active(&active_path, OPERATION)
        .map_err(|error| with_commit(vec![diagnostic(error)]))?;
    let filename = active_record
        .checkpoint_id
        .strip_prefix("sha256:")
        .ok_or_else(|| {
            with_commit(vec![simple_diagnostic(
                "invalid_active_checkpoint",
                "active CheckpointId has no sha256 prefix",
            )])
        })?;
    let checkpoint_path = snapshot
        .root
        .join("methexis/checkpoints")
        .join(format!("{filename}.yaml"));
    let (checkpoint, bytes) = read_checkpoint(&checkpoint_path, OPERATION)
        .map_err(|error| with_commit(vec![diagnostic(error)]))?;
    if active_record.checkpoint_hash != hash_bytes(&bytes)
        || active_record.checkpoint_id != checkpoint.checkpoint_id
        || active_record.trusted_commit != checkpoint.trusted_commit
    {
        return Err(with_commit(vec![simple_diagnostic(
            "active_checkpoint_mismatch",
            "active record does not match the exact immutable Checkpoint",
        )]));
    }
    git::require_ancestor(
        repository_root,
        &checkpoint.trusted_commit,
        &snapshot.commit,
        OPERATION,
    )
    .map_err(|error| with_commit(vec![diagnostic(error)]))?;
    let lineage = git::resolve_exact(repository_root, &checkpoint.trusted_commit, OPERATION)
        .map_err(|error| with_commit(vec![diagnostic(error)]))?;
    validation::verify_lineage(&lineage, &checkpoint, &bytes, OPERATION)
        .map_err(|error| with_commit(vec![diagnostic(error)]))?;
    validation::validate_integrated(
        &snapshot.commit,
        &foundation,
        &approvals,
        &checkpoint,
        OPERATION,
    )
    .map_err(|error| with_commit(vec![diagnostic(error)]))?;
    let selected = checkpoint
        .units
        .iter()
        .map(|unit| unit.id.clone())
        .collect::<BTreeSet<_>>();
    let loaded_working_sources;
    let source_record_captures;
    let working_sources = if let Some(sources) = provided_working_sources {
        source_record_captures = Vec::new();
        sources
    } else {
        (loaded_working_sources, source_record_captures) =
            source::load_captured(repository_root).map_err(|diagnostics| AuthorityFailure {
                diagnostics,
                trusted_commit: Some(snapshot.commit.clone()),
                retryable: false,
            })?;
        &loaded_working_sources
    };
    let mut source_evaluation =
        source::evaluate(repository_root, &foundation, working_sources, &selected)
            .map_err(|failure| AuthorityFailure::from_source(&snapshot.commit, failure))?;
    source_evaluation
        .guard
        .add_record_captures(source_record_captures);
    let active = source_evaluation
        .units
        .iter()
        .filter(|(_, state)| state.eligibility == source::Eligibility::Active)
        .map(|(id, _)| id.clone())
        .collect();
    Ok(Some(AuthorityEvaluation {
        trusted_commit: snapshot.commit.clone(),
        approvals,
        approval_evidence,
        active,
        freshness: source_evaluation.units,
        freshness_guard: source_evaluation.guard,
        checkpoint: source_evaluation.checkpoint,
        active_checkpoint: Some(ActiveCheckpoint {
            id: active_record.checkpoint_id,
            hash: active_record.checkpoint_hash,
            active_record_hash: hash_bytes(&active_bytes),
            authority_basis_commit: active_record.trusted_commit,
        }),
    }))
}

fn diagnostic(error: super::OperationFailure) -> Diagnostic {
    Diagnostic {
        phase: DiagnosticPhase::Global,
        path: "refs/heads/develop".to_owned(),
        code: error.error.code,
        message: error.error.message,
        line: None,
        column: None,
        affected_ids: error.error.affected_ids,
    }
}

fn simple_diagnostic(code: &str, message: &str) -> Diagnostic {
    Diagnostic {
        phase: DiagnosticPhase::Global,
        path: "methexis/active-checkpoint.yaml".to_owned(),
        code: code.to_owned(),
        message: message.to_owned(),
        line: None,
        column: None,
        affected_ids: Vec::new(),
    }
}
