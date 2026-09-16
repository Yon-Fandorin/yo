use std::path::Path;

use super::{
    files::{
        PreparedBytes, PreparedPaths, require_current, require_empty_directory, require_exact,
    },
    model::Target,
    target::{TargetPreparation, authorize_route, egress_document},
};
use crate::{review_packet, review_target_admission, slice_contract, slice_worktree};

#[allow(clippy::too_many_arguments)]
pub(super) fn final_revalidate(
    repository: &Path,
    workspace: &Path,
    request_path: &Path,
    request_bytes: &[u8],
    target: &Target,
    prepared_target: &TargetPreparation,
    published: &review_packet::PublishedReview,
    bound: &slice_contract::BoundSlice,
    paths: PreparedPaths<'_>,
    bytes: PreparedBytes<'_>,
) -> Result<(), String> {
    require_current(request_path, request_bytes)?;
    require_prepared_requests_current(&paths, &bytes)?;
    require_empty_directory(paths.delivery_output)?;
    require_same_bound_slice(repository, bound)?;
    slice_worktree::ensure_clean(
        repository,
        "candidate worktree",
        "returning review preparation",
    )?;
    if slice_worktree::resolve_commit(repository, "HEAD")? != published.candidate_commit {
        return Err("candidate HEAD changed during review preparation".to_owned());
    }
    if slice_worktree::resolve_commit(repository, "refs/heads/develop")? != published.trusted_commit
    {
        return Err("trusted integration changed during review preparation".to_owned());
    }

    let current_egress = egress_document(workspace, target, published)?;
    if current_egress != bytes.egress {
        return Err("review authorization changed during review preparation".to_owned());
    }
    authorize_route(repository, prepared_target.kind, paths.egress, published)?;
    require_eligible_admission(paths.admission, prepared_target.next_action)
}

pub(super) fn require_prepared_requests_current(
    paths: &PreparedPaths<'_>,
    bytes: &PreparedBytes<'_>,
) -> Result<(), String> {
    require_exact(paths.context, bytes.context, "ContextBuild request")?;
    require_exact(paths.review, bytes.review, "Slice review packet request")?;
    require_exact(paths.egress, bytes.egress, "review egress request")?;
    require_exact(
        paths.admission,
        bytes.admission,
        "review target admission request",
    )?;
    require_exact(paths.delivery, bytes.delivery, "review delivery request")
}

pub(super) fn require_eligible_admission(
    path: &Path,
    expected_next_action: &str,
) -> Result<(), String> {
    let admission = review_target_admission::evaluate(path)?;
    let value = serde_json::to_value(admission)
        .map_err(|error| format!("cannot inspect review admission result: {error}"))?;
    let ok = value.get("ok").and_then(serde_json::Value::as_bool) == Some(true);
    let next_action = value
        .get("next_action")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    if !ok || next_action != expected_next_action {
        return Err(format!(
            "review target admission stopped preparation with next action `{next_action}`"
        ));
    }
    Ok(())
}

fn require_same_bound_slice(
    repository: &Path,
    expected: &slice_contract::BoundSlice,
) -> Result<(), String> {
    let current = slice_contract::trusted_bound_slice(repository)?;
    if &current == expected {
        Ok(())
    } else {
        Err("Slice binding or contract changed during review preparation".to_owned())
    }
}
