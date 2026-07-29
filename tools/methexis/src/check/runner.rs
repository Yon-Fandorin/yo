//! Ordered check planning, execution, and report composition.

use std::{collections::BTreeSet, path::Path};

use super::{
    CHECK_SCHEMA, CheckClass, CheckOutcome, CheckReport, CheckStatus, Diagnostic, UnitRevision,
    artifacts, display_path, global_diagnostic, load_records, snapshot_revision, sort_diagnostics,
    validate_global,
};

pub(super) fn check_repository_selected(
    repository_root: &Path,
    requested: &[CheckClass],
) -> CheckReport {
    let requested = if requested.is_empty() {
        CheckClass::ALL.to_vec()
    } else {
        canonical_checks(requested)
    };
    let planned = planned_checks(&requested);
    let mut executed = Vec::new();
    let mut outcomes = Vec::new();

    executed.push(CheckClass::Records);
    let mut foundation = match load_records(repository_root) {
        Ok(foundation) => foundation,
        Err(mut diagnostics) => {
            sort_diagnostics(&mut diagnostics);
            outcomes.push(outcome(CheckClass::Records, CheckStatus::Failed));
            block_remaining(&planned, &mut outcomes);
            return failed_report(diagnostics, requested, executed, outcomes);
        },
    };
    outcomes.push(outcome(CheckClass::Records, CheckStatus::Passed));

    if !planned.contains(&CheckClass::Relations) {
        return successful_partial_report(requested, executed, outcomes);
    }
    executed.push(CheckClass::Relations);
    let mut diagnostics = validate_global(
        &foundation.units,
        &foundation.owners,
        &foundation.sources,
        repository_root,
    );
    sort_diagnostics(&mut diagnostics);
    if !diagnostics.is_empty() {
        outcomes.push(outcome(CheckClass::Relations, CheckStatus::Failed));
        block_remaining(&planned, &mut outcomes);
        return failed_report(diagnostics, requested, executed, outcomes);
    }
    outcomes.push(outcome(CheckClass::Relations, CheckStatus::Passed));
    foundation
        .units
        .sort_by(|left, right| left.metadata.id.cmp(&right.metadata.id));
    let snapshot_revision = snapshot_revision(&foundation.units);

    if !planned.contains(&CheckClass::Authority) {
        let mut report = successful_partial_report(requested, executed, outcomes);
        report.snapshot_revision = Some(snapshot_revision);
        return report;
    }
    executed.push(CheckClass::Authority);
    let review_validation = crate::review::validate_records(repository_root, &foundation);
    if !review_validation.diagnostics.is_empty() {
        outcomes.push(outcome(CheckClass::Authority, CheckStatus::Failed));
        block_remaining(&planned, &mut outcomes);
        return failed_report(review_validation.diagnostics, requested, executed, outcomes);
    }
    let authority = match crate::checkpoint::evaluate(repository_root, Some(&foundation.sources)) {
        Ok(authority) => authority,
        Err(mut failure) => {
            sort_diagnostics(&mut failure.diagnostics);
            outcomes.push(outcome(CheckClass::Authority, CheckStatus::Failed));
            block_remaining(&planned, &mut outcomes);
            return failed_authority_report_with_checks(failure, requested, executed, outcomes);
        },
    };
    outcomes.push(outcome(CheckClass::Authority, CheckStatus::Passed));

    if planned.contains(&CheckClass::Artifacts) {
        if !artifacts::is_registered(repository_root) {
            executed.push(CheckClass::Artifacts);
            outcomes.push(outcome(CheckClass::Artifacts, CheckStatus::Passed));
        } else {
            let Some(active) = authority
                .as_ref()
                .and_then(|authority| authority.active_checkpoint.as_ref())
            else {
                outcomes.push(outcome(CheckClass::Artifacts, CheckStatus::Blocked));
                let diagnostic = global_diagnostic(
                    "tools/methexis/examples/context-contract".to_owned(),
                    "tracked_artifact_authority_unavailable",
                    "tracked authority-derived artifacts require an active trusted Checkpoint"
                        .to_owned(),
                    Vec::new(),
                );
                let mut report = failed_report(vec![diagnostic], requested, executed, outcomes);
                report.next_actions = vec![
                    "integrate and activate trusted authority before checking tracked artifacts"
                        .to_owned(),
                ];
                return report;
            };
            executed.push(CheckClass::Artifacts);
            let mut diagnostics = artifacts::validate(repository_root, active);
            sort_diagnostics(&mut diagnostics);
            if !diagnostics.is_empty() {
                outcomes.push(outcome(CheckClass::Artifacts, CheckStatus::Failed));
                return failed_report(diagnostics, requested, executed, outcomes);
            }
            outcomes.push(outcome(CheckClass::Artifacts, CheckStatus::Passed));
        }
    }

    let unit_revisions = foundation
        .units
        .into_iter()
        .map(|unit| {
            let state = review_validation.states.get(&unit.metadata.id);
            let trusted_approval = authority.as_ref().is_some_and(|authority| {
                authority.approvals.get(&unit.metadata.id) == Some(&unit.revision)
            });
            let active = trusted_approval
                && authority
                    .as_ref()
                    .is_some_and(|authority| authority.active.contains(&unit.metadata.id));
            let freshness = authority
                .as_ref()
                .and_then(|authority| authority.freshness.get(&unit.metadata.id));
            UnitRevision {
                id: unit.metadata.id,
                revision: unit.revision,
                path: display_path(&unit.path, repository_root),
                effective_approval: if trusted_approval {
                    "approved"
                } else {
                    "draft"
                },
                approval_evidence: if trusted_approval {
                    "trusted_approval"
                } else {
                    state.map_or("missing", |state| state.evidence)
                },
                approval_reason: if trusted_approval {
                    None
                } else {
                    state.and_then(|state| state.reason)
                },
                eligibility: if active {
                    "active"
                } else if trusted_approval {
                    freshness.map_or("inactive", |state| state.eligibility.as_str())
                } else if authority.is_some() {
                    "inactive"
                } else {
                    "not_evaluated"
                },
                eligibility_evidence: if !trusted_approval {
                    Vec::new()
                } else {
                    freshness.map_or_else(Vec::new, |state| state.evidence.clone())
                },
            }
        })
        .collect();

    CheckReport {
        schema: CHECK_SCHEMA,
        ok: true,
        requested_checks: requested,
        executed_checks: executed,
        checks: outcomes,
        authority: "draft",
        approval: if authority.is_some() {
            "trusted_evaluated"
        } else {
            "proposal_evaluated"
        },
        checkpoint: authority
            .as_ref()
            .map_or("not_evaluated", |authority| authority.checkpoint),
        retryable: false,
        trusted_commit: authority
            .as_ref()
            .map(|authority| authority.trusted_commit.clone()),
        snapshot_revision: Some(snapshot_revision),
        affected_ids: Vec::new(),
        units: unit_revisions,
        diagnostics: Vec::new(),
        next_actions: Vec::new(),
    }
}

fn failed_report(
    diagnostics: Vec<Diagnostic>,
    requested_checks: Vec<CheckClass>,
    executed_checks: Vec<CheckClass>,
    checks: Vec<CheckOutcome>,
) -> CheckReport {
    failed_report_with_context(
        diagnostics,
        None,
        false,
        requested_checks,
        executed_checks,
        checks,
    )
}

#[cfg(test)]
pub(super) fn failed_authority_report(failure: crate::checkpoint::AuthorityFailure) -> CheckReport {
    failed_authority_report_with_checks(
        failure,
        CheckClass::ALL.to_vec(),
        [
            CheckClass::Records,
            CheckClass::Relations,
            CheckClass::Authority,
        ]
        .to_vec(),
        vec![
            outcome(CheckClass::Records, CheckStatus::Passed),
            outcome(CheckClass::Relations, CheckStatus::Passed),
            outcome(CheckClass::Authority, CheckStatus::Failed),
            outcome(CheckClass::Artifacts, CheckStatus::Blocked),
        ],
    )
}

fn failed_authority_report_with_checks(
    failure: crate::checkpoint::AuthorityFailure,
    requested_checks: Vec<CheckClass>,
    executed_checks: Vec<CheckClass>,
    checks: Vec<CheckOutcome>,
) -> CheckReport {
    failed_report_with_context(
        failure.diagnostics,
        failure.trusted_commit,
        failure.retryable,
        requested_checks,
        executed_checks,
        checks,
    )
}

fn failed_report_with_context(
    diagnostics: Vec<Diagnostic>,
    trusted_commit: Option<String>,
    retryable: bool,
    requested_checks: Vec<CheckClass>,
    executed_checks: Vec<CheckClass>,
    checks: Vec<CheckOutcome>,
) -> CheckReport {
    let affected_ids = diagnostics
        .iter()
        .flat_map(|diagnostic| diagnostic.affected_ids.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    CheckReport {
        schema: CHECK_SCHEMA,
        ok: false,
        requested_checks,
        executed_checks,
        checks,
        authority: "draft",
        approval: "not_evaluated",
        checkpoint: "not_evaluated",
        retryable,
        trusted_commit,
        snapshot_revision: None,
        affected_ids,
        units: Vec::new(),
        diagnostics,
        next_actions: if retryable {
            vec!["retry `methexis check`; no state was published".to_owned()]
        } else {
            vec!["fix the listed diagnostics and rerun `methexis check`".to_owned()]
        },
    }
}

fn canonical_checks(checks: &[CheckClass]) -> Vec<CheckClass> {
    CheckClass::ALL
        .into_iter()
        .filter(|candidate| checks.contains(candidate))
        .collect()
}

fn planned_checks(requested: &[CheckClass]) -> Vec<CheckClass> {
    CheckClass::ALL
        .into_iter()
        .filter(|candidate| {
            requested
                .iter()
                .any(|requested| requested.prerequisites().contains(candidate))
        })
        .collect()
}

fn outcome(check: CheckClass, status: CheckStatus) -> CheckOutcome {
    CheckOutcome { check, status }
}

fn block_remaining(planned: &[CheckClass], outcomes: &mut Vec<CheckOutcome>) {
    for check in planned {
        if !outcomes.iter().any(|outcome| outcome.check == *check) {
            outcomes.push(outcome(*check, CheckStatus::Blocked));
        }
    }
}

fn successful_partial_report(
    requested_checks: Vec<CheckClass>,
    executed_checks: Vec<CheckClass>,
    checks: Vec<CheckOutcome>,
) -> CheckReport {
    CheckReport {
        schema: CHECK_SCHEMA,
        ok: true,
        requested_checks,
        executed_checks,
        checks,
        authority: "draft",
        approval: "not_evaluated",
        checkpoint: "not_evaluated",
        retryable: false,
        trusted_commit: None,
        snapshot_revision: None,
        affected_ids: Vec::new(),
        units: Vec::new(),
        diagnostics: Vec::new(),
        next_actions: Vec::new(),
    }
}
