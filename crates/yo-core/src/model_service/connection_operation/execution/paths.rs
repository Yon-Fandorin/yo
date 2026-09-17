use std::{
    fs,
    io::ErrorKind,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
};

use rustix::process;

use super::{ConnectionOperationExecutionError, ConnectionOperationRepositoryKind};

pub(in super::super) fn validated_parent<'a>(
    repository: ConnectionOperationRepositoryKind,
    path: &'a Path,
    filename: &str,
) -> Result<&'a Path, ConnectionOperationExecutionError> {
    let valid_filename = path.file_name().is_some_and(|name| name == filename);
    let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return Err(ConnectionOperationExecutionError::InvalidRepositoryLayout {
            repository,
            path: path.to_owned(),
        });
    };
    if !path.is_absolute() || !valid_filename {
        return Err(ConnectionOperationExecutionError::InvalidRepositoryLayout {
            repository,
            path: path.to_owned(),
        });
    }
    Ok(parent)
}

pub(in super::super) fn validate_path_components(
    repository: ConnectionOperationRepositoryKind,
    path: &Path,
) -> Result<(), ConnectionOperationExecutionError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
    {
        return Err(ConnectionOperationExecutionError::InvalidRepositoryLayout {
            repository,
            path: path.to_owned(),
        });
    }
    let mut current = PathBuf::from("/");
    for component in path.components().skip(1) {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ConnectionOperationExecutionError::InvalidRepositoryLayout {
                    repository,
                    path: current,
                });
            },
            Ok(_) => {},
            Err(source) if source.kind() == ErrorKind::NotFound => break,
            Err(_) => {
                return Err(ConnectionOperationExecutionError::InvalidRepositoryLayout {
                    repository,
                    path: current,
                });
            },
        }
    }
    Ok(())
}

#[derive(Debug)]
pub(in super::super) struct LocalDirectoryIdentity {
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl LocalDirectoryIdentity {
    pub(in super::super) fn capture(
        path: &Path,
    ) -> Result<Self, ConnectionOperationExecutionError> {
        validate_path_components(ConnectionOperationRepositoryKind::Public, path)?;
        let directory = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_DIRECTORY)
            .open(path)
            .map_err(
                |_| ConnectionOperationExecutionError::InvalidRepositoryLayout {
                    repository: ConnectionOperationRepositoryKind::Public,
                    path: path.to_owned(),
                },
            )?;
        let metadata = directory.metadata().map_err(|_| {
            ConnectionOperationExecutionError::InvalidRepositoryLayout {
                repository: ConnectionOperationRepositoryKind::Public,
                path: path.to_owned(),
            }
        })?;
        if !metadata.is_dir() || metadata.uid() != process::geteuid().as_raw() {
            return Err(ConnectionOperationExecutionError::InvalidRepositoryLayout {
                repository: ConnectionOperationRepositoryKind::Public,
                path: path.to_owned(),
            });
        }
        Ok(Self {
            path: path.to_owned(),
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    pub(in super::super) fn revalidate(&self) -> Result<(), ConnectionOperationExecutionError> {
        let observed = Self::capture(&self.path)?;
        if (observed.device, observed.inode) != (self.device, self.inode) {
            return Err(ConnectionOperationExecutionError::InvalidRepositoryLayout {
                repository: ConnectionOperationRepositoryKind::Public,
                path: self.path.clone(),
            });
        }
        Ok(())
    }
}
