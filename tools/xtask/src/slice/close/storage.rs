use std::{fs::File, io::Read, path::Path};

use rustix::fs::{FileType, Mode, OFlags, fstat, open};

use crate::bounded_file;

const MAX_PLAN_BYTES: usize = 64 * 1024;
const READ_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::NONBLOCK)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

pub(super) fn read_plan(path: &Path) -> Result<Vec<u8>, String> {
    let fd = open(path, READ_FLAGS, Mode::empty())
        .map_err(|error| format!("cannot open Slice close plan {}: {error}", path.display()))?;
    let stat = fstat(&fd).map_err(|error| {
        format!(
            "cannot inspect Slice close plan {}: {error}",
            path.display()
        )
    })?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err("Slice close plan must be a regular file".to_owned());
    }
    let declared = usize::try_from(stat.st_size)
        .map_err(|_| "Slice close plan has an unsupported size".to_owned())?;
    if declared > MAX_PLAN_BYTES {
        return Err(format!(
            "Slice close plan exceeds the {MAX_PLAN_BYTES}-byte limit"
        ));
    }
    let mut bytes = Vec::with_capacity(declared.min(MAX_PLAN_BYTES));
    File::from(fd)
        .take((MAX_PLAN_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read Slice close plan {}: {error}", path.display()))?;
    if bytes.len() > MAX_PLAN_BYTES {
        return Err(format!(
            "Slice close plan exceeds the {MAX_PLAN_BYTES}-byte limit"
        ));
    }
    Ok(bytes)
}

pub(super) fn publish_plan(path: &Path, bytes: &[u8]) -> Result<bool, String> {
    bounded_file::publish_new_or_exact(path, bytes, MAX_PLAN_BYTES, "Slice close plan")
}

pub(super) fn remove_coordination_contract(
    path: &Path,
    expected_hash: &str,
) -> Result<bool, String> {
    bounded_file::remove_regular_matching_sha256(
        path,
        expected_hash,
        MAX_PLAN_BYTES,
        "Slice coordination contract",
    )
}
