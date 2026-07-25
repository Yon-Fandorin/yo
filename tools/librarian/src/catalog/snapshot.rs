//! Symlink-safe, stable working-tree byte capture.

use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::Read,
    os::{
        fd::{AsFd, OwnedFd},
        unix::ffi::OsStrExt,
    },
    path::Path,
};

use rustix::{
    fs::{Dir, FileType, Mode, OFlags, fstat, open, openat},
    io::Errno,
};

use crate::{error::DiscoveryError, hash::StableHasher};

const SNAPSHOT_DOMAIN: &[u8] = b"librarian.catalog-snapshot/v1alpha1";
const MAX_RECORD_BYTES: usize = 256 * 1024;
const MAX_CATALOG_BYTES: usize = 4 * 1024 * 1024;
const MAX_CATALOG_FILES: usize = 1024;
const MAX_CATALOG_ENTRIES: usize = 4096;
const MAX_DIRECTORY_DEPTH: usize = 64;
const OPEN_DIRECTORY: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
const OPEN_RECORD: OFlags = OFlags::RDONLY
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CapturedFile {
    pub(crate) path: String,
    pub(crate) bytes: Vec<u8>,
    identity: FileIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    device: u64,
    inode: u64,
    length: i64,
    modified_seconds: i64,
    modified_nanoseconds: u64,
}

#[derive(Default)]
struct CaptureBudget {
    bytes: usize,
    entries: usize,
    files: usize,
}

impl CaptureBudget {
    fn entry(&mut self, relative: &Path) -> Result<(), DiscoveryError> {
        self.entries += 1;
        if self.entries > MAX_CATALOG_ENTRIES {
            return Err(limit_error(
                relative,
                "catalog exceeds the 4096-entry Pilot limit",
            ));
        }
        Ok(())
    }

    fn record(&mut self, relative: &Path, bytes: usize) -> Result<(), DiscoveryError> {
        self.files += 1;
        self.bytes = self.bytes.saturating_add(bytes);
        if self.files > MAX_CATALOG_FILES || self.bytes > MAX_CATALOG_BYTES {
            return Err(limit_error(
                relative,
                "catalog exceeds the 1024-file or 4-MiB Pilot limit",
            ));
        }
        Ok(())
    }
}

impl FileIdentity {
    fn from(file: impl AsFd) -> Result<Self, Errno> {
        let stat = fstat(file)?;
        Ok(Self {
            device: stat.st_dev,
            inode: stat.st_ino,
            length: stat.st_size,
            modified_seconds: stat.st_mtime,
            modified_nanoseconds: stat.st_mtime_nsec,
        })
    }
}

pub(crate) struct CapturedCatalog {
    pub(crate) files: Vec<CapturedFile>,
    pub(crate) hash: String,
}

pub(crate) fn capture(repository_root: &Path) -> Result<CapturedCatalog, DiscoveryError> {
    capture_with_hook(repository_root, || {})
}

fn capture_with_hook(
    repository_root: &Path,
    after_first_pass: impl FnOnce(),
) -> Result<CapturedCatalog, DiscoveryError> {
    let files = capture_pass(repository_root)?;
    after_first_pass();
    let current = capture_pass(repository_root)?;
    if current != files {
        let changed = differing_paths(&files, &current);
        return Err(DiscoveryError::catalog_changed(changed));
    }

    let mut hasher = StableHasher::new(SNAPSHOT_DOMAIN);
    for file in &files {
        hasher.part(b"path", file.path.as_bytes());
        hasher.part(b"bytes", &file.bytes);
    }
    Ok(CapturedCatalog {
        files,
        hash: hasher.finish(),
    })
}

fn capture_pass(repository_root: &Path) -> Result<Vec<CapturedFile>, DiscoveryError> {
    let repository = open(repository_root, OPEN_DIRECTORY, Mode::empty()).map_err(|error| {
        open_error(
            error,
            "",
            "the repository root must be a readable non-symlink directory",
        )
    })?;
    let methexis = open_directory(&repository, OsStr::new("methexis"), "methexis")?;
    let knowledge = match openat(
        &methexis,
        OsStr::new("knowledge"),
        OPEN_DIRECTORY,
        Mode::empty(),
    ) {
        Ok(directory) => directory,
        Err(Errno::NOENT) => {
            return Err(DiscoveryError::catalog(
                "catalog_missing",
                "the working tree has no methexis/knowledge directory",
                Vec::new(),
                vec!["methexis/knowledge".to_owned()],
            ));
        },
        Err(error) => {
            return Err(open_error(
                error,
                "methexis/knowledge",
                "the Knowledge directory must not be a symlink",
            ));
        },
    };

    let mut files = Vec::new();
    let mut budget = CaptureBudget::default();
    collect_directory(
        &knowledge,
        Path::new("methexis/knowledge"),
        OsStr::new("md"),
        0,
        &mut budget,
        &mut files,
    )?;
    match openat(
        &methexis,
        OsStr::new("review-projections"),
        OPEN_DIRECTORY,
        Mode::empty(),
    ) {
        Ok(projections) => collect_directory(
            &projections,
            Path::new("methexis/review-projections"),
            OsStr::new("md"),
            0,
            &mut budget,
            &mut files,
        )?,
        Err(Errno::NOENT) => {},
        Err(error) => {
            return Err(open_error(
                error,
                "methexis/review-projections",
                "the review Projection directory must not be a symlink",
            ));
        },
    }
    for name in ["owners", "sources"] {
        let relative = format!("methexis/{name}");
        let directory = open_directory(&methexis, OsStr::new(name), &relative)?;
        collect_directory(
            &directory,
            Path::new(&relative),
            OsStr::new("yaml"),
            0,
            &mut budget,
            &mut files,
        )?;
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn collect_directory(
    directory: &OwnedFd,
    relative: &Path,
    extension: &OsStr,
    depth: usize,
    budget: &mut CaptureBudget,
    files: &mut Vec<CapturedFile>,
) -> Result<(), DiscoveryError> {
    if depth > MAX_DIRECTORY_DEPTH {
        return Err(limit_error(
            relative,
            "catalog exceeds the 64-directory-depth Pilot limit",
        ));
    }
    let directory_entries = Dir::read_from(directory)
        .map_err(|error| catalog_io(relative, format!("cannot enumerate directory: {error}")))?;
    let mut entries = Vec::new();
    for entry in directory_entries {
        let entry = entry.map_err(|error| {
            catalog_io(relative, format!("cannot enumerate directory: {error}"))
        })?;
        if entry.file_name().to_bytes() == b"." || entry.file_name().to_bytes() == b".." {
            continue;
        }
        budget.entry(relative)?;
        entries.push(entry);
    }
    entries.sort_by(|left, right| {
        left.file_name()
            .to_bytes()
            .cmp(right.file_name().to_bytes())
    });

    for entry in entries {
        let name = os_string(entry.file_name().to_bytes(), relative)?;
        let child_relative = relative.join(&name);
        match openat(directory, &name, OPEN_DIRECTORY, Mode::empty()) {
            Ok(child) => {
                collect_directory(&child, &child_relative, extension, depth + 1, budget, files)?
            },
            Err(Errno::NOTDIR) => {
                let fd = openat(directory, &name, OPEN_RECORD, Mode::empty()).map_err(|error| {
                    open_error(
                        error,
                        &display(&child_relative),
                        "catalog entries must not be symlinks",
                    )
                })?;
                let stat = fstat(&fd).map_err(|error| {
                    catalog_io(&child_relative, format!("cannot inspect record: {error}"))
                })?;
                if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
                    return Err(catalog_path_error(
                        "catalog_entry_forbidden",
                        "catalog entries must be regular files or directories",
                        &child_relative,
                    ));
                }
                if child_relative.extension() == Some(extension) {
                    let record = read_record(fd, &child_relative)?;
                    budget.record(&child_relative, record.bytes.len())?;
                    files.push(record);
                }
            },
            Err(error) => {
                return Err(open_error(
                    error,
                    &display(&child_relative),
                    "catalog entries must not be symlinks",
                ));
            },
        }
    }
    Ok(())
}

fn read_record(fd: OwnedFd, relative: &Path) -> Result<CapturedFile, DiscoveryError> {
    let before = FileIdentity::from(&fd)
        .map_err(|error| catalog_io(relative, format!("cannot inspect record: {error}")))?;
    let mut file = File::from(fd);
    let mut bytes = Vec::new();
    file.by_ref()
        .take((MAX_RECORD_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| catalog_io(relative, format!("cannot read record: {error}")))?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(catalog_path_error(
            "catalog_record_too_large",
            "catalog records must not exceed 256 KiB",
            relative,
        ));
    }
    let after = FileIdentity::from(&file)
        .map_err(|error| catalog_io(relative, format!("cannot inspect record: {error}")))?;
    if before != after {
        return Err(DiscoveryError::catalog_changed(vec![display(relative)]));
    }
    Ok(CapturedFile {
        path: display(relative),
        bytes,
        identity: after,
    })
}

fn open_directory(
    parent: impl AsFd,
    name: &OsStr,
    relative: &str,
) -> Result<OwnedFd, DiscoveryError> {
    openat(parent, name, OPEN_DIRECTORY, Mode::empty())
        .map_err(|error| open_error(error, relative, "catalog directories must not be symlinks"))
}

fn open_error(error: Errno, relative: &str, unsafe_message: &'static str) -> DiscoveryError {
    match error {
        Errno::NOENT => DiscoveryError::catalog(
            "catalog_missing",
            "a declared catalog path is missing",
            Vec::new(),
            vec![relative.to_owned()],
        ),
        Errno::LOOP | Errno::NOTDIR => DiscoveryError::catalog(
            "catalog_symlink_forbidden",
            unsafe_message,
            Vec::new(),
            vec![relative.to_owned()],
        ),
        error => DiscoveryError::catalog(
            "catalog_unreadable",
            format!("cannot open catalog path: {error}"),
            Vec::new(),
            vec![relative.to_owned()],
        ),
    }
}

fn catalog_io(relative: &Path, message: String) -> DiscoveryError {
    catalog_path_error("catalog_unreadable", message, relative)
}

fn catalog_path_error(
    code: &'static str,
    message: impl Into<String>,
    relative: &Path,
) -> DiscoveryError {
    DiscoveryError::catalog(code, message, Vec::new(), vec![display(relative)])
}

fn limit_error(relative: &Path, message: &'static str) -> DiscoveryError {
    catalog_path_error("catalog_limit_exceeded", message, relative)
}

fn os_string(bytes: &[u8], parent: &Path) -> Result<OsString, DiscoveryError> {
    if std::str::from_utf8(bytes).is_err() {
        return Err(catalog_path_error(
            "catalog_path_not_utf8",
            "catalog paths must be valid UTF-8",
            parent,
        ));
    }
    Ok(OsStr::from_bytes(bytes).to_owned())
}

fn display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn differing_paths(left: &[CapturedFile], right: &[CapturedFile]) -> Vec<String> {
    let left = left
        .iter()
        .map(|file| (&file.path, file))
        .collect::<std::collections::BTreeMap<_, _>>();
    let right = right
        .iter()
        .map(|file| (&file.path, file))
        .collect::<std::collections::BTreeMap<_, _>>();
    left.keys()
        .chain(right.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|path| left.get(*path) != right.get(*path))
        .map(|path| (*path).clone())
        .collect()
}

#[cfg(test)]
mod tests;
