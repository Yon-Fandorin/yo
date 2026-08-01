use std::{
    error::Error,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use path::prepare_state_root;

use super::{WorkspaceHostId, WorkspaceHostIdGenerationError};

mod path;

const FILE_MODE: u32 = 0o600;
const ID_FILE: &str = "host-id";
const FILE_SCHEMA: &str = "yo.workspace-host-id/v1";
const ENCODED_ID_BYTES: u64 = 61;

/// A stable per-user Yo Host identity backed by one local state file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LocalWorkspaceHostIdentity {
    id: WorkspaceHostId,
}

impl LocalWorkspaceHostIdentity {
    /// Opens or atomically creates the identity below a platform-selected state root.
    pub fn open(state_root: impl AsRef<Path>) -> Result<Self, LocalWorkspaceHostIdentityError> {
        let state_root = prepare_state_root(state_root.as_ref())?;
        let path = state_root.join(ID_FILE);
        match read_identity(&path) {
            Ok(id) => Ok(Self { id }),
            Err(LocalWorkspaceHostIdentityError::Io { source, .. })
                if source.kind() == io::ErrorKind::NotFound =>
            {
                create_identity(&state_root, &path).map(|id| Self { id })
            },
            Err(error) => Err(error),
        }
    }

    pub const fn id(self) -> WorkspaceHostId {
        self.id
    }
}

fn read_identity(path: &Path) -> Result<WorkspaceHostId, LocalWorkspaceHostIdentityError> {
    reject_symlink(path)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|source| io_error("open", path, source))?;
    let metadata = file
        .metadata()
        .map_err(|source| io_error("inspect", path, source))?;
    if !metadata.is_file() {
        return Err(LocalWorkspaceHostIdentityError::Invalid {
            path: path.to_owned(),
            reason: "the Workspace Host identity is not a regular file".to_owned(),
        });
    }
    let mode = metadata.permissions().mode() & 0o777;
    if mode != FILE_MODE {
        return Err(LocalWorkspaceHostIdentityError::Invalid {
            path: path.to_owned(),
            reason: format!("Workspace Host identity file permissions {mode:o} are not 600"),
        });
    }
    let mut encoded = Vec::new();
    file.take(ENCODED_ID_BYTES.saturating_add(1))
        .read_to_end(&mut encoded)
        .map_err(|source| io_error("read", path, source))?;
    if u64::try_from(encoded.len()).unwrap_or(u64::MAX) > ENCODED_ID_BYTES {
        return Err(invalid_identity(path, "the identity file is too large"));
    }
    let encoded = std::str::from_utf8(&encoded)
        .map_err(|_| invalid_identity(path, "the identity file is not UTF-8"))?;
    let record = encoded
        .strip_suffix('\n')
        .ok_or_else(|| invalid_identity(path, "the identity file has no final newline"))?;
    let value = record
        .strip_prefix(FILE_SCHEMA)
        .and_then(|record| record.strip_prefix(' '))
        .ok_or_else(|| invalid_identity(path, "the identity file has an unsupported schema"))?;
    let id = value
        .parse::<WorkspaceHostId>()
        .map_err(|error| invalid_identity(path, error.to_string()))?;
    if value != id.to_string() {
        return Err(invalid_identity(
            path,
            "the identity is not in canonical lowercase UUID form",
        ));
    }
    Ok(id)
}

fn create_identity(
    root: &Path,
    path: &Path,
) -> Result<WorkspaceHostId, LocalWorkspaceHostIdentityError> {
    let candidate = WorkspaceHostId::new().map_err(LocalWorkspaceHostIdentityError::Generation)?;
    let temporary = root.join(format!(".{ID_FILE}.{candidate}.tmp"));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(FILE_MODE)
        .open(&temporary)
        .map_err(|source| io_error("create temporary identity at", &temporary, source))?;
    if let Err(source) = file.set_permissions(fs::Permissions::from_mode(FILE_MODE)) {
        let _ = fs::remove_file(&temporary);
        return Err(io_error("set permissions on", &temporary, source));
    }
    let encoded = format!("{FILE_SCHEMA} {candidate}\n");
    if let Err(source) = file
        .write_all(encoded.as_bytes())
        .and_then(|()| file.sync_all())
    {
        let _ = fs::remove_file(&temporary);
        return Err(io_error("write temporary identity at", &temporary, source));
    }
    drop(file);

    match fs::hard_link(&temporary, path) {
        Ok(()) => {
            sync_directory(root)?;
            fs::remove_file(&temporary)
                .map_err(|source| io_error("remove temporary identity at", &temporary, source))?;
            sync_directory(root)?;
            Ok(candidate)
        },
        Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
            fs::remove_file(&temporary)
                .map_err(|cleanup| io_error("remove temporary identity at", &temporary, cleanup))?;
            let id = read_identity(path)?;
            sync_directory(root)?;
            Ok(id)
        },
        Err(source) => {
            let _ = fs::remove_file(&temporary);
            Err(io_error("publish identity at", path, source))
        },
    }
}

pub(super) fn sync_directory(path: &Path) -> Result<(), LocalWorkspaceHostIdentityError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error("synchronize", path, source))
}

fn reject_symlink(path: &Path) -> Result<(), LocalWorkspaceHostIdentityError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(LocalWorkspaceHostIdentityError::Invalid {
                path: path.to_owned(),
                reason: "symbolic links are not allowed at the Workspace Host identity path"
                    .to_owned(),
            })
        },
        Ok(_) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error("inspect", path, source)),
    }
}

fn invalid_identity(path: &Path, reason: impl Into<String>) -> LocalWorkspaceHostIdentityError {
    LocalWorkspaceHostIdentityError::Invalid {
        path: path.to_owned(),
        reason: reason.into(),
    }
}

pub(super) fn io_error(
    operation: &'static str,
    path: &Path,
    source: io::Error,
) -> LocalWorkspaceHostIdentityError {
    LocalWorkspaceHostIdentityError::Io {
        operation,
        path: path.to_owned(),
        source,
    }
}

#[derive(Debug)]
pub enum LocalWorkspaceHostIdentityError {
    Generation(WorkspaceHostIdGenerationError),
    Invalid {
        path: PathBuf,
        reason: String,
    },
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for LocalWorkspaceHostIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Generation(error) => error.fmt(formatter),
            Self::Invalid { path, reason } => {
                write!(
                    formatter,
                    "invalid Workspace Host identity at {}: {reason}",
                    path.display()
                )
            },
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "failed to {operation} Workspace Host identity state at {}: {source}",
                path.display()
            ),
        }
    }
}

impl Error for LocalWorkspaceHostIdentityError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Generation(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::Invalid { .. } => None,
        }
    }
}
