use std::{
    fmt, fs, io,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use rustix::process;

use super::{DIRECTORY_MODE, FILE_MODE, LOCK_FILE, MAX_CACHE_BYTES, StorageError};

#[cfg(target_vendor = "apple")]
const FILE_TYPE_MASK: u32 = libc::S_IFMT as u32;
#[cfg(not(target_vendor = "apple"))]
const FILE_TYPE_MASK: u32 = libc::S_IFMT;
#[cfg(target_vendor = "apple")]
const REGULAR_FILE_MODE: u32 = libc::S_IFREG as u32;
#[cfg(not(target_vendor = "apple"))]
const REGULAR_FILE_MODE: u32 = libc::S_IFREG;

pub(super) fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>, StorageError> {
    let mut file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(StorageError::io(path, source)),
    };
    let before = MetadataSnapshot::capture(path, &file)?;
    before.validate(path)?;
    let mut encoded = Vec::with_capacity(
        usize::try_from(before.len.min(MAX_CACHE_BYTES)).unwrap_or(MAX_CACHE_BYTES as usize),
    );
    Read::by_ref(&mut file)
        .take(MAX_CACHE_BYTES + 1)
        .read_to_end(&mut encoded)
        .map_err(|source| StorageError::io(path, source))?;
    if encoded.len() as u64 > MAX_CACHE_BYTES {
        return Err(StorageError::TooLarge(path.to_owned()));
    }
    let after = MetadataSnapshot::capture(path, &file)?;
    if before != after {
        return Err(StorageError::Changed(path.to_owned()));
    }
    Ok(Some(encoded))
}

pub(super) fn lock_repository(path: &Path) -> Result<(PathBuf, fs::File), StorageError> {
    let parent = prepare_parent(path)?;
    let lock_path = parent.join(LOCK_FILE);
    reject_symlink(&lock_path)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(FILE_MODE)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&lock_path)
        .map_err(|source| StorageError::io(&lock_path, source))?;
    MetadataSnapshot::capture(&lock_path, &file)?.validate(&lock_path)?;
    file.lock()
        .map_err(|source| StorageError::io(&lock_path, source))?;
    Ok((parent, file))
}

pub(super) fn publish(path: &Path, parent: &Path, encoded: &[u8]) -> Result<(), StorageError> {
    reject_symlink(path)?;
    let (temporary, mut file) = create_temporary(parent)?;
    let publication = (|| {
        file.write_all(encoded)
            .map_err(|source| StorageError::io(&temporary, source))?;
        file.sync_all()
            .map_err(|source| StorageError::io(&temporary, source))?;
        fs::rename(&temporary, path).map_err(|source| StorageError::io(path, source))?;
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| StorageError::io(parent, source))?;
        Ok(())
    })();
    if publication.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    publication
}

fn prepare_parent(path: &Path) -> Result<PathBuf, StorageError> {
    let parent = path
        .parent()
        .ok_or_else(|| StorageError::InvalidPath(path.to_owned()))?;
    if let Ok(metadata) = fs::symlink_metadata(parent)
        && metadata.file_type().is_symlink()
    {
        return Err(StorageError::UnsupportedFileType(parent.to_owned()));
    }
    let existed = parent.exists();
    fs::create_dir_all(parent).map_err(|source| StorageError::io(parent, source))?;
    if !existed {
        fs::set_permissions(parent, fs::Permissions::from_mode(DIRECTORY_MODE))
            .map_err(|source| StorageError::io(parent, source))?;
    }
    validate_parent(parent)?;
    Ok(parent.to_owned())
}

fn validate_parent(parent: &Path) -> Result<(), StorageError> {
    let metadata =
        fs::symlink_metadata(parent).map_err(|source| StorageError::io(parent, source))?;
    if !metadata.file_type().is_dir() {
        return Err(StorageError::UnsupportedFileType(parent.to_owned()));
    }
    let shared_sticky_directory =
        metadata.uid() != process::geteuid().as_raw() && metadata.mode() & 0o1000 != 0;
    if metadata.uid() != process::geteuid().as_raw() && !shared_sticky_directory {
        return Err(StorageError::WrongOwner(parent.to_owned()));
    }
    if metadata.mode() & 0o022 != 0 && !shared_sticky_directory {
        return Err(StorageError::InsecurePermissions(parent.to_owned()));
    }
    Ok(())
}

pub(super) fn reject_symlink(path: &Path) -> Result<(), StorageError> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && metadata.file_type().is_symlink()
    {
        return Err(StorageError::UnsupportedFileType(path.to_owned()));
    }
    Ok(())
}

fn create_temporary(parent: &Path) -> Result<(PathBuf, fs::File), StorageError> {
    for _ in 0..16 {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|error| StorageError::Randomness(error.to_string()))?;
        let mut suffix = String::with_capacity(32);
        for byte in random {
            use fmt::Write as _;
            write!(suffix, "{byte:02x}").expect("formatting into a String cannot fail");
        }
        let temporary = parent.join(format!(".account-capacity.{suffix}.pending"));
        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(FILE_MODE)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {},
            Err(source) => return Err(StorageError::io(&temporary, source)),
        }
    }
    Err(StorageError::InvalidContents(PathBuf::new()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MetadataSnapshot {
    device: u64,
    inode: u64,
    mode: u32,
    user: u32,
    group: u32,
    len: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

impl MetadataSnapshot {
    fn capture(path: &Path, file: &fs::File) -> Result<Self, StorageError> {
        let metadata = file
            .metadata()
            .map_err(|source| StorageError::io(path, source))?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            user: metadata.uid(),
            group: metadata.gid(),
            len: metadata.len(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        })
    }

    fn validate(&self, path: &Path) -> Result<(), StorageError> {
        if self.mode & FILE_TYPE_MASK != REGULAR_FILE_MODE {
            return Err(StorageError::UnsupportedFileType(path.to_owned()));
        }
        if self.user != process::geteuid().as_raw() {
            return Err(StorageError::WrongOwner(path.to_owned()));
        }
        if self.mode & 0o077 != 0 {
            return Err(StorageError::InsecurePermissions(path.to_owned()));
        }
        if self.len > MAX_CACHE_BYTES {
            return Err(StorageError::TooLarge(path.to_owned()));
        }
        Ok(())
    }
}
