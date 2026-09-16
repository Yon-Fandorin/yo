use std::path::Path;

use crate::slice_worktree;

mod action;
mod delivery;
mod lineage;
mod model;
mod scan;

#[cfg(test)]
mod tests;

use action::{next_action, next_invocation};
pub(crate) use lineage::locate;
use lineage::scan_review_lineage;
use model::{CoordinationScope, RESULT_SCHEMA, ResultDocument, ScanBudget};
use scan::scan_coordination;

pub(crate) fn run(repository: &Path, slice: &str) -> Result<(), String> {
    let state = locate(repository, slice)?;
    let workspace = slice_worktree::workspace_root(repository)?;
    let coordination = workspace
        .join(".local-exclude")
        .join("coordination")
        .join(slice);
    let mut budget = ScanBudget::default();
    let reviews = scan_review_lineage(&state, &workspace, &mut budget)?;
    let artifacts = scan_coordination(
        &coordination,
        &CoordinationScope {
            repository: &state.worktree,
            workspace: &workspace,
            candidate: &state.head,
            current_review_ids: &reviews.current_review_ids,
            latest_review_ids: &reviews.latest_review_ids,
            current_validations: &reviews.current_validations,
        },
        &mut budget,
    )?;
    let next_action = next_action(&state, &reviews, &artifacts);
    let (next_argv, blocking_reason) = next_invocation(slice, next_action, &artifacts);
    let next_working_directory = (next_action == "run_gate" && next_argv.is_some())
        .then(|| state.worktree.display().to_string());
    let result = ResultDocument {
        schema: RESULT_SCHEMA,
        ok: true,
        slice: slice.to_owned(),
        branch: state.branch,
        base_commit: state.bound.base,
        candidate_commit: state.head,
        clean: state.clean,
        review_lineage: reviews.status,
        review_packets: reviews.packets,
        review_rounds: artifacts.review_rounds,
        review_chain: reviews.current_review_ids.iter().cloned().collect(),
        latest_packet_candidate: reviews.latest_candidate,
        validation_summaries: artifacts.validations,
        gate_requests: artifacts.gate_requests,
        delivery_claims: artifacts.claims,
        delivery_receipts: artifacts.delivery_receipts,
        durable_external_requests: artifacts.durable_requests,
        superseded_artifacts: artifacts.superseded,
        delivery: artifacts.delivery,
        blocking_reason,
        next_action,
        next_argv,
        next_working_directory,
    };
    println!(
        "{}",
        serde_json::to_string(&result)
            .map_err(|error| format!("cannot encode compact Slice status: {error}"))?
    );
    Ok(())
}
