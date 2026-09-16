use std::{
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use rustix::fs::{Dir, FileType};

use super::{
    super::{
        SkillAvailability, SkillReference, SkillReferenceCandidate, SkillReferenceSearchStatus,
    },
    reject,
    roots::{LocalSkillRoot, MAX_ROOTS, Root},
    snapshot::{digest, read_snapshot},
};
use crate::{ResolvedSkill, WorkspaceHostId};

pub(super) const MAX_ENTRIES: usize = 512;
const MAX_SCAN_BYTES: usize = 16 * 1024 * 1024;

pub(super) struct Catalog {
    pub(super) roots: Vec<Root>,
    environment: String,
}

impl Catalog {
    pub(super) fn new(roots: Vec<LocalSkillRoot>, host: WorkspaceHostId) -> Result<Self, String> {
        if roots.len() > MAX_ROOTS {
            return Err("at most 16 skill roots are supported".into());
        }
        let mut pinned = Vec::new();
        for root in roots {
            let path = root.path().components().collect::<PathBuf>();
            if path.to_str().is_none()
                || path
                    .components()
                    .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
            {
                return Err(
                    "local skill roots require UTF-8 paths without parent components".into(),
                );
            }
            if pinned.iter().any(|old: &Root| old.path() == path.as_path()) {
                return Err("duplicate local skill root".into());
            }
            let root = Root::new(path, root.scope());
            // An unavailable source cannot block replay of an already frozen skill.
            // If available, bind its identity now; otherwise bind on first successful access.
            let _ = root.open();
            pinned.push(root);
        }
        Ok(Self {
            roots: pinned,
            environment: format!("local-host:{host}"),
        })
    }

    pub(super) fn reference(
        &self,
        root: &Root,
        child: &str,
        name: &str,
        generation: u64,
        revision: &str,
    ) -> SkillReference {
        let locator = root.path().join(child).join("SKILL.md");
        let root_identity = root.identity();
        let identity_bytes = serde_json::to_vec(&(
            &self.environment,
            root.path(),
            root_identity,
            format!("{:?}", root.scope()),
            child,
        ))
        .expect("local skill identity consists of serializable strings and numbers");
        let identity = format!("local-skill:{}", digest(&identity_bytes));
        SkillReference::new(
            identity,
            &self.environment,
            locator.to_string_lossy(),
            name,
            root.scope(),
            generation,
            revision,
        )
    }

    pub(super) fn select<'a>(
        &'a self,
        selected: &SkillReference,
    ) -> Result<(&'a Root, String), crate::SubmissionRejection> {
        if selected.execution_environment_identity() != self.environment {
            return Err(reject(
                crate::SubmissionRejectionKind::EnvironmentUnavailable,
                "skill belongs to another execution host",
            ));
        }
        for root in &self.roots {
            let Ok(relative) = Path::new(selected.locator()).strip_prefix(root.path()) else {
                continue;
            };
            let parts = relative.components().collect::<Vec<_>>();
            let [Component::Normal(child), Component::Normal(file)] = parts.as_slice() else {
                continue;
            };
            if *file != "SKILL.md" {
                continue;
            }
            let Some(child) = child.to_str() else {
                continue;
            };
            root.open().map_err(|error| {
                reject(
                    crate::SubmissionRejectionKind::EnvironmentUnavailable,
                    error,
                )
            })?;
            let expected = self.reference(
                root,
                child,
                selected.name(),
                selected.catalog_generation(),
                selected.entry_revision(),
            );
            if &expected != selected {
                return Err(reject(
                    crate::SubmissionRejectionKind::StaleReference,
                    "skill source identity or scope changed",
                ));
            }
            return Ok((root, child.to_owned()));
        }
        Err(reject(
            crate::SubmissionRejectionKind::Unauthorized,
            "skill is not an immediate child of a configured source",
        ))
    }

    pub(super) fn discover(
        &self,
        generation: u64,
        cancelled: &AtomicBool,
    ) -> Result<(Vec<SkillReferenceCandidate>, SkillReferenceSearchStatus), String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut candidates = Vec::new();
        let mut visited = 0;
        let mut bytes_left = MAX_SCAN_BYTES;
        let mut unavailable = Vec::new();
        for root in &self.roots {
            let descriptor = match root.open() {
                Ok(descriptor) => descriptor,
                Err(error) => {
                    unavailable.push(format!("{}: {error}", root.path().display()));
                    continue;
                },
            };
            for entry in Dir::read_from(&descriptor).map_err(|error| error.to_string())? {
                if cancelled.load(Ordering::Acquire) {
                    return Err("skill discovery cancelled".into());
                }
                if Instant::now() >= deadline {
                    return Ok((
                        candidates,
                        SkillReferenceSearchStatus::Incomplete(
                            "skill discovery deadline exceeded".into(),
                        ),
                    ));
                }
                let entry = entry.map_err(|error| error.to_string())?;
                if matches!(entry.file_name().to_bytes(), b"." | b"..") {
                    continue;
                }
                visited += 1;
                if visited > MAX_ENTRIES || bytes_left == 0 {
                    return Ok((
                        candidates,
                        SkillReferenceSearchStatus::Incomplete(
                            "skill discovery budget exceeded".into(),
                        ),
                    ));
                }
                if entry.file_type() == FileType::RegularFile {
                    continue;
                }
                let Ok(child) = entry.file_name().to_str() else {
                    return Ok((
                        candidates,
                        SkillReferenceSearchStatus::Incomplete("non-UTF-8 skill directory".into()),
                    ));
                };
                let snapshot = read_snapshot(root, child, bytes_left);
                let (name, description, revision, availability) = match snapshot {
                    Ok(snapshot) => {
                        bytes_left = bytes_left.saturating_sub(snapshot.text.len());
                        (
                            snapshot.name,
                            snapshot.description,
                            snapshot.revision,
                            snapshot.availability,
                        )
                    },
                    Err(error) => {
                        if bytes_left < ResolvedSkill::MAX_INSTRUCTION_BYTES
                            && error.kind() == crate::SubmissionRejectionKind::OverBudget
                        {
                            return Ok((
                                candidates,
                                SkillReferenceSearchStatus::Incomplete(
                                    "skill discovery byte budget exceeded".into(),
                                ),
                            ));
                        }
                        // A failed read can consume the complete per-file allowance.
                        bytes_left =
                            bytes_left.saturating_sub(ResolvedSkill::MAX_INSTRUCTION_BYTES + 1);
                        (
                            child.to_owned(),
                            String::new(),
                            "unavailable".to_owned(),
                            SkillAvailability::Disabled(error.message().to_owned()),
                        )
                    },
                };
                let reference = self.reference(root, child, &name, generation, &revision);
                candidates.push(SkillReferenceCandidate::new(
                    reference,
                    name,
                    description,
                    availability,
                ));
            }
        }
        let status = if unavailable.is_empty() {
            SkillReferenceSearchStatus::Complete
        } else {
            SkillReferenceSearchStatus::Incomplete(unavailable.join("; "))
        };
        Ok((candidates, status))
    }
}
