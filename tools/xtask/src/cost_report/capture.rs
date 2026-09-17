use std::{fs, path, path::Path};

use super::model::{Owners, Request};
use crate::{bounded_file, review_protocol};

pub(super) const SOURCE_LIMIT: usize = 4 * 1024 * 1024;

pub(super) fn owner_sources(owners: &Owners) -> [(&'static str, &[super::model::Source]); 5] {
    [
        ("packet", &owners.packet.sources),
        ("provider", &owners.provider.sources),
        ("coordinator_context", &owners.coordinator_context.sources),
        ("command_output", &owners.command_output.sources),
        ("elapsed", &owners.elapsed.sources),
    ]
}

pub(super) fn revalidate_sources(request: &Request, workspace: &Path) -> Result<(), String> {
    for (_, sources) in owner_sources(&request.owners) {
        for source in sources {
            let path = canonical_input(workspace, Path::new(&source.path))?;
            let bytes = bounded_file::read_regular(&path, SOURCE_LIMIT, "Slice cost source")?;
            if review_protocol::digest(&bytes) != source.hash {
                return Err(format!(
                    "Slice cost source changed before publication: {}",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn canonical_input(workspace: &Path, input: &Path) -> Result<path::PathBuf, String> {
    let resolved = review_protocol::resolve_input_path(workspace, &input.to_string_lossy());
    fs::canonicalize(&resolved).map_err(|error| {
        format!(
            "cannot resolve Slice cost input {}: {error}",
            resolved.display()
        )
    })
}

pub(super) fn require_hash(value: &str, label: &str) -> Result<(), String> {
    if value.strip_prefix("sha256:").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    }) {
        Ok(())
    } else {
        Err(format!("{label} must be canonical SHA-256"))
    }
}
