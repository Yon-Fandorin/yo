//! Captured registered inputs for a prospective ContextBuild manifest refresh.

use std::{io, path::Path};

use super::{RefreshFailure, io_failure, publication_failure};
use crate::publication::{self, CapturedFile, TargetLock};

pub(super) const MAX_REGISTERED_BYTES: usize = 256 * 1024;

pub(super) fn capture_registered(
    repository_root: &Path,
    relative: &str,
) -> Result<(TargetLock, CapturedFile), RefreshFailure> {
    let lock = publication::lock_target(repository_root, &repository_root.join(relative))
        .map_err(|error| publication_failure(error, relative))?;
    let capture = lock
        .capture(MAX_REGISTERED_BYTES)
        .map_err(|error| io_failure(error, relative))?;
    Ok((lock, capture))
}

pub(super) fn revalidate_registered_inputs(
    request: &CapturedFile,
    context: &CapturedFile,
) -> io::Result<()> {
    request.revalidate().and_then(|()| context.revalidate())
}
