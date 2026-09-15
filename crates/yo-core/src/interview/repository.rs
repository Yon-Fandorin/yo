use std::{
    fs::{File, TryLockError},
    io::{Error, Read, Write},
    os::unix::fs::MetadataExt,
    path::{Component, Path},
};

use rustix::{
    fs::{self, Dir, Mode, OFlags},
    io::Errno,
    process,
};

use super::{
    COPY_LIMIT, InterviewCatalog, InterviewError, WorkingCopy, invalid,
    working_copy::{new_id, valid_id},
};

/// Pinned, user-owned storage for editable copies. It never writes a Session Journal.
#[derive(Debug)]
pub struct InterviewRepository {
    root: File,
}
pub type InterviewCopyEntry = (String, Result<WorkingCopy, InterviewError>);
impl InterviewRepository {
    pub fn open(path: &Path) -> Result<Self, InterviewError> {
        if !path.is_absolute() {
            return Err(invalid("interview repository must have an absolute path"));
        }
        let components = path.components().collect::<Vec<_>>();
        let mut root = File::from(
            fs::open(
                "/",
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(Error::from)?,
        );
        for (i, component) in components.iter().enumerate().skip(1) {
            let Component::Normal(name) = component else {
                return Err(invalid("invalid interview repository path"));
            };
            let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
            let fd = match fs::openat(&root, *name, flags, Mode::empty()) {
                Err(Errno::NOENT) if i + 1 == components.len() => {
                    match fs::mkdirat(&root, *name, Mode::from_raw_mode(0o700)) {
                        Ok(()) | Err(Errno::EXIST) => {},
                        Err(e) => return Err(Error::from(e).into()),
                    }
                    fs::openat(&root, *name, flags, Mode::empty())
                },
                value => value,
            }
            .map_err(Error::from)?;
            if i + 1 == components.len() {
                root.sync_all()?;
            }
            root = File::from(fd);
        }
        secure(&root, true)?;
        let repository = Self { root };
        match repository.lease() {
            Ok(_lease) => {},
            Err(InterviewError::Busy) => {}, /* An active writer owns its temps; later */
            // operations reclaim.
            Err(error) => return Err(error),
        }
        Ok(repository)
    }
    fn lease(&self) -> Result<File, InterviewError> {
        secure(&self.root, true)?;
        let file = File::from(
            fs::openat(
                &self.root,
                ".copy.lock",
                OFlags::RDWR
                    | OFlags::CREATE
                    | OFlags::NOFOLLOW
                    | OFlags::NONBLOCK
                    | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )
            .map_err(Error::from)?,
        );
        secure(&file, false)?;
        match file.try_lock() {
            Ok(()) => {
                self.cleanup_abandoned()?;
                Ok(file)
            },
            Err(TryLockError::WouldBlock) => Err(InterviewError::Busy),
            Err(TryLockError::Error(e)) => Err(e.into()),
        }
    }
    // The private reserved .<copy UUID>.<attempt UUID>.tmp namespace belongs to
    // this repository. Exclusive lease proves no Yo writer still owns an attempt.
    fn cleanup_abandoned(&self) -> Result<(), InterviewError> {
        let mut removed = false;
        for (index, entry) in Dir::read_from(&self.root).map_err(Error::from)?.enumerate() {
            if index >= 4096 {
                return Err(invalid(
                    "interview repository directory exceeds its read limit",
                ));
            }
            let entry = entry.map_err(Error::from)?;
            let Ok(name) = entry.file_name().to_str() else {
                continue;
            };
            let Some(stem) = name.strip_prefix('.').and_then(|s| s.strip_suffix(".tmp")) else {
                continue;
            };
            let Some((copy, attempt)) = stem.split_once('.') else {
                continue;
            };
            if !valid_id(copy) || !valid_id(attempt) {
                continue;
            }
            let Ok(fd) = fs::openat(
                &self.root,
                name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            ) else {
                continue;
            };
            let file = File::from(fd);
            if secure(&file, false).is_err() {
                continue;
            }
            let metadata = file.metadata()?;
            if metadata.len() > COPY_LIMIT as u64 {
                continue;
            }
            let Ok(current) = fs::openat(
                &self.root,
                name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            ) else {
                continue;
            };
            let current = File::from(current);
            if secure(&current, false).is_err() {
                continue;
            }
            let current = current.metadata()?;
            if current.dev() != metadata.dev() || current.ino() != metadata.ino() {
                continue;
            }
            fs::unlinkat(&self.root, name, fs::AtFlags::empty()).map_err(Error::from)?;
            removed = true;
        }
        if removed {
            self.root.sync_all()?;
        }
        Ok(())
    }
    fn read_unlocked(&self, id: &str) -> Result<Option<WorkingCopy>, InterviewError> {
        if !valid_id(id) {
            return Err(invalid("invalid interview copy identity"));
        }
        let fd = match fs::openat(
            &self.root,
            format!("{id}.json"),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Err(Errno::NOENT) => return Ok(None),
            value => value.map_err(Error::from)?,
        };
        let file = File::from(fd);
        secure(&file, false)?;
        let mut bytes = Vec::new();
        file.take((COPY_LIMIT + 1) as u64).read_to_end(&mut bytes)?;
        let copy = WorkingCopy::decode(&bytes)?;
        if copy.copy_id != id {
            return Err(invalid("interview copy filename does not match content"));
        }
        Ok(Some(copy))
    }
    pub fn load(&self, id: &str) -> Result<Option<WorkingCopy>, InterviewError> {
        let _lease = self.lease()?;
        self.read_unlocked(id)
    }
    /// Lists bounded public copy identities; malformed records remain untouched and report errors.
    pub fn list(&self) -> Result<Vec<InterviewCopyEntry>, InterviewError> {
        let _lease = self.lease()?;
        let mut copies = Vec::new();
        for (i, entry) in Dir::read_from(&self.root).map_err(Error::from)?.enumerate() {
            if i >= 4096 {
                return Err(invalid(
                    "interview repository directory exceeds its read limit",
                ));
            }
            let entry = entry.map_err(Error::from)?;
            let Ok(name) = entry.file_name().to_str() else {
                continue;
            };
            let Some(id) = name.strip_suffix(".json") else {
                continue;
            };
            if !valid_id(id) {
                continue;
            }
            let value = self
                .read_unlocked(id)
                .and_then(|value| value.ok_or_else(|| invalid("copy disappeared")));
            copies.push((id.to_owned(), value));
        }
        copies.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(copies)
    }
    /// CAS publication under one exclusive lease. Caller advances generation only after success.
    pub fn save(
        &self,
        copy: &WorkingCopy,
        expected_generation: Option<u64>,
        catalog: &InterviewCatalog,
    ) -> Result<u64, InterviewError> {
        copy.validate(catalog)?;
        let _lease = self.lease()?;
        let current = self.read_unlocked(&copy.copy_id)?;
        if current.as_ref().map(|v| v.generation) != expected_generation {
            return Err(InterviewError::Conflict);
        }
        let generation = match expected_generation {
            None => 1,
            Some(value) => value
                .checked_add(1)
                .ok_or_else(|| invalid("interview generation overflow"))?,
        };
        let mut published = copy.clone();
        published.generation = generation;
        let bytes = published.encode()?;
        let temp = format!(".{}.{}.tmp", copy.copy_id, new_id()?);
        let mut file = File::from(
            fs::openat(
                &self.root,
                temp.as_str(),
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )
            .map_err(Error::from)?,
        );
        let result = (|| {
            secure(&file, false)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            secure(&file, false)?;
            secure(&self.root, true)?;
            fs::renameat(
                &self.root,
                temp.as_str(),
                &self.root,
                format!("{}.json", copy.copy_id),
            )
            .map_err(Error::from)?;
            self.root.sync_all()?;
            secure(&self.root, true)?;
            Ok(generation)
        })();
        if result.is_err() {
            let _ = fs::unlinkat(&self.root, temp.as_str(), fs::AtFlags::empty());
        }
        result
    }
}
fn secure(file: &File, directory: bool) -> Result<(), InterviewError> {
    let metadata = file.metadata()?;
    let kind = if directory {
        metadata.is_dir()
    } else {
        metadata.is_file() && metadata.nlink() == 1
    };
    if !kind
        || metadata.uid() != process::geteuid().as_raw()
        || metadata.mode() & 0o7777 != if directory { 0o700 } else { 0o600 }
    {
        return Err(invalid(
            "interview storage must be user-owned with directory0700 and regular files0600",
        ));
    }
    Ok(())
}
