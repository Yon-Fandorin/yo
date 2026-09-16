use std::{
    fs::File,
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
    sync::Mutex,
};

use rustix::{
    fd::OwnedFd,
    fs::{Mode, OFlags, open, openat},
};

use super::super::SkillReferenceScope;

pub(super) const MAX_ROOTS: usize = 16;
pub(super) const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

/// An explicitly configured source; no default directories or backend settings are inferred.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalSkillRoot {
    path: PathBuf,
    scope: SkillReferenceScope,
}

impl LocalSkillRoot {
    /// Accepts absolute workspace/user source paths. Existence is checked at host binding.
    pub fn new(path: PathBuf, scope: SkillReferenceScope) -> Result<Self, String> {
        if !path.is_absolute()
            || !matches!(
                scope,
                SkillReferenceScope::Workspace | SkillReferenceScope::User
            )
        {
            return Err(
                "local skill roots require an absolute path and Workspace or User scope".into(),
            );
        }
        Ok(Self { path, scope })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) const fn scope(&self) -> SkillReferenceScope {
        self.scope
    }
}

pub(super) struct Root {
    path: PathBuf,
    scope: SkillReferenceScope,
    identity: Mutex<Option<(u64, u64)>>,
}

impl Root {
    pub(super) fn new(path: PathBuf, scope: SkillReferenceScope) -> Self {
        Self {
            path,
            scope,
            identity: Mutex::new(None),
        }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) const fn scope(&self) -> SkillReferenceScope {
        self.scope
    }

    pub(super) fn identity(&self) -> Option<(u64, u64)> {
        *self
            .identity
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    pub(super) fn open(&self) -> Result<OwnedFd, String> {
        let descriptor = open_directory(&self.path)?;
        let file = File::from(descriptor);
        let metadata = file.metadata().map_err(|error| error.to_string())?;
        let current = (metadata.dev(), metadata.ino());
        let mut identity = self
            .identity
            .lock()
            .map_err(|_| "skill root identity poisoned")?;
        if identity.is_some_and(|expected| expected != current) {
            return Err("skill root mapping changed".into());
        }
        *identity = Some(current);
        Ok(file.into())
    }
}

fn open_directory(path: &Path) -> Result<OwnedFd, String> {
    let mut descriptor =
        open("/", DIRECTORY_FLAGS, Mode::empty()).map_err(|error| error.to_string())?;
    for component in path.components() {
        match component {
            Component::RootDir => {},
            Component::Normal(name) => {
                descriptor = openat(&descriptor, name, DIRECTORY_FLAGS, Mode::empty())
                    .map_err(|error| error.to_string())?;
            },
            _ => return Err("skill roots cannot contain parent components or symlinks".into()),
        }
    }
    Ok(descriptor)
}
