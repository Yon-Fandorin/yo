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
    check::{Diagnostic, DiagnosticPhase, Foundation, load_foundation},
    review::validate_records,
    source::{self, UnitFreshness},
};

pub(crate) struct AuthorityEvaluation {
    pub(crate) trusted_commit: String,
    pub(crate) approvals: BTreeMap<String, String>,
    pub(crate) active: BTreeSet<String>,
    pub(crate) freshness: BTreeMap<String, UnitFreshness>,
    pub(crate) checkpoint: &'static str,
}

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

pub(crate) fn evaluate(
    repository_root: &Path,
    working: &Foundation,
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

    let active_path = snapshot.root.join("methexis/active-checkpoint.yaml");
    if !active_path.exists() {
        return Ok(Some(AuthorityEvaluation {
            trusted_commit: snapshot.commit.clone(),
            approvals,
            active: BTreeSet::new(),
            freshness: BTreeMap::new(),
            checkpoint: "inactive",
        }));
    }
    let (active_record, _) = read_active(&active_path, OPERATION)
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
    let source_evaluation = source::evaluate(repository_root, &foundation, working, &selected)
        .map_err(|failure| {
            let retryable = failure.code == "source_changed_during_validation";
            AuthorityFailure {
                diagnostics: vec![Diagnostic {
                    phase: DiagnosticPhase::Global,
                    path: "methexis/sources".to_owned(),
                    code: failure.code.to_owned(),
                    message: failure.message,
                    line: None,
                    column: None,
                    affected_ids: failure.affected_ids,
                }],
                trusted_commit: Some(snapshot.commit.clone()),
                retryable,
            }
        })?;
    let active = source_evaluation
        .units
        .iter()
        .filter(|(_, state)| state.eligibility == source::Eligibility::Active)
        .map(|(id, _)| id.clone())
        .collect();
    Ok(Some(AuthorityEvaluation {
        trusted_commit: snapshot.commit.clone(),
        approvals,
        active,
        freshness: source_evaluation.units,
        checkpoint: source_evaluation.checkpoint,
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
