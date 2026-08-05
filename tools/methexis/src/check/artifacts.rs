//! Read-only validation for tracked examples derived from trusted authority.

use std::{collections::BTreeMap, fs, path::Path};

use serde::Deserialize;

use super::{Diagnostic, global_diagnostic};
use crate::{checkpoint::ActiveCheckpoint, context, context::registry, publication};
const MAX_ARTIFACT_BYTES: usize = 256 * 1024;

pub(super) fn is_registered(repository_root: &Path) -> bool {
    registry::manifest_paths()
        .any(|relative| fs::symlink_metadata(repository_root.join(relative)).is_ok())
}

#[derive(Deserialize)]
struct TrackedArtifact {
    plan: TrackedArtifactPlan,
}

#[derive(Deserialize)]
struct TrackedArtifactPlan {
    checkpoint: TrackedArtifactCheckpoint,
}

#[derive(Deserialize)]
struct TrackedArtifactCheckpoint {
    id: String,
    hash: String,
    authority_basis_commit: String,
}

pub(super) fn validate(repository_root: &Path, active: &ActiveCheckpoint) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let _transaction_guard = match context::manifest_refresh_reader_guard(repository_root) {
        Ok(guard) => guard,
        Err(error) => return vec![pending_transaction_diagnostic(error)],
    };
    for relative in registry::manifest_paths() {
        let bytes = match read_bounded(repository_root, Path::new(relative)) {
            Ok(bytes) => bytes,
            Err(error) => {
                diagnostics.push(global_diagnostic(
                    (*relative).to_owned(),
                    "tracked_artifact_unreadable",
                    format!("cannot safely read tracked authority-derived artifact: {error}"),
                    Vec::new(),
                ));
                continue;
            },
        };
        validate_bytes(relative, &bytes, active, &mut diagnostics);
    }
    diagnostics
}

pub(crate) fn pending_transaction_diagnostic(message: String) -> Diagnostic {
    global_diagnostic(
        "tools/methexis/examples/context-contract/.manifest-refresh-transaction.json".to_owned(),
        "manifest_refresh_transaction_pending",
        message,
        Vec::new(),
    )
}

pub(crate) fn validate_candidate(
    artifacts: &BTreeMap<String, Vec<u8>>,
    active: &ActiveCheckpoint,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for relative in registry::manifest_paths() {
        let Some(bytes) = artifacts.get(relative) else {
            diagnostics.push(global_diagnostic(
                relative.to_owned(),
                "tracked_artifact_candidate_missing",
                "prospective activation must stage every registered authority-derived artifact"
                    .to_owned(),
                Vec::new(),
            ));
            continue;
        };
        validate_bytes(relative, bytes, active, &mut diagnostics);
    }
    diagnostics
}

fn validate_bytes(
    relative: &str,
    bytes: &[u8],
    active: &ActiveCheckpoint,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if bytes.len() > MAX_ARTIFACT_BYTES {
        diagnostics.push(global_diagnostic(
            relative.to_owned(),
            "tracked_artifact_unreadable",
            format!("artifact exceeds the {MAX_ARTIFACT_BYTES}-byte limit"),
            Vec::new(),
        ));
        return;
    }
    let artifact: TrackedArtifact = match serde_json::from_slice(bytes) {
        Ok(artifact) => artifact,
        Err(error) => {
            diagnostics.push(global_diagnostic(
                relative.to_owned(),
                "invalid_tracked_artifact",
                format!("cannot parse tracked authority-derived artifact: {error}"),
                Vec::new(),
            ));
            return;
        },
    };
    let checkpoint = artifact.plan.checkpoint;
    if checkpoint.id != active.id
        || checkpoint.hash != active.hash
        || checkpoint.authority_basis_commit != active.authority_basis_commit
    {
        diagnostics.push(global_diagnostic(
            relative.to_owned(),
            "stale_tracked_artifact",
            "tracked artifact Checkpoint provenance differs from selected authority".to_owned(),
            Vec::new(),
        ));
    }
}

fn read_bounded(repository_root: &Path, relative: &Path) -> Result<Vec<u8>, String> {
    publication::capture_file(
        repository_root,
        &repository_root.join(relative),
        MAX_ARTIFACT_BYTES,
    )
    .map(|capture| capture.bytes().to_vec())
    .map_err(|error| format!("cannot capture artifact: {error:?}"))
}

#[cfg(test)]
mod tests;
