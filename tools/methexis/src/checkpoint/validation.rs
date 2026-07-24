//! Required-closure selection from one validated trusted snapshot.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::{
    CheckpointRecord, CheckpointUnit, OperationFailure, SelectedCheckpoint, git::TrustedSnapshot,
    records::build_checkpoint,
};
use crate::{
    check::{Foundation, load_foundation},
    review::validate_records,
};

pub(super) fn select(
    snapshot: &TrustedSnapshot,
    requested_roots: &[String],
    operation: &'static str,
) -> Result<SelectedCheckpoint, OperationFailure> {
    let foundation = load_foundation(&snapshot.root).map_err(|diagnostics| {
        let message = diagnostics.first().map_or_else(
            || "trusted foundation is invalid".to_owned(),
            |item| item.message.clone(),
        );
        OperationFailure::new(
            operation,
            Some(snapshot.commit.clone()),
            "trusted_foundation_invalid",
            message,
            diagnostics
                .into_iter()
                .flat_map(|item| item.affected_ids)
                .collect(),
            "repair and review the trusted foundation",
        )
    })?;
    let review = validate_records(&snapshot.root, &foundation);
    if !review.diagnostics.is_empty() {
        let affected = review
            .diagnostics
            .iter()
            .flat_map(|item| item.affected_ids.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        return Err(OperationFailure::new(
            operation,
            Some(snapshot.commit.clone()),
            "trusted_review_evidence_invalid",
            review.diagnostics[0].message.clone(),
            affected,
            "repair and review the trusted approval evidence",
        ));
    }

    select_from_foundation(
        &snapshot.commit,
        &foundation,
        &review.states,
        requested_roots,
        operation,
    )
}

pub(super) fn validate_integrated(
    commit: &str,
    foundation: &Foundation,
    approvals: &BTreeMap<String, String>,
    checkpoint: &CheckpointRecord,
    operation: &'static str,
) -> Result<(), OperationFailure> {
    let states = approvals
        .keys()
        .map(|id| {
            (
                id.clone(),
                crate::review::ProposalState {
                    evidence: "matching_proposal",
                    reason: None,
                },
            )
        })
        .collect();
    let expected =
        select_from_foundation(commit, foundation, &states, &checkpoint.roots, operation)?;
    if expected.units != checkpoint.units {
        return Err(failure(
            operation,
            commit,
            "active_checkpoint_selection_mismatch",
            "active Checkpoint does not match the current approved required closure",
            checkpoint
                .units
                .iter()
                .map(|unit| unit.id.clone())
                .collect(),
        ));
    }
    Ok(())
}

pub(super) fn verify_lineage(
    snapshot: &TrustedSnapshot,
    checkpoint: &CheckpointRecord,
    bytes: &[u8],
    operation: &'static str,
) -> Result<(), OperationFailure> {
    let selected = select(snapshot, &checkpoint.roots, operation)?;
    let (_, expected, _) = build_checkpoint(&snapshot.commit, selected.roots, selected.units)?;
    if expected != bytes {
        return Err(failure(
            operation,
            &snapshot.commit,
            "checkpoint_lineage_mismatch",
            "Checkpoint bytes cannot be reproduced from the recorded trusted commit",
            checkpoint
                .units
                .iter()
                .map(|unit| unit.id.clone())
                .collect(),
        ));
    }
    Ok(())
}

fn select_from_foundation(
    commit: &str,
    foundation: &Foundation,
    states: &BTreeMap<String, crate::review::ProposalState>,
    requested_roots: &[String],
    operation: &'static str,
) -> Result<SelectedCheckpoint, OperationFailure> {
    let mut roots = requested_roots.to_vec();
    roots.sort();
    roots.dedup();
    if roots.is_empty() {
        return Err(failure(
            operation,
            commit,
            "empty_checkpoint_roots",
            "Checkpoint request must contain at least one root KnowledgeId",
            Vec::new(),
        ));
    }

    let units = foundation
        .units
        .iter()
        .map(|unit| (unit.metadata.id.as_str(), unit))
        .collect::<BTreeMap<_, _>>();
    let mut reasons = BTreeMap::<String, BTreeSet<String>>::new();
    let mut queue = VecDeque::new();
    for root in &roots {
        if !units.contains_key(root.as_str()) {
            return Err(failure(
                operation,
                commit,
                "unknown_checkpoint_root",
                format!("root KnowledgeId `{root}` does not exist"),
                vec![root.clone()],
            ));
        }
        reasons
            .entry(root.clone())
            .or_default()
            .insert(format!("root:{root}"));
        queue.push_back(root.clone());
    }

    while let Some(id) = queue.pop_front() {
        let unit = units[id.as_str()];
        if states.get(&id).map(|state| state.evidence) != Some("matching_proposal") {
            return Err(failure(
                operation,
                commit,
                "trusted_approval_missing",
                format!("KnowledgeId `{id}` lacks exact trusted approval"),
                vec![id],
            ));
        }
        for target in unit.metadata.relations.required_targets() {
            if !units.contains_key(target.as_str()) {
                return Err(failure(
                    operation,
                    commit,
                    "required_dependency_missing",
                    format!("KnowledgeId `{id}` requires missing `{target}`"),
                    vec![id.clone(), target.clone()],
                ));
            }
            let inserted = reasons
                .entry(target.clone())
                .or_default()
                .insert(format!("required_by:{id}"));
            if inserted {
                queue.push_back(target.clone());
            }
        }
    }

    let selected_ids = reasons.keys().cloned().collect::<BTreeSet<_>>();
    for replacement in &selected_ids {
        for replaced in &units[replacement.as_str()].metadata.relations.supersedes {
            if selected_ids.contains(replaced) {
                return Err(failure(
                    operation,
                    commit,
                    "superseded_units_co_selected",
                    format!(
                        "Checkpoint cannot select replacement `{replacement}` with superseded `{replaced}`"
                    ),
                    vec![replacement.clone(), replaced.clone()],
                ));
            }
        }
    }

    let selected = reasons
        .into_iter()
        .map(|(id, reasons)| CheckpointUnit {
            revision: units[id.as_str()].revision.clone(),
            id,
            reasons: reasons.into_iter().collect(),
        })
        .collect();
    Ok(SelectedCheckpoint {
        roots,
        units: selected,
    })
}

fn failure(
    operation: &'static str,
    commit: &str,
    code: &'static str,
    message: impl Into<String>,
    affected_ids: Vec<String>,
) -> OperationFailure {
    OperationFailure::new(
        operation,
        Some(commit.to_owned()),
        code,
        message,
        affected_ids,
        "repair the trusted approval closure and retry",
    )
}
