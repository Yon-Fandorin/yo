//! Local execution-workspace discovery kept outside the terminal UI thread.

use std::{
    ffi::OsStr,
    os::unix::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    task::{Context, Poll},
    thread,
    time::{Duration, Instant},
};

use rustix::{
    fd::OwnedFd,
    fs::{AtFlags, Dir, FileType, Mode, OFlags, fstat, open, openat, statat},
    io::Errno,
};

#[cfg(test)]
use self::{
    filesystem::discover_entries,
    git::{classify_git_workspace, is_git_workspace},
    ranking::rank,
};
use self::{git::git_command, inventory::build_inventory, ranking::search};
use super::{
    WorkspaceReferenceProvider, WorkspaceReferenceProviderPoll, WorkspaceReferenceSearchRequest,
    WorkspaceReferenceSearchStatus, WorkspaceReferenceSearchUpdate,
};
use crate::{
    InputAdmissionHost, InputReference, SubmissionRejection, SubmissionRejectionKind, UserInput,
    WorkspaceHostId, WorkspaceReferenceKind,
};

mod filesystem;
mod git;
mod inventory;
mod ranking;

/// Shared bounds for filesystem and tracked-path discovery in one inventory.
struct DiscoveryBudget {
    entries_left: usize,
    bytes_left: usize,
    deadline: Instant,
    exhausted: bool,
}

impl Default for DiscoveryBudget {
    fn default() -> Self {
        Self {
            entries_left: 32_768,
            bytes_left: 16 * 1024 * 1024,
            deadline: Instant::now() + Duration::from_secs(30),
            exhausted: false,
        }
    }
}

impl DiscoveryBudget {
    fn admit(&mut self, path: &Path) -> bool {
        if self.exhausted
            || self.entries_left == 0
            || path.as_os_str().len() > self.bytes_left
            || Instant::now() >= self.deadline
        {
            self.exhausted = true;
            return false;
        }
        self.entries_left -= 1;
        self.bytes_left -= path.as_os_str().len();
        true
    }
}

pub struct LocalWorkspaceReferenceProvider {
    requests: Sender<WorkspaceReferenceSearchRequest>,
    updates: crate::readiness::ReadyReceiver<WorkspaceReferenceSearchUpdate>,
}

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
        let root = std::fs::canonicalize(root).map_err(|error| error.to_string())?;
        let descriptor = pin_root(&root).map_err(|error| {
            format!("workspace root {} is unavailable: {error}", root.display())
        })?;
        let root_identity = inventory::root_identity(&root, &descriptor)?;
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
        let current_root = inventory::root_identity(&self.root, &root)
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
            let expected = inventory::reference(
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
        let current = inventory::root_identity(&self.root, &descriptor)
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

fn pin_root(root: &Path) -> Result<OwnedFd, Errno> {
    let filesystem = open(
        "/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    pin_directory(
        &filesystem,
        root.strip_prefix("/").map_err(|_| Errno::INVAL)?,
    )
}

fn pin_directory(root: &OwnedFd, relative: &Path) -> Result<OwnedFd, Errno> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut directory = openat(root, ".", flags, Mode::empty())?;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(Errno::INVAL);
        };
        directory = openat(&directory, name, flags, Mode::empty())?;
    }
    Ok(directory)
}

impl LocalWorkspaceReferenceProvider {
    /// Execution-host discovery command for local output navigation, never run by rendering.
    pub fn output_link_command(root: &Path) -> std::process::Command {
        let mut command = git_command(root);
        command.args([
            "-c",
            "core.fsmonitor=false",
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ]);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("GIT_") {
                command.env_remove(key);
            }
        }
        command
    }

    /// Validates a fresh file inventory for this canonical local execution root.
    /// None permits bounded plain-directory discovery only when no Git marker exists.
    /// Missing, changed, symlinked and outside-root candidates never become links.
    pub fn output_links(
        root: &Path,
        candidates: Option<&[u8]>,
        cancelled: impl Fn() -> bool,
    ) -> Result<std::collections::BTreeMap<String, PathBuf>, String> {
        use std::{
            collections::BTreeMap,
            time::{Duration, Instant},
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        if cancelled() {
            return Err("link discovery cancelled".into());
        }
        if std::fs::canonicalize(root).ok().as_deref() != Some(root) {
            return Err("workspace root changed".into());
        }
        let mut paths = Vec::new();
        if let Some(candidates) = candidates {
            if candidates.len() > 4 * 1024 * 1024 {
                return Err("link inventory exceeds byte limit".into());
            }
            for candidate in candidates
                .split(|byte| *byte == 0)
                .filter(|path| !path.is_empty())
            {
                paths.push(PathBuf::from(
                    std::str::from_utf8(candidate).map_err(|_| "non-UTF-8 link inventory")?,
                ));
                if paths.len() > 8192 {
                    return Err("link inventory exceeds entry limit".into());
                }
            }
        } else {
            for ancestor in root.ancestors() {
                match std::fs::symlink_metadata(ancestor.join(".git")) {
                    Ok(_) => return Err("Git discovery unavailable".into()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                    Err(_) => return Err("workspace discovery unavailable".into()),
                }
            }
            let filesystem = open(
                "/",
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| error.to_string())?;
            let root_directory = Self::output_directory(
                &filesystem,
                root.strip_prefix("/")
                    .map_err(|_| "non-absolute workspace root")?,
            )?;
            let mut frontier = vec![PathBuf::new()];
            let mut visited = 0;
            while let Some(directory) = frontier.pop() {
                if cancelled() || Instant::now() >= deadline {
                    return Err("link discovery cancelled or expired".into());
                }
                // Resolve each queued component against pinned descriptors: a replaced
                // directory must never redirect enumeration through a symlink.
                let descriptor = Self::output_directory(&root_directory, &directory)?;
                for entry in Dir::read_from(&descriptor).map_err(|error| error.to_string())? {
                    let entry = entry.map_err(|error| error.to_string())?;
                    let name = entry.file_name().to_bytes();
                    if matches!(name, b"." | b"..") {
                        continue;
                    }
                    if cancelled() || Instant::now() >= deadline {
                        return Err("link discovery cancelled or expired".into());
                    }
                    visited += 1;
                    if visited > 8192 {
                        return Err("link inventory exceeds entry limit".into());
                    }
                    if name == b".git" {
                        continue;
                    }
                    let kind = if entry.file_type() == FileType::Unknown {
                        let metadata =
                            statat(&descriptor, entry.file_name(), AtFlags::SYMLINK_NOFOLLOW)
                                .map_err(|error| error.to_string())?;
                        FileType::from_raw_mode(metadata.st_mode)
                    } else {
                        entry.file_type()
                    };
                    let relative = directory.join(OsStr::from_bytes(name));
                    if kind == FileType::Directory {
                        frontier.push(relative);
                    } else if kind == FileType::RegularFile {
                        paths.push(relative);
                    }
                }
            }
        }
        let mut links = BTreeMap::new();
        for relative in paths {
            if cancelled() || Instant::now() >= deadline {
                return Err("link discovery cancelled or expired".into());
            }
            let components = relative.components().collect::<Vec<_>>();
            if components.is_empty()
                || components.iter().any(
                    |component| !matches!(component, Component::Normal(name) if *name != ".git"),
                )
            {
                continue;
            }
            let mut path = root.to_owned();
            let mut valid = true;
            for (index, component) in components.iter().enumerate() {
                path.push(component);
                let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                    valid = false;
                    break;
                };
                if metadata.file_type().is_symlink()
                    || (index + 1 == components.len() && !metadata.is_file())
                    || (index + 1 < components.len() && !metadata.is_dir())
                {
                    valid = false;
                    break;
                }
            }
            if !valid {
                continue;
            }
            let Ok(canonical) = std::fs::canonicalize(&path) else {
                continue;
            };
            if canonical != path || !canonical.starts_with(root) {
                continue;
            }
            let Some(relative) = relative.to_str() else {
                continue;
            };
            links.insert(relative.to_owned(), canonical);
        }
        if std::fs::canonicalize(root).ok().as_deref() != Some(root) {
            return Err("workspace root changed".into());
        }
        Ok(links)
    }

    fn output_directory(root: &OwnedFd, relative: &Path) -> Result<OwnedFd, String> {
        pin_directory(root, relative).map_err(|error| error.to_string())
    }

    pub fn start(root: &Path, workspace_host_id: WorkspaceHostId) -> Result<Self, std::io::Error> {
        let (request_tx, request_rx) = mpsc::channel();
        let (update_tx, update_rx) = mpsc::channel();
        let readiness = Arc::new(crate::readiness::Readiness::new());
        let worker_readiness = Arc::clone(&readiness);
        let root = std::fs::canonicalize(root)?;
        thread::Builder::new()
            .name("yo-workspace-search".to_owned())
            .spawn(move || {
                worker(
                    root,
                    workspace_host_id,
                    request_rx,
                    update_tx,
                    &worker_readiness,
                );
                worker_readiness.notify();
            })?;
        Ok(Self {
            requests: request_tx,
            updates: crate::readiness::ReadyReceiver::new(update_rx, readiness),
        })
    }
}

impl WorkspaceReferenceProvider for LocalWorkspaceReferenceProvider {
    fn search(&mut self, request: WorkspaceReferenceSearchRequest) -> Result<(), String> {
        self.requests
            .send(request)
            .map_err(|_| "workspace search worker closed".to_owned())
    }

    fn poll(&mut self) -> Result<WorkspaceReferenceProviderPoll, String> {
        match self.updates.try_recv() {
            Ok(update) => Ok(WorkspaceReferenceProviderPoll::Update(update)),
            Err(TryRecvError::Empty) => Ok(WorkspaceReferenceProviderPoll::Pending),
            Err(TryRecvError::Disconnected) => Err("workspace search worker closed".to_owned()),
        }
    }

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<()> {
        self.updates.poll_ready(context)
    }
}

fn worker(
    root: PathBuf,
    workspace_host_id: WorkspaceHostId,
    requests: Receiver<WorkspaceReferenceSearchRequest>,
    updates: Sender<WorkspaceReferenceSearchUpdate>,
    readiness: &crate::readiness::Readiness,
) {
    let inventory = build_inventory(&root, workspace_host_id);
    while let Ok(mut request) = requests.recv() {
        while let Ok(newer) = requests.try_recv() {
            request = newer;
        }
        let update = match &inventory {
            Ok(inventory) => WorkspaceReferenceSearchUpdate::final_result(
                &request,
                inventory.status.clone(),
                search(&inventory.entries, request.query()),
            ),
            Err(error) => WorkspaceReferenceSearchUpdate::final_result(
                &request,
                WorkspaceReferenceSearchStatus::Failed(error.clone()),
                Vec::new(),
            ),
        };
        if updates.send(update).is_err() {
            break;
        }
        readiness.notify();
    }
}

#[cfg(test)]
mod tests;
