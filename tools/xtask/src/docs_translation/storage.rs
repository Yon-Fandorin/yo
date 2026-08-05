use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{Read, Write},
    os::fd::OwnedFd,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use rustix::{
    fs::{
        AtFlags, FileType, FlockOperation, Mode, OFlags, fchmod, flock, fstat, open, openat,
        renameat, unlinkat,
    },
    io::Errno,
};

const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
const READ_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::NONBLOCK)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(1);

pub(super) struct RepositoryFiles {
    root: OwnedFd,
    _lock: File,
}

pub(super) struct CapturedFile {
    parent: OwnedFd,
    name: OsString,
    display: PathBuf,
    identity: FileIdentity,
    bytes: Vec<u8>,
    max_bytes: usize,
    mode: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    device: i128,
    inode: i128,
    size: i128,
    modified_seconds: i128,
    modified_nanoseconds: i128,
}

impl RepositoryFiles {
    pub(super) fn open(repository: &Path) -> Result<Self, String> {
        let root = open(repository, DIRECTORY_FLAGS, Mode::empty())
            .map_err(|error| format!("cannot open repository root: {}", io_error(error)))?;
        let lock = File::from(
            rustix::io::dup(&root)
                .map_err(|error| format!("cannot retain repository root: {}", io_error(error)))?,
        );
        flock(&lock, FlockOperation::NonBlockingLockExclusive).map_err(|error| {
            format!(
                "another cooperating repository mutation is active: {}",
                io_error(error)
            )
        })?;
        Ok(Self { root, _lock: lock })
    }

    pub(super) fn capture(
        &self,
        relative: &Path,
        max_bytes: usize,
    ) -> Result<CapturedFile, String> {
        let (parent, name) = open_parent(&self.root, relative)?;
        let (file, identity, bytes, mode) = capture_relative(&parent, &name, max_bytes)
            .map_err(|error| format!("{}: {error}", relative.display()))?;
        drop(file);
        Ok(CapturedFile {
            parent,
            name,
            display: relative.to_owned(),
            identity,
            bytes,
            max_bytes,
            mode,
        })
    }
}

impl CapturedFile {
    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(super) fn revalidate(&self) -> Result<(), String> {
        let (file, identity, bytes, _) = capture_relative(&self.parent, &self.name, self.max_bytes)
            .map_err(|error| format!("{}: {error}", self.display.display()))?;
        drop(file);
        if identity != self.identity || bytes != self.bytes {
            return Err(format!(
                "{} changed before publication; retry",
                self.display.display()
            ));
        }
        Ok(())
    }

    pub(super) fn atomic_replace_guarded(
        &self,
        updated: &[u8],
        final_guard: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let temporary = prepare_new_file(&self.parent, &self.name, updated, self.mode)?;
        if let Err(error) = final_guard() {
            return Err(cleanup_error(&self.parent, &temporary, error));
        }
        if let Err(error) = self.revalidate() {
            return Err(cleanup_error(&self.parent, &temporary, error));
        }
        if let Err(error) = renameat(&self.parent, &temporary, &self.parent, &self.name) {
            return Err(cleanup_error(
                &self.parent,
                &temporary,
                format!("cannot replace docs/ko/source.sha256: {}", io_error(error)),
            ));
        }
        sync_directory(&self.parent).map_err(|error| {
            format!(
                "docs/ko/source.sha256 was replaced but directory durability is unknown: {error}"
            )
        })
    }
}

fn open_parent(root: &OwnedFd, relative: &Path) -> Result<(OwnedFd, OsString), String> {
    let components = relative.components().collect::<Vec<_>>();
    if components.is_empty()
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "{} is not a safe repository path",
            relative.display()
        ));
    }
    let name = match components.last() {
        Some(Component::Normal(name)) => name.to_os_string(),
        _ => {
            return Err(format!(
                "{} is not a safe repository path",
                relative.display()
            ));
        },
    };
    let mut parent = rustix::io::dup(root)
        .map_err(|error| format!("cannot retain repository root: {}", io_error(error)))?;
    for component in components.iter().take(components.len() - 1) {
        let Component::Normal(component) = component else {
            unreachable!("components were validated above");
        };
        parent = openat(&parent, *component, DIRECTORY_FLAGS, Mode::empty())
            .map_err(|error| path_error(relative, error))?;
    }
    Ok((parent, name))
}

fn capture_relative(
    parent: &OwnedFd,
    name: &OsStr,
    max_bytes: usize,
) -> Result<(File, FileIdentity, Vec<u8>, u32), String> {
    let fd = openat(parent, name, READ_FLAGS, Mode::empty())
        .map_err(|error| path_error(Path::new(name), error))?;
    let stat = fstat(&fd).map_err(|error| format!("cannot inspect file: {}", io_error(error)))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err("Developer Docs inputs must be regular files".to_owned());
    }
    let before = FileIdentity::from_stat(&stat);
    let mut file = File::from(fd);
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read Developer Docs input: {error}"))?;
    if bytes.len() > max_bytes {
        return Err(format!(
            "Developer Docs input exceeds the {max_bytes} byte limit"
        ));
    }
    let after =
        fstat(&file).map_err(|error| format!("cannot re-inspect file: {}", io_error(error)))?;
    if FileIdentity::from_stat(&after) != before {
        return Err("Developer Docs input changed while it was being read; retry".to_owned());
    }
    Ok((file, before, bytes, stat.st_mode as u32))
}

impl FileIdentity {
    fn from_stat(stat: &rustix::fs::Stat) -> Self {
        Self {
            device: i128::from(stat.st_dev),
            inode: i128::from(stat.st_ino),
            size: i128::from(stat.st_size),
            modified_seconds: i128::from(stat.st_mtime),
            modified_nanoseconds: i128::from(stat.st_mtime_nsec),
        }
    }
}

fn prepare_new_file(
    parent: &OwnedFd,
    target: &OsStr,
    bytes: &[u8],
    source_mode: u32,
) -> Result<OsString, String> {
    for _ in 0..32 {
        let name = temporary_name(target);
        let fd = match openat(
            parent,
            &name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode((source_mode & 0o7777) as _),
        ) {
            Ok(fd) => fd,
            Err(Errno::EXIST) => continue,
            Err(error) => {
                return Err(format!(
                    "cannot create temporary manifest: {}",
                    io_error(error)
                ));
            },
        };
        let mut file = File::from(fd);
        let prepared = (|| -> Result<(), String> {
            fchmod(&file, Mode::from_raw_mode((source_mode & 0o7777) as _)).map_err(|error| {
                format!("cannot preserve manifest permissions: {}", io_error(error))
            })?;
            file.write_all(bytes)
                .map_err(|error| format!("cannot write temporary manifest: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("cannot sync temporary manifest: {error}"))
        })();
        if let Err(error) = prepared {
            drop(file);
            return Err(cleanup_error(parent, &name, error));
        }
        return Ok(name);
    }
    Err("cannot allocate a temporary manifest after 32 attempts".to_owned())
}

fn cleanup_error(parent: &OwnedFd, temporary: &OsStr, primary: String) -> String {
    match unlinkat(parent, temporary, AtFlags::empty()) {
        Ok(()) | Err(Errno::NOENT) => primary,
        Err(error) => format!(
            "{primary}; temporary manifest cleanup also failed: {}",
            io_error(error)
        ),
    }
}

fn temporary_name(target: &OsStr) -> OsString {
    let sequence = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
    let mut name = OsString::from(".");
    name.push(target);
    name.push(format!(".tmp-{}-{sequence}", std::process::id()));
    name
}

fn sync_directory(directory: &OwnedFd) -> Result<(), String> {
    File::from(
        rustix::io::dup(directory)
            .map_err(|error| format!("cannot retain manifest directory: {}", io_error(error)))?,
    )
    .sync_all()
    .map_err(|error| format!("cannot sync manifest directory: {error}"))
}

fn path_error(path: &Path, error: Errno) -> String {
    match error {
        Errno::LOOP => format!("{} must not contain symbolic links", path.display()),
        _ => format!("cannot open {}: {}", path.display(), io_error(error)),
    }
}

fn io_error(error: Errno) -> std::io::Error {
    std::io::Error::from_raw_os_error(error.raw_os_error())
}
