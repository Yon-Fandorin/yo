use std::{
    fs::{File, TryLockError},
    io::{Error, ErrorKind, Read, Write},
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
};

use rustix::{
    fs::{self, Dir, Mode, OFlags},
    io::Errno,
    process,
};

use super::{
    COPY_LIMIT, InterviewCatalog, InterviewError, SecretRecoveryDestination, WorkingCopy, invalid,
    recovery::{RecoveryBinding, RecoveryStore, SecretRecoveryReference},
    working_copy::{new_id, valid_id},
};
use crate::SecretInput;

/// Pinned, user-owned storage for editable copies. It never writes a Session Journal.
#[derive(Debug)]
pub struct InterviewRepository {
    root: File,
    recovery: Option<RecoveryStore>,
}
pub type InterviewCopyEntry = (String, Result<WorkingCopy, InterviewError>);

#[derive(Debug)]
pub struct SecretRecoveryUpdate {
    pub copy: WorkingCopy,
    pub cleanup_warning: Option<String>,
}

#[derive(Debug, Default)]
pub struct SecretRecoveryMaintenance {
    pub expired_references: usize,
    pub warning: Option<String>,
}

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
        let repository = Self {
            root,
            recovery: None,
        };
        match repository.lease() {
            Ok(_lease) => {},
            Err(InterviewError::Busy) => {}, /* An active writer owns its temps; later */
            // operations reclaim.
            Err(error) => return Err(error),
        }
        Ok(repository)
    }

    pub fn open_with_recovery(
        copy_path: &Path,
        vault_path: PathBuf,
        key_path: PathBuf,
    ) -> Result<Self, InterviewError> {
        let mut repository = Self::open(copy_path)?;
        repository.recovery = Some(RecoveryStore::new(vault_path, key_path)?);
        Ok(repository)
    }

    pub fn recovery_boundary(&self) -> Option<String> {
        self.recovery.as_ref().map(RecoveryStore::boundary)
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
        let _lease = self.lease()?;
        self.save_unlocked(copy, expected_generation, catalog)
    }

    fn save_unlocked(
        &self,
        copy: &WorkingCopy,
        expected_generation: Option<u64>,
        catalog: &InterviewCatalog,
    ) -> Result<u64, InterviewError> {
        copy.validate(catalog)?;
        self.save_validated_unlocked(copy, expected_generation)
    }

    /// Recovery-reference removal preserves the already decoded working-copy
    /// shape and does not need its historical Journal to remain available.
    fn save_shape_unlocked(
        &self,
        copy: &WorkingCopy,
        expected_generation: Option<u64>,
    ) -> Result<u64, InterviewError> {
        copy.encode()?;
        self.save_validated_unlocked(copy, expected_generation)
    }

    fn save_validated_unlocked(
        &self,
        copy: &WorkingCopy,
        expected_generation: Option<u64>,
    ) -> Result<u64, InterviewError> {
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

    pub fn store_secret_recovery(
        &self,
        copy: &WorkingCopy,
        expected_generation: Option<u64>,
        catalog: &InterviewCatalog,
        question_id: &str,
        destination: &SecretRecoveryDestination,
        secret: &SecretInput,
    ) -> Result<WorkingCopy, InterviewError> {
        let recovery = self
            .recovery
            .as_ref()
            .ok_or_else(|| invalid("secret recovery storage is unavailable"))?;
        let capture = copy.validate(catalog)?;
        let question = capture
            .questions
            .iter()
            .find(|question| question.id == question_id && question.is_secret)
            .ok_or_else(|| invalid("secret recovery question does not match the saved copy"))?;
        question.validate_answer(
            copy.answers
                .iter()
                .find(|answer| answer.question_id == question_id)
                .ok_or_else(|| invalid("secret recovery answer marker is missing"))?,
            true,
        )?;
        let fingerprint = capture.public_batch_fingerprint()?;
        let _lease = self.lease()?;
        let current = self.read_unlocked(&copy.copy_id)?;
        if current.as_ref().map(|value| value.generation) != expected_generation {
            return Err(InterviewError::Conflict);
        }
        let entry = recovery.write(
            &copy.copy_id,
            &fingerprint,
            question_id,
            destination,
            secret,
        )?;
        let old = copy.recovery_reference(question_id).cloned();
        let mut published = copy.clone();
        published.set_recovery_reference(SecretRecoveryReference::new(
            question_id.to_owned(),
            entry.id.clone(),
        ));
        match self.save_unlocked(&published, expected_generation, catalog) {
            Ok(generation) => {
                published.generation = generation;
                if let Some(old) = old {
                    let _ = recovery.delete(&old.entry_id);
                }
                Ok(published)
            },
            Err(error) => {
                let reference_state = self.read_unlocked(&copy.copy_id).map(|current| {
                    current.is_some_and(|current| {
                        current
                            .recovery_reference(question_id)
                            .is_some_and(|reference| reference.entry_id == entry.id)
                    })
                });
                // A failed fsync or readback can follow a successful rename.
                // Reclaim only when a successful read positively proves the
                // published copy does not name this entry.
                if matches!(reference_state, Ok(false)) {
                    let _ = recovery.delete_written(&entry);
                }
                Err(error)
            },
        }
    }

    pub fn recovery_available(
        &self,
        copy: &WorkingCopy,
        catalog: &InterviewCatalog,
        live_capture: &super::CapturedInterview,
        question_id: &str,
        destination: &SecretRecoveryDestination,
    ) -> Result<bool, InterviewError> {
        let Some(recovery) = &self.recovery else {
            return Ok(false);
        };
        let source = copy.validate(catalog)?;
        if source.public_batch_fingerprint()? != live_capture.public_batch_fingerprint()?
            || !live_capture
                .questions
                .iter()
                .any(|question| question.id == question_id && question.is_secret)
        {
            return Ok(false);
        }
        let Some(reference) = copy.recovery_reference(question_id) else {
            return Ok(false);
        };
        recovery.authenticate(RecoveryBinding {
            entry_id: &reference.entry_id,
            copy_id: &copy.copy_id,
            batch_fingerprint: &source.public_batch_fingerprint()?,
            question_id,
            destination,
        })?;
        Ok(true)
    }

    pub fn stored_recovery_available(
        &self,
        copy: &WorkingCopy,
        question_id: &str,
    ) -> Result<bool, InterviewError> {
        let Some(recovery) = &self.recovery else {
            return Ok(false);
        };
        let Some(reference) = copy.recovery_reference(question_id) else {
            return Ok(false);
        };
        recovery.available(&reference.entry_id)
    }

    pub fn recover_secret(
        &self,
        copy: &WorkingCopy,
        catalog: &InterviewCatalog,
        live_capture: &super::CapturedInterview,
        question_id: &str,
        destination: &SecretRecoveryDestination,
    ) -> Result<SecretInput, InterviewError> {
        let recovery = self
            .recovery
            .as_ref()
            .ok_or_else(|| invalid("secret recovery storage is unavailable"))?;
        let source = copy.validate(catalog)?;
        let fingerprint = source.public_batch_fingerprint()?;
        if fingerprint != live_capture.public_batch_fingerprint()?
            || !live_capture
                .questions
                .iter()
                .any(|question| question.id == question_id && question.is_secret)
        {
            return Err(invalid(
                "the live secret request does not match the saved public batch",
            ));
        }
        let reference = copy
            .recovery_reference(question_id)
            .ok_or_else(|| invalid("no secret recovery entry is available"))?;
        let _lease = self.lease()?;
        let current = self
            .read_unlocked(&copy.copy_id)?
            .ok_or_else(|| invalid("saved interview copy is unavailable"))?;
        if current.generation != copy.generation
            || current.recovery_reference(question_id) != Some(reference)
        {
            return Err(InterviewError::Conflict);
        }
        recovery.read(RecoveryBinding {
            entry_id: &reference.entry_id,
            copy_id: &copy.copy_id,
            batch_fingerprint: &fingerprint,
            question_id,
            destination,
        })
    }

    pub fn forget_secret_recovery(
        &self,
        copy: &WorkingCopy,
        expected_generation: Option<u64>,
        catalog: &InterviewCatalog,
        question_id: &str,
    ) -> Result<SecretRecoveryUpdate, InterviewError> {
        let mut published = copy.clone();
        let reference = published
            .take_recovery_reference(question_id)
            .ok_or_else(|| invalid("no secret recovery entry is available"))?;
        let _lease = self.lease()?;
        let generation = self.save_unlocked(&published, expected_generation, catalog)?;
        published.generation = generation;
        let cleanup_warning = self.recovery.as_ref().and_then(|recovery| {
            recovery
                .delete(&reference.entry_id)
                .err()
                .map(|error| format!("encrypted entry cleanup remains pending: {error}"))
        });
        Ok(SecretRecoveryUpdate {
            copy: published,
            cleanup_warning,
        })
    }

    pub fn forget_all_secret_recovery(
        &self,
        copy: &WorkingCopy,
        expected_generation: Option<u64>,
        catalog: &InterviewCatalog,
    ) -> Result<SecretRecoveryUpdate, InterviewError> {
        let mut published = copy.clone();
        let references = published.take_all_recovery_references();
        if references.is_empty() {
            return Ok(SecretRecoveryUpdate {
                copy: published,
                cleanup_warning: None,
            });
        }
        let _lease = self.lease()?;
        let generation = self.save_unlocked(&published, expected_generation, catalog)?;
        published.generation = generation;
        let mut cleanup_warning = None;
        if let Some(recovery) = &self.recovery {
            for reference in references {
                if let Err(error) = recovery.delete(&reference.entry_id) {
                    cleanup_warning = Some(format!(
                        "one or more encrypted entry cleanups remain pending: {error}"
                    ));
                }
            }
        }
        Ok(SecretRecoveryUpdate {
            copy: published,
            cleanup_warning,
        })
    }

    pub fn expire_secret_recovery(
        &self,
        copy: &WorkingCopy,
        expected_generation: Option<u64>,
        catalog: &InterviewCatalog,
    ) -> Result<Option<SecretRecoveryUpdate>, InterviewError> {
        let Some(recovery) = &self.recovery else {
            return Ok(None);
        };
        let mut expired_ids = Vec::new();
        for reference in &copy.secret_recovery {
            match recovery.expired(&reference.entry_id) {
                Ok(true) => expired_ids.push(reference.question_id.clone()),
                Ok(false) => {},
                Err(InterviewError::Io(error)) if error.kind() == ErrorKind::NotFound => {},
                Err(error) => return Err(error),
            }
        }
        if expired_ids.is_empty() {
            return Ok(None);
        }
        let mut published = copy.clone();
        let mut references = Vec::new();
        for id in expired_ids {
            if let Some(reference) = published.take_recovery_reference(&id) {
                references.push(reference);
            }
        }
        let _lease = self.lease()?;
        let generation = self.save_unlocked(&published, expected_generation, catalog)?;
        published.generation = generation;
        let mut cleanup_warning = None;
        for reference in references {
            if let Err(error) = recovery.delete(&reference.entry_id) {
                cleanup_warning = Some(format!("expired entry cleanup remains pending: {error}"));
            }
        }
        Ok(Some(SecretRecoveryUpdate {
            copy: published,
            cleanup_warning,
        }))
    }

    /// Removes every expired public reference before attempting to unlink its
    /// encrypted entry. This scan needs only the canonical working-copy shape,
    /// so expiry continues even when the historical Session is unavailable.
    pub fn maintain_secret_recovery(&self) -> Result<SecretRecoveryMaintenance, InterviewError> {
        let Some(recovery) = &self.recovery else {
            return Ok(SecretRecoveryMaintenance::default());
        };
        let _lease = self.lease()?;
        let mut ids = Vec::new();
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
            let Some(id) = name.strip_suffix(".json").filter(|id| valid_id(id)) else {
                continue;
            };
            ids.push(id.to_owned());
        }
        ids.sort();

        let mut result = SecretRecoveryMaintenance::default();
        for id in ids {
            let mut copy = match self.read_unlocked(&id) {
                Ok(Some(copy)) => copy,
                Ok(None) | Err(_) => continue,
            };
            let mut expired = Vec::new();
            let mut check_failed = false;
            for reference in &copy.secret_recovery {
                match recovery.expired(&reference.entry_id) {
                    Ok(true) => expired.push(reference.question_id.clone()),
                    Ok(false) => {},
                    Err(_) => check_failed = true,
                }
            }
            if check_failed {
                result.warning = Some(
                    "one or more secret recovery entries could not be checked for expiry"
                        .to_owned(),
                );
            }
            if expired.is_empty() {
                continue;
            }
            let mut references = Vec::new();
            for question_id in expired {
                if let Some(reference) = copy.take_recovery_reference(&question_id) {
                    references.push(reference);
                }
            }
            let generation = self.save_shape_unlocked(&copy, Some(copy.generation))?;
            copy.generation = generation;
            result.expired_references += references.len();
            for reference in references {
                if recovery.delete(&reference.entry_id).is_err() {
                    result.warning = Some(
                        "one or more expired encrypted entry cleanups remain pending".to_owned(),
                    );
                }
            }
        }
        Ok(result)
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
