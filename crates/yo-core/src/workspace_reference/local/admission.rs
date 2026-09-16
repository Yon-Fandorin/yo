use std::{
    fs,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use rustix::{
    fs::{AtFlags, FileType, Mode, OFlags, fstat, openat, statat},
    io::Errno,
};

use super::{
    discovery::{pin_directory, pin_root},
    inventory::{reference as make_reference, root_identity},
};
use crate::{
    InputAdmissionHost, InputReference, SubmissionRejection, SubmissionRejectionKind, UserInput,
    WorkspaceHostId, WorkspaceReferenceKind,
};

/// Local execution-host admission for workspace path references, without reading contents.
/// Skill references require an additional authoritative skill admission implementation.
#[derive(Debug)]
pub struct LocalWorkspaceInputAdmission {
    root: PathBuf,
    host: WorkspaceHostId,
    root_identity: String,
}

impl LocalWorkspaceInputAdmission {
    /// Captures the canonical root mapping used by this live execution environment.
    pub fn new(root: &Path, host: WorkspaceHostId) -> Result<Self, String> {
        let root = fs::canonicalize(root).map_err(|error| error.to_string())?;
        let descriptor = pin_root(&root).map_err(|error| {
            format!("workspace root {} is unavailable: {error}", root.display())
        })?;
        let root_identity = root_identity(&root, &descriptor)?;
        Ok(Self {
            root,
            host,
            root_identity,
        })
    }
}

impl InputAdmissionHost for LocalWorkspaceInputAdmission {
    fn validate(&self, input: &UserInput) -> Result<(), SubmissionRejection> {
        let reject = |kind, detail| SubmissionRejection::new(kind, detail);
        if input.references().len() > 128 {
            return Err(reject(
                SubmissionRejectionKind::OverBudget,
                "at most 128 input references are supported".to_owned(),
            ));
        }
        // Reject unsupported kinds before attempting any workspace lookup or asset loading.
        if input
            .references()
            .iter()
            .any(|reference| matches!(reference, InputReference::Skill { .. }))
        {
            return Err(reject(
                SubmissionRejectionKind::Incompatible,
                "this execution host has no skill admission capability".to_owned(),
            ));
        }
        let root =
            pin_root(&self.root).map_err(|error| reference_access_rejection(&self.root, error))?;
        let current_root = root_identity(&self.root, &root)
            .map_err(|detail| reject(SubmissionRejectionKind::EnvironmentUnavailable, detail))?;
        if current_root != self.root_identity {
            return Err(reject(
                SubmissionRejectionKind::StaleReference,
                "workspace root mapping changed; select references again".to_owned(),
            ));
        }
        for occurrence in input.references() {
            let reference = occurrence
                .workspace_reference()
                .expect("unsupported reference kinds were rejected");
            if reference.relative_path().len() > 4096 {
                return Err(reject(
                    SubmissionRejectionKind::OverBudget,
                    "workspace reference path exceeds 4096 bytes".to_owned(),
                ));
            }
            if reference.execution_environment_identity() != format!("local-host:{}", self.host)
                || reference.workspace_identity() != format!("{}:{}", self.host, self.root_identity)
            {
                return Err(reject(
                    SubmissionRejectionKind::EnvironmentUnavailable,
                    "reference belongs to another execution environment or workspace".to_owned(),
                ));
            }
            let expected = make_reference(
                &self.root_identity,
                self.host,
                reference.relative_path().to_owned(),
                reference.kind(),
            )
            .map_err(|detail| reject(SubmissionRejectionKind::InvalidReference, detail))?;
            if &expected != reference {
                return Err(reject(
                    SubmissionRejectionKind::StaleReference,
                    "workspace reference identity changed; select it again".to_owned(),
                ));
            }
            let relative = Path::new(reference.relative_path());
            if relative
                .components()
                .any(|part| part.as_os_str().as_bytes().eq_ignore_ascii_case(b".git"))
            {
                return Err(reject(
                    SubmissionRejectionKind::Unauthorized,
                    "Git internals cannot be attached as workspace references".to_owned(),
                ));
            }
            let parent = pin_directory(&root, relative.parent().unwrap_or_else(|| Path::new("")))
                .map_err(|error| reference_access_rejection(relative, error))?;
            let name = relative
                .file_name()
                .expect("validated relative path has a basename");
            let expected_kind = match reference.kind() {
                WorkspaceReferenceKind::File => FileType::RegularFile,
                WorkspaceReferenceKind::Directory => FileType::Directory,
            };
            // Exclude special files before opening; verify again on the opened descriptor.
            let before = statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|error| reference_access_rejection(relative, error))?;
            if FileType::from_raw_mode(before.st_mode) != expected_kind {
                return Err(reject(
                    SubmissionRejectionKind::StaleReference,
                    format!("reference {} changed kind", reference.relative_path()),
                ));
            }
            let descriptor = openat(
                &parent,
                name,
                OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| reference_access_rejection(relative, error))?;
            let metadata = fstat(&descriptor).map_err(|error| {
                reject(
                    SubmissionRejectionKind::EnvironmentUnavailable,
                    error.to_string(),
                )
            })?;
            if FileType::from_raw_mode(metadata.st_mode) != expected_kind {
                return Err(reject(
                    SubmissionRejectionKind::StaleReference,
                    format!("reference {} changed kind", reference.relative_path()),
                ));
            }
        }
        // A rename during validation must not substitute a different backend working root.
        let descriptor =
            pin_root(&self.root).map_err(|error| reference_access_rejection(&self.root, error))?;
        let current = root_identity(&self.root, &descriptor)
            .map_err(|detail| reject(SubmissionRejectionKind::EnvironmentUnavailable, detail))?;
        if current != self.root_identity {
            return Err(reject(
                SubmissionRejectionKind::StaleReference,
                "workspace root changed during admission".to_owned(),
            ));
        }
        Ok(())
    }
}

fn reference_access_rejection(path: &Path, error: Errno) -> SubmissionRejection {
    SubmissionRejection::new(
        if matches!(error, Errno::ACCESS | Errno::PERM) {
            SubmissionRejectionKind::Unauthorized
        } else {
            SubmissionRejectionKind::StaleReference
        },
        format!("reference {} is unavailable: {error}", path.display()),
    )
}
