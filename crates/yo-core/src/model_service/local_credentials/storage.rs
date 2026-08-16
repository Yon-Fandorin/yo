use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use super::{
    LocalCredentialStoreError,
    repository::{CredentialRevision, StoredCredentialSnapshot},
    wire,
};

pub(super) const MAX_CREDENTIAL_FILE_BYTES: u64 = 64 * 1024;
const FILE_MODE: u32 = 0o600;
const DIRECTORY_MODE: u32 = 0o700;
const REPOSITORY_LOCK_FILE: &str = ".credentials.lock";

#[cfg(target_vendor = "apple")]
const FILE_TYPE_MASK: u32 = libc::S_IFMT as u32;
#[cfg(not(target_vendor = "apple"))]
const FILE_TYPE_MASK: u32 = libc::S_IFMT;
#[cfg(target_vendor = "apple")]
const REGULAR_FILE_MODE: u32 = libc::S_IFREG as u32;
#[cfg(not(target_vendor = "apple"))]
const REGULAR_FILE_MODE: u32 = libc::S_IFREG;

pub(super) fn read_snapshot(
    path: &Path,
) -> Result<StoredCredentialSnapshot, LocalCredentialStoreError> {
    let mut file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return StoredCredentialSnapshot::new(CredentialRevision::absent(), Vec::new());
        },
        Err(source) => return Err(LocalCredentialStoreError::io(path, source)),
    };

    let before = MetadataSnapshot::capture(path, &file)?;
    before.validate(path)?;
    let mut bytes = Vec::with_capacity(
        usize::try_from(before.len.min(MAX_CREDENTIAL_FILE_BYTES))
            .unwrap_or(MAX_CREDENTIAL_FILE_BYTES as usize),
    );
    Read::by_ref(&mut file)
        .take(MAX_CREDENTIAL_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| LocalCredentialStoreError::io(path, source))?;
    if bytes.len() as u64 > MAX_CREDENTIAL_FILE_BYTES {
        return Err(LocalCredentialStoreError::TooLarge(path.to_owned()));
    }
    let after = MetadataSnapshot::capture(path, &file)?;
    validate_unchanged(path, &before, &after)?;

    let decoded = wire::decode(path, &bytes, before.derived_revision())?;
    StoredCredentialSnapshot::new(decoded.revision, decoded.entries)
}

pub(super) fn lock_repository(
    path: &Path,
) -> Result<(PathBuf, fs::File), LocalCredentialStoreError> {
    let parent = prepare_parent(path)?;
    let lock_path = parent.join(REPOSITORY_LOCK_FILE);
    reject_symlink(&lock_path)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(FILE_MODE)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&lock_path)
        .map_err(|source| LocalCredentialStoreError::io(&lock_path, source))?;
    MetadataSnapshot::capture(&lock_path, &file)?.validate(&lock_path)?;
    file.lock()
        .map_err(|source| LocalCredentialStoreError::io(&lock_path, source))?;
    Ok((parent, file))
}

pub(super) fn publish(
    path: &Path,
    parent: &Path,
    expected_absent: bool,
    encoded: &[u8],
) -> Result<(), LocalCredentialStoreError> {
    if encoded.len() as u64 > MAX_CREDENTIAL_FILE_BYTES {
        return Err(LocalCredentialStoreError::PreparedTooLarge);
    }
    let (temporary, mut file) = create_temporary(parent)?;
    let publication = (|| {
        file.write_all(encoded)
            .map_err(|source| LocalCredentialStoreError::io(&temporary, source))?;
        file.sync_all()
            .map_err(|source| LocalCredentialStoreError::io(&temporary, source))?;
        if expected_absent {
            match fs::hard_link(&temporary, path) {
                Ok(()) => {},
                Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Err(LocalCredentialStoreError::Conflict(path.to_owned()));
                },
                Err(source) => return Err(LocalCredentialStoreError::io(path, source)),
            }
            fs::remove_file(&temporary)
                .map_err(|source| LocalCredentialStoreError::io(&temporary, source))?;
        } else {
            fs::rename(&temporary, path)
                .map_err(|source| LocalCredentialStoreError::io(path, source))?;
        }
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| LocalCredentialStoreError::io(parent, source))?;
        Ok(())
    })();
    if publication.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    publication
}

fn create_temporary(parent: &Path) -> Result<(PathBuf, fs::File), LocalCredentialStoreError> {
    for _ in 0..16 {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|error| LocalCredentialStoreError::Randomness(error.to_string()))?;
        let mut suffix = String::with_capacity(32);
        for byte in random {
            use std::fmt::Write as _;
            write!(suffix, "{byte:02x}").expect("formatting into a String cannot fail");
        }
        let temporary = parent.join(format!(".credentials.{suffix}.pending"));
        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(FILE_MODE)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {},
            Err(source) => return Err(LocalCredentialStoreError::io(&temporary, source)),
        }
    }
    Err(LocalCredentialStoreError::InvalidMutation)
}

fn prepare_parent(path: &Path) -> Result<PathBuf, LocalCredentialStoreError> {
    let parent = path
        .parent()
        .ok_or_else(|| LocalCredentialStoreError::InvalidPath(path.to_owned()))?;
    if let Ok(metadata) = fs::symlink_metadata(parent)
        && metadata.file_type().is_symlink()
    {
        return Err(LocalCredentialStoreError::UnsupportedFileType(
            parent.to_owned(),
        ));
    }
    let existed = parent.exists();
    fs::create_dir_all(parent).map_err(|source| LocalCredentialStoreError::io(parent, source))?;
    if !existed {
        fs::set_permissions(parent, fs::Permissions::from_mode(DIRECTORY_MODE))
            .map_err(|source| LocalCredentialStoreError::io(parent, source))?;
    }
    Ok(parent.to_owned())
}

fn reject_symlink(path: &Path) -> Result<(), LocalCredentialStoreError> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && metadata.file_type().is_symlink()
    {
        return Err(LocalCredentialStoreError::UnsupportedFileType(
            path.to_owned(),
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MetadataSnapshot {
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
    fn capture(path: &Path, file: &fs::File) -> Result<Self, LocalCredentialStoreError> {
        let metadata = file
            .metadata()
            .map_err(|source| LocalCredentialStoreError::io(path, source))?;
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

    fn validate(&self, path: &Path) -> Result<(), LocalCredentialStoreError> {
        if self.mode & FILE_TYPE_MASK != REGULAR_FILE_MODE {
            return Err(LocalCredentialStoreError::UnsupportedFileType(
                path.to_owned(),
            ));
        }
        if self.user != rustix::process::geteuid().as_raw() {
            return Err(LocalCredentialStoreError::WrongOwner(path.to_owned()));
        }
        if self.mode & 0o077 != 0 {
            return Err(LocalCredentialStoreError::InsecurePermissions(
                path.to_owned(),
            ));
        }
        if self.len > MAX_CREDENTIAL_FILE_BYTES {
            return Err(LocalCredentialStoreError::TooLarge(path.to_owned()));
        }
        Ok(())
    }

    fn derived_revision(&self) -> CredentialRevision {
        CredentialRevision::derived(format!(
            "derived-{:016x}{:016x}{:016x}{:016x}{:016x}{:016x}{:016x}",
            self.device,
            self.inode,
            self.len,
            self.modified_seconds as u64,
            self.modified_nanoseconds as u64,
            self.changed_seconds as u64,
            self.changed_nanoseconds as u64,
        ))
    }
}

fn validate_unchanged(
    path: &Path,
    before: &MetadataSnapshot,
    after: &MetadataSnapshot,
) -> Result<(), LocalCredentialStoreError> {
    if before != after {
        return Err(LocalCredentialStoreError::Changed(path.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn secure_snapshot() -> MetadataSnapshot {
    MetadataSnapshot {
        device: 1,
        inode: 2,
        mode: REGULAR_FILE_MODE | 0o600,
        user: rustix::process::geteuid().as_raw(),
        group: 3,
        len: 100,
        modified_seconds: 4,
        modified_nanoseconds: 5,
        changed_seconds: 6,
        changed_nanoseconds: 7,
    }
}

#[cfg(test)]
pub(super) fn change_owner(snapshot: &mut MetadataSnapshot) {
    snapshot.user = snapshot.user.wrapping_add(1);
}

#[cfg(test)]
pub(super) fn change_length(snapshot: &mut MetadataSnapshot) {
    snapshot.len = snapshot.len.saturating_add(1);
}

#[cfg(test)]
pub(super) fn validate_snapshot(
    path: &Path,
    snapshot: &MetadataSnapshot,
) -> Result<(), LocalCredentialStoreError> {
    snapshot.validate(path)
}

#[cfg(test)]
pub(super) fn validate_stable_snapshots(
    path: &Path,
    before: &MetadataSnapshot,
    after: &MetadataSnapshot,
) -> Result<(), LocalCredentialStoreError> {
    validate_unchanged(path, before, after)
}
