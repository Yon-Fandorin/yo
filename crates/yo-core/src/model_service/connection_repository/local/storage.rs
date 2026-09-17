use std::{
    fmt, fs,
    io::{ErrorKind, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use super::{
    super::{
        MAX_CONNECTION_BYTES,
        error::ConnectionRepositoryError,
        model::{ConnectionRevision, ConnectionSnapshot},
        mutation::{ConnectionCommit, PreparedConnectionMutation},
        wire,
    },
    security::{FILE_MODE, MetadataSnapshot, REPOSITORY_LOCK_FILE, open_lock_file, prepare_parent},
};

// 한 번 재시도해 candidate 점유와 지속적으로 비정상인 name source를 구분합니다.
const CONNECTION_TEMPORARY_ATTEMPTS: usize = 2;

impl super::LocalConnectionRepository {
    /// file이나 parent directory를 만들지 않고 캡처합니다.
    pub fn capture(&self) -> Result<ConnectionSnapshot, ConnectionRepositoryError> {
        read_snapshot(&self.path)
    }

    /// expected revision이 여전히 path를 소유하면 준비된 정확한 bytes를 게시합니다.
    pub fn commit(
        &self,
        mutation: &PreparedConnectionMutation,
    ) -> Result<ConnectionCommit, ConnectionRepositoryError> {
        let parent = prepare_parent(&self.path)?;
        let lock_path = parent.join(REPOSITORY_LOCK_FILE);
        let lock = open_lock_file(&lock_path)?;
        lock.lock()
            .map_err(|source| ConnectionRepositoryError::io(&lock_path, source))?;

        let current = read_snapshot(&self.path)?;
        if current.revision == mutation.planned_revision
            && current.encoded == mutation.planned_bytes
        {
            return Ok(ConnectionCommit::AlreadyCommitted);
        }
        if current.revision != mutation.expected_revision {
            return Err(ConnectionRepositoryError::Conflict {
                expected: mutation.expected_revision.clone(),
                observed: current.revision,
            });
        }

        let (temporary, mut file) = create_connection_temporary(&parent)?;
        let publication = (|| {
            file.write_all(&mutation.planned_bytes)
                .map_err(|source| ConnectionRepositoryError::io(&temporary, source))?;
            file.sync_all()
                .map_err(|source| ConnectionRepositoryError::io(&temporary, source))?;
            if mutation.expected_revision.is_absent() {
                fs::hard_link(&temporary, &self.path)
                    .map_err(|source| ConnectionRepositoryError::io(&self.path, source))?;
                fs::remove_file(&temporary)
                    .map_err(|source| ConnectionRepositoryError::io(&temporary, source))?;
            } else {
                fs::rename(&temporary, &self.path)
                    .map_err(|source| ConnectionRepositoryError::io(&self.path, source))?;
            }
            fs::File::open(&parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|source| ConnectionRepositoryError::io(&parent, source))?;
            Ok(())
        })();
        if publication.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        publication?;
        Ok(ConnectionCommit::Committed)
    }
}

fn create_connection_temporary(
    parent: &Path,
) -> Result<(PathBuf, fs::File), ConnectionRepositoryError> {
    create_connection_temporary_with(parent, CONNECTION_TEMPORARY_ATTEMPTS, || {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|error| error.to_string())?;
        Ok(random)
    })
}

fn create_connection_temporary_with(
    parent: &Path,
    attempt_limit: usize,
    mut next_candidate: impl FnMut() -> Result<[u8; 16], String>,
) -> Result<(PathBuf, fs::File), ConnectionRepositoryError> {
    for _ in 0..attempt_limit {
        let random =
            next_candidate().map_err(ConnectionRepositoryError::TemporaryNameRandomness)?;
        let temporary = connection_temporary_path(parent, random);
        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(FILE_MODE)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(source) if source.kind() == ErrorKind::AlreadyExists => {},
            Err(source) => return Err(ConnectionRepositoryError::io(&temporary, source)),
        }
    }
    Err(
        ConnectionRepositoryError::TemporaryNameCollisionExhaustion {
            attempts: attempt_limit,
        },
    )
}

fn connection_temporary_path(parent: &Path, random: [u8; 16]) -> PathBuf {
    let mut suffix = String::with_capacity(32);
    for byte in random {
        use fmt::Write as _;
        write!(suffix, "{byte:02x}").expect("formatting into a String cannot fail");
    }
    parent.join(format!(".connections.{suffix}.pending"))
}

#[cfg(test)]
pub(in super::super::super) fn create_connection_temporary_for_test(
    parent: &Path,
    attempt_limit: usize,
    next_candidate: impl FnMut() -> Result<[u8; 16], String>,
) -> Result<(PathBuf, fs::File), ConnectionRepositoryError> {
    create_connection_temporary_with(parent, attempt_limit, next_candidate)
}

#[cfg(test)]
pub(in super::super::super) fn connection_temporary_path_for_test(
    parent: &Path,
    random: [u8; 16],
) -> PathBuf {
    connection_temporary_path(parent, random)
}

#[cfg(test)]
pub(in super::super::super) const CONNECTION_TEMPORARY_ATTEMPTS_FOR_TEST: usize =
    CONNECTION_TEMPORARY_ATTEMPTS;

fn read_snapshot(path: &Path) -> Result<ConnectionSnapshot, ConnectionRepositoryError> {
    let mut file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(ConnectionSnapshot {
                revision: ConnectionRevision::Absent,
                preference: None,
                accounts: Vec::new(),
                bindings: Vec::new(),
                catalog_seeds: Vec::new(),
                encoded: Vec::new(),
            });
        },
        Err(source) => return Err(ConnectionRepositoryError::io(path, source)),
    };
    let before = MetadataSnapshot::capture(path, &file)?;
    before.validate(path)?;
    let mut encoded = Vec::with_capacity(
        usize::try_from(before.len.min(MAX_CONNECTION_BYTES))
            .unwrap_or(MAX_CONNECTION_BYTES as usize),
    );
    Read::by_ref(&mut file)
        .take(MAX_CONNECTION_BYTES + 1)
        .read_to_end(&mut encoded)
        .map_err(|source| ConnectionRepositoryError::io(path, source))?;
    if encoded.len() as u64 > MAX_CONNECTION_BYTES {
        return Err(ConnectionRepositoryError::TooLarge(path.to_owned()));
    }
    let after = MetadataSnapshot::capture(path, &file)?;
    if before != after {
        return Err(ConnectionRepositoryError::Changed(path.to_owned()));
    }
    let decoded = wire::decode(path, &encoded)?;
    Ok(ConnectionSnapshot {
        revision: decoded.revision,
        preference: decoded.preference,
        accounts: decoded.accounts,
        bindings: decoded.bindings,
        catalog_seeds: decoded.catalog_seeds,
        encoded,
    })
}
