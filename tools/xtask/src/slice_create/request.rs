use std::{fs, io, path::Path};

use crate::{bounded_file, slice_contract};

pub(super) const MAX_CONTRACT_BYTES: usize = 64 * 1024;

pub(super) fn read_contract(path: &Path) -> Result<Vec<u8>, String> {
    bounded_file::read_regular(path, MAX_CONTRACT_BYTES, "Slice contract input")
}

pub(super) fn parse_contract(bytes: &[u8]) -> Result<slice_contract::SliceContract, String> {
    serde_json::from_slice(bytes).map_err(|error| format!("invalid Slice contract input: {error}"))
}

pub(super) fn validate_base_ref_shape(base_ref: &str) -> Result<(), String> {
    if base_ref == "refs/heads/develop" || base_ref.starts_with("refs/heads/wave/") {
        Ok(())
    } else {
        Err("Slice base_ref must identify develop or a Wave integration branch".to_owned())
    }
}
pub(super) fn validate_slice_name(slice: &str) -> Result<(), String> {
    if slice.is_empty()
        || slice != slice.trim()
        || slice.contains('/')
        || matches!(slice, "." | "..")
    {
        Err("Slice name must be one non-empty branch segment".to_owned())
    } else {
        Ok(())
    }
}
pub(super) fn read_optional_contract(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            bounded_file::read_regular(path, MAX_CONTRACT_BYTES, "Slice coordination contract")
                .map(Some)
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
    }
}
