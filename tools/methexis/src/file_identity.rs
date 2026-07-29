//! Platform-neutral identity for detecting changes to an open file.

use std::{io, os::fd::AsFd};

use rustix::fs::fstat;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FileIdentity {
    // rustix exposes the platform ABI here: Darwin uses signed integers while
    // Linux uses unsigned ones. Widening preserves either value losslessly.
    device: i128,
    inode: u64,
    size: i64,
    modified_seconds: i64,
    modified_nanoseconds: i128,
}

impl FileIdentity {
    pub(crate) fn capture(file: impl AsFd) -> io::Result<Self> {
        let stat = fstat(file).map_err(io::Error::from)?;
        Ok(Self {
            device: stat.st_dev.into(),
            inode: stat.st_ino,
            size: stat.st_size,
            modified_seconds: stat.st_mtime,
            modified_nanoseconds: stat.st_mtime_nsec.into(),
        })
    }
}
