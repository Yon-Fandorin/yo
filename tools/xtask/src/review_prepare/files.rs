use std::path::Path;

use rustix::fs::Dir;
use serde::Serialize;

use crate::bounded_file;

pub(super) const REQUEST_LIMIT: usize = 256 * 1024;
pub(super) const GENERATED_REQUEST_LIMIT: usize = 256 * 1024;
pub(super) const AUTHORIZATION_LIMIT: usize = 64 * 1024;

pub(super) struct PreparedPaths<'a> {
    pub(super) context: &'a Path,
    pub(super) review: &'a Path,
    pub(super) egress: &'a Path,
    pub(super) admission: &'a Path,
    pub(super) delivery: &'a Path,
    pub(super) delivery_output: &'a Path,
}

pub(super) struct PreparedBytes<'a> {
    pub(super) context: &'a [u8],
    pub(super) review: &'a [u8],
    pub(super) egress: &'a [u8],
    pub(super) admission: &'a [u8],
    pub(super) delivery: &'a [u8],
}

pub(super) fn canonical_json(value: &impl Serialize) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("cannot encode prepared review request: {error}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(super) fn publish(path: &Path, bytes: &[u8], label: &str) -> Result<bool, String> {
    bounded_file::publish_new_or_exact(path, bytes, GENERATED_REQUEST_LIMIT, label)
}

pub(super) fn require_exact(path: &Path, expected: &[u8], label: &str) -> Result<(), String> {
    let current = bounded_file::read_regular(path, GENERATED_REQUEST_LIMIT, label)?;
    if current == expected {
        Ok(())
    } else {
        Err(format!("{label} changed during review preparation"))
    }
}

pub(super) fn require_current(path: &Path, expected: &[u8]) -> Result<(), String> {
    let current =
        bounded_file::read_regular(path, REQUEST_LIMIT, "Slice review preparation request")?;
    if current == expected {
        Ok(())
    } else {
        Err("Slice review preparation request changed during preparation".to_owned())
    }
}

pub(super) fn require_empty_directory(path: &Path) -> Result<(), String> {
    let directory = bounded_file::open_directory(path, "review delivery output")?;
    let mut entries = Dir::read_from(&directory)
        .map_err(|error| format!("cannot enumerate review delivery output: {error}"))?;
    while let Some(entry) = entries.read() {
        let entry =
            entry.map_err(|error| format!("cannot enumerate review delivery output: {error}"))?;
        let name = entry.file_name().to_bytes();
        if name != b"." && name != b".." {
            return Err(
                "review delivery output is not empty; inspect its existing claim or result"
                    .to_owned(),
            );
        }
    }
    Ok(())
}
