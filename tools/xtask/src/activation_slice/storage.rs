use std::path::Path;

use crate::bounded_file;

const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_CONTRACT_BYTES: usize = 64 * 1024;

pub(super) fn read_request(path: &Path) -> Result<Vec<u8>, String> {
    bounded_file::read_regular(path, MAX_REQUEST_BYTES, "activation Slice request")
}

pub(super) fn read_existing_contract(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => bounded_file::read_regular(path, MAX_CONTRACT_BYTES, "activation Slice contract")
            .map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
    }
}

pub(super) fn publish_exact(path: &Path, expected: &[u8]) -> Result<bool, String> {
    bounded_file::publish_new_or_exact(
        path,
        expected,
        MAX_CONTRACT_BYTES,
        "activation Slice contract",
    )
}

pub(super) fn ensure_directory(path: &Path) -> Result<(), String> {
    bounded_file::ensure_directory(path, "activation Slice")
}

pub(super) fn path_entry_exists(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
    }
}
