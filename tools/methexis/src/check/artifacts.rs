//! Read-only validation for tracked examples derived from trusted authority.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    path::{Component, Path},
};

use rustix::fs::{Mode, OFlags, open, openat};
use serde::Deserialize;

use super::{Diagnostic, global_diagnostic};
use crate::checkpoint::ActiveCheckpoint;

pub(crate) const TRACKED_ARTIFACTS: &[&str] = &[
    "tools/methexis/examples/context-contract/manifest.json",
    "tools/methexis/examples/context-contract/stable-leaf-manifest.json",
];
const MAX_ARTIFACT_BYTES: usize = 256 * 1024;
const OPEN_DIRECTORY: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

pub(super) fn is_registered(repository_root: &Path) -> bool {
    TRACKED_ARTIFACTS
        .iter()
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
    for relative in TRACKED_ARTIFACTS {
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

pub(crate) fn validate_candidate(
    artifacts: &BTreeMap<String, Vec<u8>>,
    active: &ActiveCheckpoint,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for relative in TRACKED_ARTIFACTS {
        let Some(bytes) = artifacts.get(*relative) else {
            diagnostics.push(global_diagnostic(
                (*relative).to_owned(),
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
    let components = relative.components().collect::<Vec<_>>();
    if components.is_empty()
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("artifact path contains an unsafe component".to_owned());
    }

    let mut directory = open(repository_root, OPEN_DIRECTORY, Mode::empty())
        .map_err(|error| format!("cannot open repository root: {error}"))?;
    for component in components.iter().take(components.len() - 1) {
        let Component::Normal(name) = component else {
            return Err("artifact path contains an unsafe component".to_owned());
        };
        directory = openat(&directory, *name, OPEN_DIRECTORY, Mode::empty())
            .map_err(|error| format!("cannot open artifact directory: {error}"))?;
    }
    let Component::Normal(name) = components[components.len() - 1] else {
        return Err("artifact path contains an unsafe final component".to_owned());
    };
    let descriptor = openat(
        &directory,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|error| format!("cannot open artifact file: {error}"))?;
    let mut file = File::from(descriptor);
    if !file
        .metadata()
        .map_err(|error| format!("cannot inspect artifact file: {error}"))?
        .is_file()
    {
        return Err("artifact path is not a regular file".to_owned());
    }

    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_ARTIFACT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read artifact file: {error}"))?;
    if bytes.len() > MAX_ARTIFACT_BYTES {
        return Err(format!(
            "artifact exceeds the {MAX_ARTIFACT_BYTES}-byte limit"
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
