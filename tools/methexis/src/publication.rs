//! Directory-handle-relative publication, locking, and path-safety policy.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, Read, Seek, Write},
    os::{
        fd::OwnedFd,
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use rustix::{
    fs::{
        AtFlags, Dir, FileType, FlockOperation, Mode, OFlags, RenameFlags, flock, ftruncate,
        linkat, mkdirat, open, openat, renameat, renameat_with, statat, unlinkat,
    },
    io::Errno,
};
use sha2::{Digest, Sha256};

use crate::file_identity::FileIdentity;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const DIRECTORY_MODE: Mode = Mode::from_raw_mode(0o755);
const FILE_MODE: Mode = Mode::from_raw_mode(0o644);
const OPEN_DIRECTORY: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

#[derive(Debug)]
pub(crate) enum PublicationError {
    OutsideRepository,
    Symlink(PathBuf),
    NotDirectory(PathBuf),
    Locked(io::Error),
    DurabilityUnknown(io::Error),
    Io(io::Error),
}

impl PublicationError {
    pub(crate) const fn namespace_may_be_committed(&self) -> bool {
        matches!(self, Self::DurabilityUnknown(_))
    }
}

pub(crate) enum GuardedDirectoryError<E> {
    Publication(PublicationError),
    Guard(E),
}

pub(crate) enum DirectoryState {
    Missing,
    Matches(VerifiedDirectory),
    Different,
}

pub(crate) struct VerifiedDirectory {
    _directory: OwnedFd,
    identity: FileIdentity,
    files: BTreeMap<OsString, FileIdentity>,
}

pub(crate) struct TargetLock {
    parent: OwnedFd,
    target_name: OsString,
    _lock_file: File,
}

pub(crate) struct RepositoryGuard {
    _root: File,
}

pub(crate) struct CapturedFile {
    parent: OwnedFd,
    target_name: OsString,
    _file: File,
    identity: FileIdentity,
    bytes: Vec<u8>,
    max_bytes: usize,
}

impl CapturedFile {
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn revalidate(&self) -> io::Result<()> {
        let (file, identity, bytes) = capture_relative(
            rustix::io::dup(&self.parent).map_err(errno)?,
            &self.target_name,
            self.max_bytes,
        )?;
        drop(file);
        if identity != self.identity || bytes != self.bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "captured file changed during operation",
            ));
        }
        Ok(())
    }
}

impl TargetLock {
    pub(crate) fn capture(&self, max_bytes: usize) -> io::Result<CapturedFile> {
        let parent = rustix::io::dup(&self.parent).map_err(errno)?;
        let (file, identity, bytes) = capture_relative(
            rustix::io::dup(&parent).map_err(errno)?,
            &self.target_name,
            max_bytes,
        )?;
        Ok(CapturedFile {
            parent,
            target_name: self.target_name.clone(),
            _file: file,
            identity,
            bytes,
            max_bytes,
        })
    }

    pub(crate) fn read(&self) -> io::Result<Vec<u8>> {
        let fd = openat(
            &self.parent,
            &self.target_name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(errno)?;
        let mut bytes = Vec::new();
        File::from(fd).read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    pub(crate) fn atomic_write(&self, bytes: &[u8]) -> Result<(), PublicationError> {
        let temporary = temporary_name(&self.target_name);
        if let Err(error) = write_new_file(&self.parent, &temporary, bytes) {
            let _ = unlinkat(&self.parent, &temporary, AtFlags::empty());
            return Err(PublicationError::Io(error));
        }
        if let Err(error) = renameat(&self.parent, &temporary, &self.parent, &self.target_name) {
            let _ = unlinkat(&self.parent, &temporary, AtFlags::empty());
            return Err(PublicationError::Io(errno(error)));
        }
        sync_directory(&self.parent).map_err(PublicationError::DurabilityUnknown)
    }

    pub(crate) fn atomic_replace_or_restore(
        &self,
        bytes: &[u8],
        previous: Option<&[u8]>,
    ) -> Result<(), PublicationError> {
        let error = match self.atomic_write(bytes) {
            Ok(()) => return Ok(()),
            Err(PublicationError::DurabilityUnknown(error)) => error,
            Err(error) => return Err(error),
        };
        let restored = match previous {
            Some(previous) => self.atomic_write(previous),
            None => self.remove(),
        };
        Err(classify_ambiguous_recovery(error, restored))
    }

    pub(crate) fn atomic_create(&self, bytes: &[u8]) -> Result<(), PublicationError> {
        let temporary = temporary_name(&self.target_name);
        if let Err(error) = write_new_file(&self.parent, &temporary, bytes) {
            let _ = unlinkat(&self.parent, &temporary, AtFlags::empty());
            return Err(PublicationError::Io(error));
        }
        if let Err(error) = linkat(
            &self.parent,
            &temporary,
            &self.parent,
            &self.target_name,
            AtFlags::empty(),
        ) {
            let _ = unlinkat(&self.parent, &temporary, AtFlags::empty());
            return Err(PublicationError::Io(errno(error)));
        }
        let committed = unlinkat(&self.parent, &temporary, AtFlags::empty())
            .map_err(errno)
            .and_then(|()| sync_directory(&self.parent));
        if let Err(error) = committed {
            return match rollback_created_file(&self.parent, &self.target_name, &temporary) {
                Ok(()) => Err(PublicationError::Io(error)),
                Err(_) => Err(PublicationError::DurabilityUnknown(error)),
            };
        }
        Ok(())
    }

    pub(crate) fn remove(&self) -> Result<(), PublicationError> {
        unlinkat(&self.parent, &self.target_name, AtFlags::empty())
            .map_err(errno)
            .map_err(PublicationError::Io)?;
        sync_directory(&self.parent).map_err(PublicationError::DurabilityUnknown)
    }

    pub(crate) fn directory_state(
        &self,
        files: &[(&str, &[u8])],
    ) -> Result<DirectoryState, PublicationError> {
        let directory = match openat(
            &self.parent,
            &self.target_name,
            OPEN_DIRECTORY,
            Mode::empty(),
        ) {
            Ok(directory) => directory,
            Err(Errno::NOENT) => return Ok(DirectoryState::Missing),
            Err(Errno::LOOP) => {
                return Err(PublicationError::Symlink(PathBuf::from(&self.target_name)));
            },
            Err(Errno::NOTDIR) => {
                return Err(PublicationError::NotDirectory(PathBuf::from(
                    &self.target_name,
                )));
            },
            Err(error) => return Err(PublicationError::Io(errno(error))),
        };
        let identity = FileIdentity::capture(&directory).map_err(PublicationError::Io)?;
        if directory_names(&directory)? != expected_names(files) {
            return Ok(DirectoryState::Different);
        }
        let mut identities = BTreeMap::new();
        for (name, expected) in files {
            let (file, file_identity, actual) = match capture_relative(
                rustix::io::dup(&directory).map_err(|error| PublicationError::Io(errno(error)))?,
                OsStr::new(name),
                expected.len(),
            ) {
                Ok(capture) => capture,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    return Ok(DirectoryState::Different);
                },
                Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                    return Ok(DirectoryState::Different);
                },
                Err(error) => return Err(PublicationError::Io(error)),
            };
            drop(file);
            if actual != *expected {
                return Ok(DirectoryState::Different);
            }
            identities.insert(OsString::from(name), file_identity);
        }
        if FileIdentity::capture(&directory).map_err(PublicationError::Io)? != identity {
            return Ok(DirectoryState::Different);
        }
        Ok(DirectoryState::Matches(VerifiedDirectory {
            _directory: directory,
            identity,
            files: identities,
        }))
    }

    pub(crate) fn revalidate_directory(
        &self,
        verified: &VerifiedDirectory,
        files: &[(&str, &[u8])],
    ) -> Result<bool, PublicationError> {
        let directory = match openat(
            &self.parent,
            &self.target_name,
            OPEN_DIRECTORY,
            Mode::empty(),
        ) {
            Ok(directory) => directory,
            Err(Errno::NOENT) => return Ok(false),
            Err(Errno::LOOP) => {
                return Err(PublicationError::Symlink(PathBuf::from(&self.target_name)));
            },
            Err(Errno::NOTDIR) => {
                return Err(PublicationError::NotDirectory(PathBuf::from(
                    &self.target_name,
                )));
            },
            Err(error) => return Err(PublicationError::Io(errno(error))),
        };
        if FileIdentity::capture(&directory).map_err(PublicationError::Io)? != verified.identity
            || directory_names(&directory)? != expected_names(files)
        {
            return Ok(false);
        }
        for (name, expected) in files {
            let (file, identity, actual) = match capture_relative(
                rustix::io::dup(&directory).map_err(|error| PublicationError::Io(errno(error)))?,
                OsStr::new(name),
                expected.len(),
            ) {
                Ok(capture) => capture,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::InvalidData
                    ) =>
                {
                    return Ok(false);
                },
                Err(error) => return Err(PublicationError::Io(error)),
            };
            drop(file);
            if actual != *expected
                || verified.files.get(OsStr::new(name)).copied() != Some(identity)
            {
                return Ok(false);
            }
        }
        Ok(FileIdentity::capture(&directory).map_err(PublicationError::Io)? == verified.identity)
    }

    pub(crate) fn atomic_create_directory(
        &self,
        files: &[(&str, &[u8])],
    ) -> Result<(), PublicationError> {
        match self.atomic_create_directory_guarded(files, || Ok::<(), std::convert::Infallible>(()))
        {
            Ok(()) => Ok(()),
            Err(GuardedDirectoryError::Publication(error)) => Err(error),
            Err(GuardedDirectoryError::Guard(never)) => match never {},
        }
    }

    pub(crate) fn atomic_create_directory_guarded<E>(
        &self,
        files: &[(&str, &[u8])],
        final_guard: impl FnOnce() -> Result<(), E>,
    ) -> Result<(), GuardedDirectoryError<E>> {
        let temporary = temporary_name(&self.target_name);
        mkdirat(&self.parent, &temporary, DIRECTORY_MODE).map_err(|error| {
            GuardedDirectoryError::Publication(PublicationError::Io(errno(error)))
        })?;
        let prepared = (|| -> io::Result<()> {
            let directory =
                openat(&self.parent, &temporary, OPEN_DIRECTORY, Mode::empty()).map_err(errno)?;
            for (name, bytes) in files {
                write_new_file(&directory, *name, bytes)?;
            }
            Ok(())
        })();
        if let Err(error) = prepared {
            cleanup_directory(&self.parent, &temporary, files);
            return Err(GuardedDirectoryError::Publication(PublicationError::Io(
                error,
            )));
        }
        if let Err(error) = final_guard() {
            cleanup_directory(&self.parent, &temporary, files);
            return Err(GuardedDirectoryError::Guard(error));
        }
        if let Err(error) = renameat_with(
            &self.parent,
            &temporary,
            &self.parent,
            &self.target_name,
            RenameFlags::NOREPLACE,
        ) {
            cleanup_directory(&self.parent, &temporary, files);
            return Err(GuardedDirectoryError::Publication(PublicationError::Io(
                errno(error),
            )));
        }
        if let Err(error) = sync_directory(&self.parent) {
            return match remove_directory(&self.parent, &self.target_name, files)
                .and_then(|()| sync_directory(&self.parent))
            {
                Ok(()) => Err(GuardedDirectoryError::Publication(PublicationError::Io(
                    error,
                ))),
                Err(_) => Err(GuardedDirectoryError::Publication(
                    PublicationError::DurabilityUnknown(error),
                )),
            };
        }
        Ok(())
    }
}

pub(crate) fn lock_target(
    repository_root: &Path,
    target: &Path,
) -> Result<TargetLock, PublicationError> {
    let (parent, target_name) = open_parent(repository_root, target)?;
    reject_symlink(&parent, &target_name, target)?;
    let relative = repository_relative(repository_root, target)?;
    let lock_target = repository_root
        .join(".local-exclude/methexis/locks")
        .join(format!("{}.lock", lock_identity(&relative)));
    let (lock_parent, lock_name) = open_parent(repository_root, &lock_target)?;
    let fd = openat(
        &lock_parent,
        &lock_name,
        OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        FILE_MODE,
    )
    .map_err(|error| PublicationError::Io(errno(error)))?;
    let mut file = File::from(fd);
    let metadata = file.metadata().map_err(PublicationError::Io)?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(PublicationError::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            "publication lock must be a singly linked regular file",
        )));
    }
    flock(&file, FlockOperation::NonBlockingLockExclusive)
        .map_err(|error| PublicationError::Locked(errno(error)))?;
    ftruncate(&file, 0).map_err(|error| PublicationError::Io(errno(error)))?;
    file.rewind().map_err(PublicationError::Io)?;
    writeln!(
        file,
        "pid={} target={}",
        std::process::id(),
        relative.display()
    )
    .map_err(PublicationError::Io)?;
    file.sync_all().map_err(PublicationError::Io)?;
    Ok(TargetLock {
        parent,
        target_name,
        _lock_file: file,
    })
}

pub(crate) fn lock_repository_shared(
    repository_root: &Path,
) -> Result<RepositoryGuard, PublicationError> {
    lock_repository(repository_root, FlockOperation::NonBlockingLockShared)
}

pub(crate) fn lock_repository_exclusive(
    repository_root: &Path,
) -> Result<RepositoryGuard, PublicationError> {
    lock_repository(repository_root, FlockOperation::NonBlockingLockExclusive)
}

fn lock_repository(
    repository_root: &Path,
    operation: FlockOperation,
) -> Result<RepositoryGuard, PublicationError> {
    let fd = open(repository_root, OPEN_DIRECTORY, Mode::empty())
        .map_err(|error| PublicationError::Io(errno(error)))?;
    let root = File::from(fd);
    flock(&root, operation).map_err(|error| PublicationError::Locked(errno(error)))?;
    Ok(RepositoryGuard { _root: root })
}

pub(crate) fn capture_file(
    repository_root: &Path,
    target: &Path,
    max_bytes: usize,
) -> Result<CapturedFile, PublicationError> {
    let (parent, target_name) = open_existing_parent(repository_root, target)?;
    reject_symlink(&parent, &target_name, target)?;
    let (file, identity, bytes) = capture_relative(
        rustix::io::dup(&parent).map_err(|error| PublicationError::Io(errno(error)))?,
        &target_name,
        max_bytes,
    )
    .map_err(PublicationError::Io)?;
    Ok(CapturedFile {
        parent,
        target_name,
        _file: file,
        identity,
        bytes,
        max_bytes,
    })
}

fn open_parent(
    repository_root: &Path,
    target: &Path,
) -> Result<(OwnedFd, OsString), PublicationError> {
    let relative = repository_relative(repository_root, target)?;
    let components = relative.components().collect::<Vec<_>>();
    if components.is_empty()
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(PublicationError::OutsideRepository);
    }
    let target_name = components
        .last()
        .and_then(|component| match component {
            Component::Normal(name) => Some(name.to_os_string()),
            _ => None,
        })
        .ok_or(PublicationError::OutsideRepository)?;
    let mut directory = open(repository_root, OPEN_DIRECTORY, Mode::empty())
        .map_err(|error| PublicationError::Io(errno(error)))?;
    let mut display = repository_root.to_owned();
    for component in components.iter().take(components.len() - 1) {
        let Component::Normal(name) = component else {
            return Err(PublicationError::OutsideRepository);
        };
        display.push(name);
        directory = open_or_create_directory(directory, name, &display)?;
    }
    Ok((directory, target_name))
}

fn open_existing_parent(
    repository_root: &Path,
    target: &Path,
) -> Result<(OwnedFd, OsString), PublicationError> {
    let relative = repository_relative(repository_root, target)?;
    let components = relative.components().collect::<Vec<_>>();
    if components.is_empty()
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(PublicationError::OutsideRepository);
    }
    let target_name = match components.last() {
        Some(Component::Normal(name)) => name.to_os_string(),
        _ => return Err(PublicationError::OutsideRepository),
    };
    let mut directory = open(repository_root, OPEN_DIRECTORY, Mode::empty())
        .map_err(|error| PublicationError::Io(errno(error)))?;
    let mut display = repository_root.to_owned();
    for component in components.iter().take(components.len() - 1) {
        let Component::Normal(name) = component else {
            return Err(PublicationError::OutsideRepository);
        };
        display.push(name);
        directory = openat(&directory, *name, OPEN_DIRECTORY, Mode::empty())
            .map_err(|error| component_error(error, &display))?;
    }
    Ok((directory, target_name))
}

fn repository_relative(repository_root: &Path, target: &Path) -> Result<PathBuf, PublicationError> {
    if let Ok(relative) = target.strip_prefix(repository_root) {
        return Ok(relative.to_owned());
    }
    if !target.is_absolute() {
        return Err(PublicationError::OutsideRepository);
    }
    let root = std::fs::metadata(repository_root).map_err(PublicationError::Io)?;
    for ancestor in target.ancestors().skip(1) {
        let Ok(candidate) = std::fs::metadata(ancestor) else {
            continue;
        };
        if candidate.dev() == root.dev() && candidate.ino() == root.ino() {
            return target
                .strip_prefix(ancestor)
                .map(Path::to_owned)
                .map_err(|_| PublicationError::OutsideRepository);
        }
    }
    Err(PublicationError::OutsideRepository)
}

fn open_or_create_directory(
    parent: OwnedFd,
    name: &OsStr,
    display: &Path,
) -> Result<OwnedFd, PublicationError> {
    match openat(&parent, name, OPEN_DIRECTORY, Mode::empty()) {
        Ok(directory) => Ok(directory),
        Err(Errno::NOENT) => {
            match mkdirat(&parent, name, DIRECTORY_MODE) {
                Ok(()) | Err(Errno::EXIST) => {},
                Err(error) => return Err(PublicationError::Io(errno(error))),
            }
            openat(&parent, name, OPEN_DIRECTORY, Mode::empty())
                .map_err(|error| component_error(error, display))
        },
        Err(error) => Err(component_error(error, display)),
    }
}

fn reject_symlink(parent: &OwnedFd, name: &OsStr, display: &Path) -> Result<(), PublicationError> {
    match statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) if FileType::from_raw_mode(stat.st_mode) == FileType::Symlink => {
            Err(PublicationError::Symlink(display.to_owned()))
        },
        Ok(_) | Err(Errno::NOENT) => Ok(()),
        Err(error) => Err(PublicationError::Io(errno(error))),
    }
}

fn component_error(error: Errno, display: &Path) -> PublicationError {
    match error {
        Errno::LOOP => PublicationError::Symlink(display.to_owned()),
        Errno::NOTDIR => PublicationError::NotDirectory(display.to_owned()),
        error => PublicationError::Io(errno(error)),
    }
}

fn write_new_file(parent: &OwnedFd, name: impl rustix::path::Arg, bytes: &[u8]) -> io::Result<()> {
    let fd = openat(
        parent,
        name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        FILE_MODE,
    )
    .map_err(errno)?;
    let mut file = File::from(fd);
    file.write_all(bytes)?;
    file.sync_all()
}

fn capture_relative(
    parent: OwnedFd,
    name: &OsStr,
    max_bytes: usize,
) -> io::Result<(File, FileIdentity, Vec<u8>)> {
    let fd = openat(
        &parent,
        name,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(errno)?;
    let mut file = File::from(fd);
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "captured path is not a regular file",
        ));
    }
    let before = FileIdentity::capture(&file)?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("captured file exceeds the {max_bytes}-byte limit"),
        ));
    }
    let after = FileIdentity::capture(&file)?;
    if before != after {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "captured file changed while being read",
        ));
    }
    Ok((file, before, bytes))
}

fn expected_names(files: &[(&str, &[u8])]) -> BTreeSet<OsString> {
    files.iter().map(|(name, _)| OsString::from(name)).collect()
}

fn directory_names(directory: &OwnedFd) -> Result<BTreeSet<OsString>, PublicationError> {
    let mut names = BTreeSet::new();
    for entry in Dir::read_from(directory).map_err(|error| PublicationError::Io(errno(error)))? {
        let entry = entry.map_err(|error| PublicationError::Io(errno(error)))?;
        let name = entry.file_name().to_bytes();
        if name != b"." && name != b".." {
            names.insert(OsString::from(OsStr::from_bytes(name)));
        }
    }
    Ok(names)
}

fn cleanup_directory(parent: &OwnedFd, temporary: &OsStr, files: &[(&str, &[u8])]) {
    if let Ok(directory) = openat(parent, temporary, OPEN_DIRECTORY, Mode::empty()) {
        for (name, _) in files {
            let _ = unlinkat(&directory, *name, AtFlags::empty());
        }
    }
    let _ = unlinkat(parent, temporary, AtFlags::REMOVEDIR);
}

fn rollback_created_file(parent: &OwnedFd, target: &OsStr, temporary: &OsStr) -> io::Result<()> {
    for name in [target, temporary] {
        match unlinkat(parent, name, AtFlags::empty()) {
            Ok(()) | Err(Errno::NOENT) => {},
            Err(error) => return Err(errno(error)),
        }
    }
    sync_directory(parent)
}

fn remove_directory(parent: &OwnedFd, name: &OsStr, files: &[(&str, &[u8])]) -> io::Result<()> {
    let directory = openat(parent, name, OPEN_DIRECTORY, Mode::empty()).map_err(errno)?;
    for (file, _) in files {
        unlinkat(&directory, *file, AtFlags::empty()).map_err(errno)?;
    }
    unlinkat(parent, name, AtFlags::REMOVEDIR).map_err(errno)
}

fn lock_identity(target: &Path) -> String {
    let digest = Sha256::digest(target.as_os_str().as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn temporary_name(target: &OsStr) -> OsString {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut name = OsString::from(".");
    name.push(target);
    name.push(format!(".tmp-{}-{sequence}", std::process::id()));
    name
}

fn errno(error: Errno) -> io::Error {
    io::Error::from_raw_os_error(error.raw_os_error())
}

fn classify_ambiguous_recovery(
    original: io::Error,
    restored: Result<(), PublicationError>,
) -> PublicationError {
    match restored {
        Ok(()) => PublicationError::Io(original),
        Err(_) => PublicationError::DurabilityUnknown(original),
    }
}

fn sync_directory(directory: &OwnedFd) -> io::Result<()> {
    File::from(rustix::io::dup(directory).map_err(errno)?).sync_all()
}

#[cfg(test)]
#[path = "publication/tests.rs"]
mod tests;
