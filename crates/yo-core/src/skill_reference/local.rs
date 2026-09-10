//! Explicit local skill sources shared by execution hosts, without backend policy.

use std::{
    ffi::OsStr,
    fmt::Write as _,
    fs::File,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use rustix::{
    fd::OwnedFd,
    fs::{Dir, FileType, Mode, OFlags, open, openat},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{
    SkillAvailability, SkillReference, SkillReferenceCandidate, SkillReferenceProvider,
    SkillReferenceProviderPoll, SkillReferenceScope, SkillReferenceSearchRequest,
    SkillReferenceSearchStatus, SkillReferenceSearchUpdate, search_skill_reference_candidates,
};
use crate::{
    InputAdmissionHost, InputReference, LocalWorkspaceInputAdmission, ResolvedSkill,
    SubmissionRejection, SubmissionRejectionKind, UserInput, WorkspaceHostId, readiness::Readiness,
};

const MAX_ROOTS: usize = 16;
const MAX_ENTRIES: usize = 512;
const MAX_SCAN_BYTES: usize = 16 * 1024 * 1024;
const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
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
}

struct Root {
    path: PathBuf,
    scope: SkillReferenceScope,
    identity: Mutex<Option<(u64, u64)>>,
}

struct Catalog {
    roots: Vec<Root>,
    environment: String,
}

impl Catalog {
    fn new(roots: Vec<LocalSkillRoot>, host: WorkspaceHostId) -> Result<Self, String> {
        if roots.len() > MAX_ROOTS {
            return Err("at most 16 skill roots are supported".into());
        }
        let mut pinned = Vec::new();
        for root in roots {
            let path = root.path.components().collect::<PathBuf>();
            if path.to_str().is_none()
                || path
                    .components()
                    .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
            {
                return Err(
                    "local skill roots require UTF-8 paths without parent components".into(),
                );
            }
            if pinned.iter().any(|old: &Root| old.path == path) {
                return Err("duplicate local skill root".into());
            }
            let root = Root {
                path,
                scope: root.scope,
                identity: Mutex::new(None),
            };
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

    fn reference(
        &self,
        root: &Root,
        child: &str,
        name: &str,
        generation: u64,
        revision: &str,
    ) -> SkillReference {
        let locator = root.path.join(child).join("SKILL.md");
        let root_identity = *root
            .identity
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let identity_bytes = serde_json::to_vec(&(
            &self.environment,
            &root.path,
            root_identity,
            format!("{:?}", root.scope),
            child,
        ))
        .expect("local skill identity consists of serializable strings and numbers");
        let identity = format!("local-skill:{}", digest(&identity_bytes));
        SkillReference::new(
            identity,
            &self.environment,
            locator.to_string_lossy(),
            name,
            root.scope,
            generation,
            revision,
        )
    }

    fn select<'a>(
        &'a self,
        selected: &SkillReference,
    ) -> Result<(&'a Root, String), SubmissionRejection> {
        if selected.execution_environment_identity() != self.environment {
            return Err(reject(
                SubmissionRejectionKind::EnvironmentUnavailable,
                "skill belongs to another execution host",
            ));
        }
        for root in &self.roots {
            let Ok(relative) = Path::new(selected.locator()).strip_prefix(&root.path) else {
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
            root.open()
                .map_err(|error| reject(SubmissionRejectionKind::EnvironmentUnavailable, error))?;
            let expected = self.reference(
                root,
                child,
                selected.name(),
                selected.catalog_generation(),
                selected.entry_revision(),
            );
            if &expected != selected {
                return Err(reject(
                    SubmissionRejectionKind::StaleReference,
                    "skill source identity or scope changed",
                ));
            }
            return Ok((root, child.to_owned()));
        }
        Err(reject(
            SubmissionRejectionKind::Unauthorized,
            "skill is not an immediate child of a configured source",
        ))
    }

    fn discover(
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
                    unavailable.push(format!("{}: {error}", root.path.display()));
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
                            && error.kind() == SubmissionRejectionKind::OverBudget
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

impl Root {
    fn open(&self) -> Result<OwnedFd, String> {
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

#[derive(Default, Deserialize)]
struct Metadata {
    name: Option<String>,
    description: Option<String>,
    enabled: Option<bool>,
    #[serde(rename = "user-invocable")]
    user_invocable: Option<bool>,
}

struct Snapshot {
    text: String,
    name: String,
    description: String,
    revision: String,
    availability: SkillAvailability,
}

fn read_snapshot(root: &Root, child: &str, budget: usize) -> Result<Snapshot, SubmissionRejection> {
    let unavailable =
        |error: String| reject(SubmissionRejectionKind::RequiredAssetUnavailable, error);
    let parent = root.open().map_err(unavailable)?;
    let directory =
        openat(parent, OsStr::new(child), DIRECTORY_FLAGS, Mode::empty()).map_err(|error| {
            unavailable(format!(
                "skill directory unavailable (symlinks are not supported): {error}"
            ))
        })?;
    let descriptor = openat(
        directory,
        "SKILL.md",
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| {
        unavailable(format!(
            "SKILL.md unavailable (symlinks are not supported): {error}"
        ))
    })?;
    let file = File::from(descriptor);
    let metadata = file
        .metadata()
        .map_err(|error| unavailable(error.to_string()))?;
    if !metadata.is_file() {
        return Err(unavailable("SKILL.md is not a regular file".into()));
    }
    let limit = ResolvedSkill::MAX_INSTRUCTION_BYTES.min(budget);
    if metadata.len() > limit as u64 {
        return Err(reject(
            SubmissionRejectionKind::OverBudget,
            "skill instruction or scan byte limit exceeded",
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| unavailable(error.to_string()))?;
    if bytes.len() > limit {
        return Err(reject(
            SubmissionRejectionKind::OverBudget,
            "skill instruction or scan byte limit exceeded",
        ));
    }
    root.open()
        .map_err(|error| reject(SubmissionRejectionKind::EnvironmentUnavailable, error))?;
    let text = String::from_utf8(bytes).map_err(|_| unavailable("SKILL.md is not UTF-8".into()))?;
    let metadata = if text.starts_with("---\n") || text.starts_with("---\r\n") {
        let start = text.find('\n').expect("frontmatter opening has newline") + 1;
        let mut offset = start;
        let mut end = None;
        for line in text[start..].split_inclusive('\n') {
            if line.trim_end_matches(['\r', '\n']) == "---" {
                end = Some(offset);
                break;
            }
            offset += line.len();
        }
        let header =
            &text[start..end.ok_or_else(|| unavailable("unclosed skill frontmatter".into()))?];
        if header.len() > 16 * 1024 {
            return Err(unavailable("skill frontmatter exceeds 16 KiB".into()));
        }
        yo_yaml::from_str_with_limits::<Metadata>(
            header,
            yo_yaml::ParseLimits::with_max_total_scalar_bytes(16 * 1024),
        )
        .map_err(|error| unavailable(format!("invalid skill frontmatter: {error}")))?
    } else {
        Metadata::default()
    };
    let name = metadata.name.unwrap_or_else(|| child.to_owned());
    let description = metadata.description.unwrap_or_default();
    if name.is_empty()
        || name.len() > 128
        || name.chars().any(|ch| ch.is_whitespace() || ch.is_control())
        || description.len() > 4096
        || description
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\t' | '\r' | '\n'))
    {
        return Err(unavailable("invalid skill name or description".into()));
    }
    let revision = format!("sha256:{}", digest(text.as_bytes()));
    let availability = if !metadata.enabled.unwrap_or(true) {
        SkillAvailability::Disabled("disabled by local skill metadata".into())
    } else if !metadata.user_invocable.unwrap_or(true) {
        SkillAvailability::Disabled("local skill is not user-invocable".into())
    } else if text.trim().is_empty() {
        SkillAvailability::Disabled("skill instructions are blank".into())
    } else {
        SkillAvailability::Enabled
    };
    Ok(Snapshot {
        text,
        name,
        description,
        revision,
        availability,
    })
}

fn digest(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

fn reject(kind: SubmissionRejectionKind, message: impl Into<String>) -> SubmissionRejection {
    SubmissionRejection::new(kind, message)
}

/// Local workspace and explicit-skill admission; no model/provider-specific interpretation.
pub struct LocalSkillInputAdmission {
    workspace: LocalWorkspaceInputAdmission,
    catalog: Catalog,
}

impl LocalSkillInputAdmission {
    /// Binds only explicitly supplied source roots to one local execution host.
    pub fn new(
        workspace: &Path,
        roots: Vec<LocalSkillRoot>,
        host: WorkspaceHostId,
    ) -> Result<Self, String> {
        Ok(Self {
            workspace: LocalWorkspaceInputAdmission::new(workspace, host)?,
            catalog: Catalog::new(roots, host)?,
        })
    }
}

impl InputAdmissionHost for LocalSkillInputAdmission {
    fn validate(&self, input: &UserInput) -> Result<(), SubmissionRejection> {
        if input.references().len() > 128 {
            return Err(reject(
                SubmissionRejectionKind::OverBudget,
                "at most 128 input references are supported",
            ));
        }
        let workspace = UserInput::with_references(
            input.as_str(),
            input
                .references()
                .iter()
                .filter(|reference| reference.workspace_reference().is_some())
                .cloned()
                .collect(),
        )
        .map_err(|error| reject(SubmissionRejectionKind::InvalidReference, error.to_string()))?;
        self.workspace.validate(&workspace)?;
        if let Some(selected) = input
            .references()
            .iter()
            .find_map(InputReference::skill_reference)
        {
            self.catalog.select(selected)?;
        }
        Ok(())
    }

    fn prepare(&self, input: &UserInput) -> Result<Option<ResolvedSkill>, SubmissionRejection> {
        self.validate(input)?;
        let Some(selected) = input
            .references()
            .iter()
            .find_map(InputReference::skill_reference)
        else {
            return Ok(None);
        };
        let (root, child) = self.catalog.select(selected)?;
        let snapshot = read_snapshot(root, &child, ResolvedSkill::MAX_INSTRUCTION_BYTES)?;
        let expected = self.catalog.reference(
            root,
            &child,
            &snapshot.name,
            selected.catalog_generation(),
            &snapshot.revision,
        );
        if &expected != selected {
            return Err(reject(
                SubmissionRejectionKind::StaleReference,
                "selected skill changed; select it again",
            ));
        }
        if let SkillAvailability::Disabled(reason) = snapshot.availability {
            return Err(reject(SubmissionRejectionKind::Unauthorized, reason));
        }
        ResolvedSkill::new(selected.clone(), snapshot.text)
            .map(Some)
            .map_err(|error| {
                reject(
                    SubmissionRejectionKind::RequiredAssetUnavailable,
                    error.to_string(),
                )
            })
    }
}

#[derive(Default)]
struct Mailbox {
    request: Option<SkillReferenceSearchRequest>,
    refresh: bool,
    update: Option<SkillReferenceSearchUpdate>,
}

/// Nonblocking catalog connection with coalesced requests and a cancellable owned worker.
pub struct LocalSkillReferenceProvider {
    shared: Arc<(Mutex<Mailbox>, Condvar)>,
    stopped: Arc<AtomicBool>,
    readiness: Arc<Readiness>,
    worker: Option<JoinHandle<()>>,
}

impl LocalSkillReferenceProvider {
    /// Starts bounded discovery for explicitly configured roots.
    pub fn start(roots: Vec<LocalSkillRoot>, host: WorkspaceHostId) -> Result<Self, String> {
        let catalog = Catalog::new(roots, host)?;
        let shared = Arc::new((Mutex::new(Mailbox::default()), Condvar::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let readiness = Arc::new(Readiness::new());
        let worker_shared = Arc::clone(&shared);
        let worker_stopped = Arc::clone(&stopped);
        let worker_readiness = Arc::clone(&readiness);
        let worker = thread::Builder::new()
            .name("yo-local-skill-catalog".into())
            .spawn(move || {
                let (lock, changed) = &*worker_shared;
                let mut cache = None;
                let mut generation = 0_u64;
                loop {
                    let (request, refresh) = {
                        let mut mailbox = lock.lock().unwrap_or_else(|error| error.into_inner());
                        while mailbox.request.is_none() && !worker_stopped.load(Ordering::Acquire) {
                            mailbox = changed
                                .wait(mailbox)
                                .unwrap_or_else(|error| error.into_inner());
                        }
                        if worker_stopped.load(Ordering::Acquire) {
                            break;
                        }
                        (
                            mailbox.request.take().expect("request checked"),
                            std::mem::take(&mut mailbox.refresh),
                        )
                    };
                    if refresh || cache.is_none() {
                        generation = generation.saturating_add(1);
                        cache = Some(catalog.discover(generation, &worker_stopped));
                    }
                    if worker_stopped.load(Ordering::Acquire) {
                        break;
                    }
                    let (candidates, status) =
                        match cache.as_ref().expect("first request initializes cache") {
                            Ok((candidates, status)) => (
                                search_skill_reference_candidates(candidates, request.query()),
                                status.clone(),
                            ),
                            Err(error) => (
                                Vec::new(),
                                SkillReferenceSearchStatus::Failed(error.clone()),
                            ),
                        };
                    lock.lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .update = Some(SkillReferenceSearchUpdate::final_result(
                        &request, status, candidates,
                    ));
                    worker_readiness.notify();
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            shared,
            stopped,
            readiness,
            worker: Some(worker),
        })
    }
}

impl SkillReferenceProvider for LocalSkillReferenceProvider {
    fn search(&mut self, request: SkillReferenceSearchRequest) -> Result<(), String> {
        let mut mailbox = self
            .shared
            .0
            .lock()
            .map_err(|_| "skill worker mailbox poisoned")?;
        mailbox.refresh |= request.refresh_catalog();
        mailbox.request = Some(request);
        self.shared.1.notify_one();
        Ok(())
    }
    fn poll(&mut self) -> Result<SkillReferenceProviderPoll, String> {
        Ok(self
            .shared
            .0
            .lock()
            .map_err(|_| "skill worker mailbox poisoned")?
            .update
            .take()
            .map_or(
                SkillReferenceProviderPoll::Pending,
                SkillReferenceProviderPoll::Update,
            ))
    }
    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<()> {
        if self
            .shared
            .0
            .lock()
            .map_or(true, |mailbox| mailbox.update.is_some())
        {
            return Poll::Ready(());
        }
        self.readiness.poll(context)
    }
}

impl Drop for LocalSkillReferenceProvider {
    fn drop(&mut self) {
        // Pair the stop predicate with the wait mutex to avoid a lost shutdown wakeup.
        let mailbox = self
            .shared
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.stopped.store(true, Ordering::Release);
        self.shared.1.notify_one();
        drop(mailbox);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests;
