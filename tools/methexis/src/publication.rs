//! Directory-handle-relative publication, locking, and path-safety policy.

use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, Read, Write},
    os::fd::OwnedFd,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use rustix::{
    fs::{
        AtFlags, FileType, Mode, OFlags, RenameFlags, linkat, mkdirat, open, openat, renameat,
        renameat_with, statat, unlinkat,
    },
    io::Errno,
};

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
    Io(io::Error),
}

pub(crate) enum DirectoryState {
    Missing,
    Matches,
    Different,
}

pub(crate) struct TargetLock {
    parent: OwnedFd,
    target_name: OsString,
    lock_name: OsString,
}

impl Drop for TargetLock {
    fn drop(&mut self) {
        let _ = unlinkat(&self.parent, &self.lock_name, AtFlags::empty());
    }
}

impl TargetLock {
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
        if let Err(error) = write_new_file(&self.parent, &temporary, bytes).and_then(|()| {
            renameat(&self.parent, &temporary, &self.parent, &self.target_name).map_err(errno)
        }) {
            let _ = unlinkat(&self.parent, &temporary, AtFlags::empty());
            return Err(PublicationError::Io(error));
        }
        Ok(())
    }

    pub(crate) fn atomic_create(&self, bytes: &[u8]) -> Result<(), PublicationError> {
        let temporary = temporary_name(&self.target_name);
        if let Err(error) = write_new_file(&self.parent, &temporary, bytes).and_then(|()| {
            linkat(
                &self.parent,
                &temporary,
                &self.parent,
                &self.target_name,
                AtFlags::empty(),
            )
            .map_err(errno)?;
            unlinkat(&self.parent, &temporary, AtFlags::empty()).map_err(errno)
        }) {
            let _ = unlinkat(&self.parent, &temporary, AtFlags::empty());
            return Err(PublicationError::Io(error));
        }
        Ok(())
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
        for (name, expected) in files {
            let actual = match read_relative(&directory, name) {
                Ok(actual) => actual,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    return Ok(DirectoryState::Different);
                },
                Err(error) => return Err(PublicationError::Io(error)),
            };
            if actual != *expected {
                return Ok(DirectoryState::Different);
            }
        }
        Ok(DirectoryState::Matches)
    }

    pub(crate) fn atomic_create_directory(
        &self,
        files: &[(&str, &[u8])],
    ) -> Result<(), PublicationError> {
        let temporary = temporary_name(&self.target_name);
        mkdirat(&self.parent, &temporary, DIRECTORY_MODE)
            .map_err(|error| PublicationError::Io(errno(error)))?;
        let result = (|| -> io::Result<()> {
            let directory =
                openat(&self.parent, &temporary, OPEN_DIRECTORY, Mode::empty()).map_err(errno)?;
            for (name, bytes) in files {
                write_new_file(&directory, *name, bytes)?;
            }
            renameat_with(
                &self.parent,
                &temporary,
                &self.parent,
                &self.target_name,
                RenameFlags::NOREPLACE,
            )
            .map_err(errno)
        })();
        if let Err(error) = result {
            cleanup_directory(&self.parent, &temporary, files);
            return Err(PublicationError::Io(error));
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
    let lock_name = lock_name(&target_name);
    let fd = openat(
        &parent,
        &lock_name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        FILE_MODE,
    )
    .map_err(|error| PublicationError::Locked(errno(error)))?;
    let mut file = File::from(fd);
    if let Err(error) = writeln!(file, "pid={}", std::process::id()) {
        let _ = unlinkat(&parent, &lock_name, AtFlags::empty());
        return Err(PublicationError::Io(error));
    }
    Ok(TargetLock {
        parent,
        target_name,
        lock_name,
    })
}

fn open_parent(
    repository_root: &Path,
    target: &Path,
) -> Result<(OwnedFd, OsString), PublicationError> {
    let relative = target
        .strip_prefix(repository_root)
        .map_err(|_| PublicationError::OutsideRepository)?;
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

fn read_relative(parent: &OwnedFd, name: &str) -> io::Result<Vec<u8>> {
    let fd = openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(errno)?;
    let mut bytes = Vec::new();
    File::from(fd).read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn cleanup_directory(parent: &OwnedFd, temporary: &OsStr, files: &[(&str, &[u8])]) {
    if let Ok(directory) = openat(parent, temporary, OPEN_DIRECTORY, Mode::empty()) {
        for (name, _) in files {
            let _ = unlinkat(&directory, *name, AtFlags::empty());
        }
    }
    let _ = unlinkat(parent, temporary, AtFlags::REMOVEDIR);
}

fn lock_name(target: &OsStr) -> OsString {
    let mut name = OsString::from(".");
    name.push(target);
    name.push(".lock");
    name
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

#[cfg(test)]
#[path = "publication/tests.rs"]
mod tests;
