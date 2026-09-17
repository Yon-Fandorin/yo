use std::{fs, path::Path};

use super::{
    capture, measurements,
    model::{CapturedSource, Request},
};
use crate::{bounded_file, review_protocol, slice_worktree};

const REQUEST_SCHEMA: &str = "yo.slice-cost-report-request/v1alpha1";
const REQUEST_LIMIT: usize = 256 * 1024;
const MAX_SOURCES: usize = 64;

#[derive(serde::Deserialize)]
struct SchemaEnvelope {
    schema: String,
}

pub(super) struct LoadedRequest {
    pub(super) request: Request,
    pub(super) request_bytes: Vec<u8>,
    pub(super) workspace: std::path::PathBuf,
    pub(super) request_path: std::path::PathBuf,
    pub(super) output: std::path::PathBuf,
    pub(super) sources: Vec<CapturedSource>,
}

pub(super) fn load(
    repository: &Path,
    request_path: &Path,
    output: &Path,
) -> Result<LoadedRequest, String> {
    let request_bytes =
        bounded_file::read_regular(request_path, REQUEST_LIMIT, "Slice cost report request")?;
    let request: Request = serde_json::from_slice(&request_bytes)
        .map_err(|error| format!("invalid Slice cost report request: {error}"))?;
    validate_request(&request)?;

    let workspace = slice_worktree::workspace_root(repository)?;
    let request_path = capture::canonical_input(&workspace, request_path)?;
    let output = canonical_output(&workspace, output)?;
    if request_path == output {
        return Err("Slice cost report output must differ from its request".to_owned());
    }

    let source_count = capture::owner_sources(&request.owners)
        .iter()
        .map(|(_, sources)| sources.len())
        .sum::<usize>();
    if source_count > MAX_SOURCES {
        return Err(format!(
            "Slice cost report supports at most {MAX_SOURCES} sources"
        ));
    }

    let mut paths = std::collections::BTreeSet::new();
    let mut identities = std::collections::BTreeSet::new();
    let mut sources = Vec::new();
    for (owner, owner_sources) in capture::owner_sources(&request.owners) {
        for source in owner_sources {
            let path = capture::canonical_input(&workspace, Path::new(&source.path))?;
            if path == request_path || path == output {
                return Err(
                    "Slice cost source paths must differ from request and output".to_owned(),
                );
            }
            let bytes =
                bounded_file::read_regular(&path, capture::SOURCE_LIMIT, "Slice cost source")?;
            let hash = review_protocol::digest(&bytes);
            capture::require_hash(&source.hash, "Slice cost source hash")?;
            if hash != source.hash {
                return Err(format!(
                    "Slice cost source hash changed: {}",
                    path.display()
                ));
            }
            let envelope: SchemaEnvelope = serde_json::from_slice(&bytes)
                .map_err(|error| format!("Slice cost source is not JSON: {error}"))?;
            if envelope.schema != source.schema {
                return Err(format!(
                    "Slice cost source schema changed: {}",
                    path.display()
                ));
            }
            if !paths.insert(path.clone())
                || !identities.insert((envelope.schema.clone(), hash.clone()))
            {
                return Err("Slice cost sources cannot be counted more than once".to_owned());
            }
            sources.push(CapturedSource {
                owner,
                path: path.to_string_lossy().into_owned(),
                hash,
                schema: envelope.schema,
                bytes: bytes.len(),
            });
        }
    }

    Ok(LoadedRequest {
        request,
        request_bytes,
        workspace,
        request_path,
        output,
        sources,
    })
}

pub(super) fn require_request_unchanged(
    request_path: &Path,
    expected: &[u8],
) -> Result<(), String> {
    let current =
        bounded_file::read_regular(request_path, REQUEST_LIMIT, "Slice cost report request")?;
    if current != expected {
        return Err("Slice cost report request changed before publication".to_owned());
    }
    Ok(())
}

fn validate_request(request: &Request) -> Result<(), String> {
    if request.schema != REQUEST_SCHEMA {
        return Err(format!(
            "Slice cost request must use schema `{REQUEST_SCHEMA}`"
        ));
    }
    measurements::require_text(&request.slice, "Slice cost slice")?;
    review_protocol::require_commit(&request.candidate_commit, "Slice cost candidate")?;
    for (label, basis) in [
        ("packet", &request.owners.packet.basis),
        ("provider", &request.owners.provider.basis),
        (
            "coordinator context",
            &request.owners.coordinator_context.basis,
        ),
        ("command output", &request.owners.command_output.basis),
        ("elapsed", &request.owners.elapsed.basis),
    ] {
        measurements::require_text(basis, &format!("{label} basis"))?;
    }
    measurements::validate_owners(&request.owners)
}

fn canonical_output(workspace: &Path, output: &Path) -> Result<std::path::PathBuf, String> {
    let resolved = review_protocol::resolve_input_path(workspace, &output.to_string_lossy());
    let parent = resolved
        .parent()
        .ok_or_else(|| "Slice cost output has no parent".to_owned())?;
    let parent = fs::canonicalize(parent)
        .map_err(|error| format!("cannot resolve Slice cost output parent: {error}"))?;
    let name = resolved
        .file_name()
        .ok_or_else(|| "Slice cost output has no file name".to_owned())?;
    Ok(parent.join(name))
}
