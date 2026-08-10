//! Concrete workspace-local tools for the Yo-managed model backend.

use std::{
    ffi::OsString,
    fs::{File, OpenOptions},
    io::Read,
    os::{
        fd::{AsFd, OwnedFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
            process::CommandExt,
        },
    },
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use nix::{
    dir::Dir,
    fcntl::{AtFlags, FcntlArg, OFlag, fcntl, openat},
    sys::{
        signal::{Signal, kill},
        stat::{Mode, SFlag, fstat, fstatat},
    },
    unistd::Pid,
};
use serde_json::{Value, json};
use yo_core::{
    TOOL_SCHEMA_DIALECT, ToolApprovalRequirement, ToolDefinition, ToolEffect, ToolExecution,
    ToolExecutionError, ToolExecutionHost, ToolExecutionOutcome, ToolExecutionPoll,
    ToolExecutionRequest, ToolExecutionResult, ToolId, ToolRegistry, ToolSemanticAdmission,
    ToolSemanticAdmissionError,
};

const HOST_IDENTITY: &str = "yo.local-workspace-tools/v1";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const PROCESS_CLEANUP_GRACE: Duration = Duration::from_secs(1);
const MAX_LIST_ENTRIES: usize = 100_000;

pub(crate) fn registry() -> Result<ToolRegistry, ToolExecutionError> {
    ToolRegistry::new([
        definition(
            "read-file",
            "read_file",
            "Read one UTF-8 file inside the current workspace.",
            path_schema(),
            ToolEffect::ReadOnly,
            ToolApprovalRequirement::Automatic,
        )?,
        definition(
            "list-files",
            "list_files",
            "List files recursively below one directory inside the current workspace.",
            path_schema(),
            ToolEffect::ReadOnly,
            ToolApprovalRequirement::Automatic,
        )?,
        definition(
            "run-command",
            "run_command",
            "Run one shell command in the current workspace after explicit user approval.",
            json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command to run from the workspace root"
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
            ToolEffect::Process,
            ToolApprovalRequirement::Required,
        )?,
    ])
    .map_err(|error| ToolExecutionError::new(error.to_string()))
}

fn definition(
    id: &str,
    wire_name: &str,
    description: &str,
    schema: Value,
    effect: ToolEffect,
    approval: ToolApprovalRequirement,
) -> Result<ToolDefinition, ToolExecutionError> {
    ToolDefinition::new(
        ToolId::new(id).map_err(|error| ToolExecutionError::new(error.to_string()))?,
        wire_name,
        description,
        TOOL_SCHEMA_DIALECT,
        schema,
        effect,
        approval,
    )
    .map_err(|error| ToolExecutionError::new(error.to_string()))
}

fn path_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Workspace-relative path"
            }
        },
        "required": ["path"],
        "additionalProperties": false
    })
}

pub(crate) struct LocalSemanticAdmission {
    credentials: yo_core::CredentialStore,
}

impl LocalSemanticAdmission {
    pub(crate) const fn new(credentials: yo_core::CredentialStore) -> Self {
        Self { credentials }
    }

    fn admit(&self, value: &str) -> Result<String, ToolSemanticAdmissionError> {
        if self.credentials.contains_secret_material(value) {
            Err(ToolSemanticAdmissionError::new(
                "tool semantic value contains prohibited credential material",
            ))
        } else {
            Ok(value.to_owned())
        }
    }
}

impl ToolSemanticAdmission for LocalSemanticAdmission {
    fn admit_arguments(
        &self,
        _definition: &ToolDefinition,
        validated_argument_bytes: &str,
    ) -> Result<String, ToolSemanticAdmissionError> {
        self.admit(validated_argument_bytes)
    }

    fn admit_output(
        &self,
        _definition: &ToolDefinition,
        bounded_output: &str,
    ) -> Result<String, ToolSemanticAdmissionError> {
        self.admit(bounded_output)
    }
}

pub(crate) struct LocalToolHost {
    workspace: PathBuf,
    workspace_directory: File,
    denied_credential: Option<FileIdentity>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(target_vendor = "apple")]
const fn normalize_device_id(device: libc::dev_t) -> u64 {
    device as u64
}

#[cfg(not(target_vendor = "apple"))]
const fn normalize_device_id(device: libc::dev_t) -> u64 {
    device
}

impl LocalToolHost {
    pub(crate) fn new(
        workspace: &Path,
        credential_path: &Path,
    ) -> Result<Self, ToolExecutionError> {
        let workspace_directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(workspace)
            .map_err(|_| ToolExecutionError::new("workspace cannot be opened safely"))?;
        let workspace = workspace
            .canonicalize()
            .map_err(|_| ToolExecutionError::new("workspace cannot be canonicalized"))?;
        let denied_credential = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(credential_path)
            .ok()
            .and_then(|file| file.metadata().ok())
            .map(|metadata| FileIdentity {
                device: metadata.dev(),
                inode: metadata.ino(),
            });
        Ok(Self {
            workspace,
            workspace_directory,
            denied_credential,
        })
    }

    fn path_components(value: &str) -> Result<Vec<OsString>, ToolExecutionError> {
        if value.is_empty()
            || Path::new(value).is_absolute()
            || Path::new(value).components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(ToolExecutionError::new(
                "tool path must be a non-empty workspace-relative path without parent traversal",
            ));
        }
        Ok(Path::new(value)
            .components()
            .filter_map(|component| match component {
                Component::Normal(value) => Some(value.to_owned()),
                Component::CurDir => None,
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    unreachable!("invalid components were rejected")
                },
            })
            .collect())
    }

    fn open_file(&self, value: &str) -> Result<File, ToolExecutionError> {
        let components = Self::path_components(value)?;
        let descriptor = open_beneath(
            &self.workspace_directory,
            &components,
            OFlag::O_RDONLY | OFlag::O_NONBLOCK,
        )?;
        let metadata =
            fstat(&descriptor).map_err(|_| ToolExecutionError::new("tool path is unavailable"))?;
        if !SFlag::from_bits_truncate(metadata.st_mode).contains(SFlag::S_IFREG) {
            return Err(ToolExecutionError::new("read_file requires a regular file"));
        }
        let device = normalize_device_id(metadata.st_dev);
        if self.denied_credential
            == Some(FileIdentity {
                device,
                inode: metadata.st_ino,
            })
        {
            return Err(ToolExecutionError::new(
                "the model credential file is not readable through local tools",
            ));
        }
        Ok(File::from(descriptor))
    }

    fn open_directory(&self, value: &str) -> Result<(Dir, PathBuf), ToolExecutionError> {
        let components = Self::path_components(value)?;
        let relative = components.iter().collect();
        let descriptor = open_beneath(
            &self.workspace_directory,
            &components,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY,
        )?;
        let directory = Dir::from_fd(descriptor)
            .map_err(|_| ToolExecutionError::new("list_files requires a directory"))?;
        Ok((directory, relative))
    }
}

fn open_beneath(
    workspace: &File,
    components: &[OsString],
    final_flags: OFlag,
) -> Result<OwnedFd, ToolExecutionError> {
    let mut current: OwnedFd = workspace
        .try_clone()
        .map_err(|_| ToolExecutionError::new("workspace handle is unavailable"))?
        .into();
    for (index, component) in components.iter().enumerate() {
        let is_final = index + 1 == components.len();
        let flags = OFlag::O_CLOEXEC
            | OFlag::O_NOFOLLOW
            | if is_final {
                final_flags
            } else {
                OFlag::O_RDONLY | OFlag::O_DIRECTORY
            };
        current = openat(&current, component.as_os_str(), flags, Mode::empty())
            .map_err(|_| ToolExecutionError::new("tool path is unavailable"))?;
    }
    Ok(current)
}

impl ToolExecutionHost for LocalToolHost {
    fn identity(&self) -> &str {
        HOST_IDENTITY
    }

    fn is_available(&self, tool: &ToolId) -> bool {
        matches!(tool.as_str(), "read-file" | "list-files" | "run-command")
    }

    fn start(
        &mut self,
        request: ToolExecutionRequest,
    ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
        let maximum_output_bytes = request.maximum_output_bytes;
        match request.call.definition().id().as_str() {
            "read-file" => {
                let path = string_argument(request.call.arguments(), "path")?;
                let file = self.open_file(path)?;
                Ok(Box::new(ThreadExecution::spawn(move |cancelled| {
                    read_file(file, maximum_output_bytes, &cancelled)
                })))
            },
            "list-files" => {
                let path = string_argument(request.call.arguments(), "path")?;
                let (directory, relative) = self.open_directory(path)?;
                Ok(Box::new(ThreadExecution::spawn(move |cancelled| {
                    list_files(directory, relative, maximum_output_bytes, &cancelled)
                })))
            },
            "run-command" => {
                let command = string_argument(request.call.arguments(), "command")?.to_owned();
                Ok(Box::new(CommandExecution::spawn(
                    self.workspace.clone(),
                    command,
                    maximum_output_bytes,
                )?))
            },
            _ => Err(ToolExecutionError::new("unknown local tool")),
        }
    }

    fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
        Ok(())
    }
}

fn string_argument<'a>(value: &'a Value, name: &str) -> Result<&'a str, ToolExecutionError> {
    value
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolExecutionError::new("validated local tool argument is unavailable"))
}

fn read_file(file: File, limit: usize, cancelled: &AtomicBool) -> ToolExecutionResult {
    if cancelled.load(Ordering::Acquire) {
        return interrupted();
    }
    let result = read_bounded(file, limit);
    if cancelled.load(Ordering::Acquire) {
        return interrupted();
    }
    match result {
        Ok((bytes, truncated)) => match String::from_utf8(bytes) {
            Ok(output) => completed(output, truncated),
            Err(_) => failed("read_file supports UTF-8 text files only"),
        },
        Err(_) => failed("read_file failed"),
    }
}

fn list_files(
    root: Dir,
    root_relative: PathBuf,
    limit: usize,
    cancelled: &AtomicBool,
) -> ToolExecutionResult {
    let mut pending = vec![(root, root_relative)];
    let mut output = String::new();
    let mut truncated = false;
    let mut visited_entries = 0_usize;
    'walk: while let Some((mut directory, relative_directory)) = pending.pop() {
        if cancelled.load(Ordering::Acquire) {
            return interrupted();
        }
        let mut names = Vec::new();
        for entry in directory.iter() {
            if cancelled.load(Ordering::Acquire) {
                return interrupted();
            }
            let Ok(entry) = entry else {
                return failed("list_files failed");
            };
            let name = entry.file_name().to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            visited_entries = visited_entries.saturating_add(1);
            if visited_entries > MAX_LIST_ENTRIES {
                truncated = true;
                break 'walk;
            }
            names.push(OsString::from(std::ffi::OsStr::from_bytes(name)));
        }
        names.sort();
        let mut child_directories = Vec::new();
        for name in names {
            let Ok(metadata) = fstatat(&directory, name.as_os_str(), AtFlags::AT_SYMLINK_NOFOLLOW)
            else {
                return failed("list_files failed");
            };
            let file_type = SFlag::from_bits_truncate(metadata.st_mode);
            if file_type.contains(SFlag::S_IFLNK) {
                continue;
            }
            let relative = relative_directory.join(&name);
            if file_type.contains(SFlag::S_IFDIR) {
                if name != ".git" {
                    let Ok(child) = Dir::openat(
                        &directory,
                        name.as_os_str(),
                        OFlag::O_RDONLY | OFlag::O_CLOEXEC | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW,
                        Mode::empty(),
                    ) else {
                        return failed("list_files failed");
                    };
                    child_directories.push((child, relative));
                }
                continue;
            }
            if !file_type.contains(SFlag::S_IFREG) {
                continue;
            }
            let Ok(file) = openat(
                &directory,
                name.as_os_str(),
                OFlag::O_RDONLY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK,
                Mode::empty(),
            ) else {
                return failed("list_files failed");
            };
            let Ok(opened_metadata) = fstat(&file) else {
                return failed("list_files failed");
            };
            if !SFlag::from_bits_truncate(opened_metadata.st_mode).contains(SFlag::S_IFREG) {
                return failed("list_files path changed during traversal");
            }
            let line = format!("{}\n", relative.display());
            if output.len().saturating_add(line.len()) > limit {
                truncated = true;
                break;
            }
            output.push_str(&line);
        }
        pending.extend(child_directories.into_iter().rev());
        if truncated {
            break;
        }
    }
    completed(output, truncated)
}

fn read_bounded(mut reader: impl Read, limit: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut output = Vec::with_capacity(limit.min(64 * 1024));
    let mut chunk = [0_u8; 8 * 1024];
    let target = limit.saturating_add(1);
    while output.len() < target {
        let remaining = target.saturating_sub(output.len());
        let read_len = chunk.len().min(remaining);
        let count = reader.read(&mut chunk[..read_len])?;
        if count == 0 {
            break;
        }
        output.extend_from_slice(&chunk[..count]);
    }
    let truncated = output.len() > limit;
    output.truncate(limit);
    Ok((output, truncated))
}

fn completed(output: String, truncated: bool) -> ToolExecutionResult {
    ToolExecutionResult::new(ToolExecutionOutcome::Completed, output, truncated)
}

fn failed(message: &str) -> ToolExecutionResult {
    ToolExecutionResult::new(ToolExecutionOutcome::Failed, message, false)
}

fn interrupted() -> ToolExecutionResult {
    ToolExecutionResult::new(ToolExecutionOutcome::Interrupted, "interrupted", false)
}

struct ThreadExecution {
    cancelled: Arc<AtomicBool>,
    receiver: Receiver<ToolExecutionResult>,
    worker: Option<JoinHandle<()>>,
    result: Option<ToolExecutionResult>,
}

impl ThreadExecution {
    fn spawn(task: impl FnOnce(Arc<AtomicBool>) -> ToolExecutionResult + Send + 'static) -> Self {
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let _ = sender.send(task(worker_cancelled));
        });
        Self {
            cancelled,
            receiver,
            worker: Some(worker),
            result: None,
        }
    }

    fn poll_result(&mut self) -> Result<ToolExecutionPoll, ToolExecutionError> {
        if self.result.is_some() {
            return Ok(ToolExecutionPoll::Ready);
        }
        match self.receiver.try_recv() {
            Ok(result) => {
                self.result = Some(result);
                Ok(ToolExecutionPoll::Ready)
            },
            Err(TryRecvError::Empty) => Ok(ToolExecutionPoll::Pending),
            Err(TryRecvError::Disconnected) => {
                Err(ToolExecutionError::new("local tool worker stopped"))
            },
        }
    }

    fn join(&mut self) -> Result<(), ToolExecutionError> {
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| ToolExecutionError::new("local tool worker panicked"))?;
        }
        Ok(())
    }
}

impl ToolExecution for ThreadExecution {
    fn poll(&mut self) -> Result<ToolExecutionPoll, ToolExecutionError> {
        self.poll_result()
    }

    fn take_result(&mut self) -> Option<ToolExecutionResult> {
        self.result.take()
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
        self.cancel();
        self.join()
    }
}

struct CommandExecution {
    inner: ThreadExecution,
    child: Arc<Mutex<Option<Child>>>,
}

impl CommandExecution {
    fn spawn(
        workspace: PathBuf,
        command: String,
        maximum_output_bytes: usize,
    ) -> Result<Self, ToolExecutionError> {
        if command.is_empty() || command.chars().any(|character| character == '\0') {
            return Err(ToolExecutionError::new(
                "run_command requires a non-empty command",
            ));
        }
        let child = Arc::new(Mutex::new(None));
        let worker_child = Arc::clone(&child);
        let inner = ThreadExecution::spawn(move |cancelled| {
            run_command(
                &workspace,
                &command,
                maximum_output_bytes,
                &cancelled,
                &worker_child,
            )
        });
        Ok(Self { inner, child })
    }
}

impl ToolExecution for CommandExecution {
    fn poll(&mut self) -> Result<ToolExecutionPoll, ToolExecutionError> {
        self.inner.poll()
    }

    fn take_result(&mut self) -> Option<ToolExecutionResult> {
        self.inner.take_result()
    }

    fn cancel(&self) {
        self.inner.cancel();
        if let Ok(mut child) = self.child.lock()
            && let Some(child) = child.as_mut()
        {
            terminate_process_group(child);
        }
    }

    fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
        self.cancel();
        self.inner.join()
    }
}

fn run_command(
    workspace: &Path,
    command: &str,
    maximum_output_bytes: usize,
    cancelled: &AtomicBool,
    shared_child: &Mutex<Option<Child>>,
) -> ToolExecutionResult {
    let spawned = Command::new("/bin/sh")
        .arg("-lc")
        .arg(command)
        .current_dir(workspace)
        .process_group(0)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut child) = spawned else {
        return failed("run_command could not start");
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let process_group = i32::try_from(child.id()).ok().map(Pid::from_raw);
    let (Some(stdout), Some(stderr)) = (stdout, stderr) else {
        terminate_process_group(&mut child);
        return failed("run_command output pipes are unavailable");
    };
    if set_nonblocking(&stdout).is_err() || set_nonblocking(&stderr).is_err() {
        terminate_process_group(&mut child);
        return failed("run_command output pipes are unavailable");
    }
    let mut stdout = Some(stdout);
    let mut stderr = Some(stderr);
    {
        let Ok(mut slot) = shared_child.lock() else {
            terminate_process_group(&mut child);
            return failed("run_command state is unavailable");
        };
        *slot = Some(child);
    }

    let stdout_limit = maximum_output_bytes / 2;
    let stderr_limit = maximum_output_bytes.saturating_sub(stdout_limit);
    let mut stdout_bytes = Vec::with_capacity(stdout_limit.min(64 * 1024));
    let mut stderr_bytes = Vec::with_capacity(stderr_limit.min(64 * 1024));
    let mut stdout_truncated = false;
    let mut stderr_truncated = false;
    let started = Instant::now();
    let mut timed_out = false;
    let mut leader_exited = false;
    let mut status = None;
    let mut cleanup_started = None;
    loop {
        let stdout_read = stdout
            .as_mut()
            .map(|pipe| read_nonblocking_bounded(pipe, &mut stdout_bytes, stdout_limit));
        match stdout_read {
            Some(Ok(PipeRead::Open)) => {},
            Some(Ok(PipeRead::Closed)) => stdout = None,
            Some(Ok(PipeRead::Truncated) | Err(())) => {
                stdout = None;
                stdout_truncated = true;
            },
            None => {},
        }
        let stderr_read = stderr
            .as_mut()
            .map(|pipe| read_nonblocking_bounded(pipe, &mut stderr_bytes, stderr_limit));
        match stderr_read {
            Some(Ok(PipeRead::Open)) => {},
            Some(Ok(PipeRead::Closed)) => stderr = None,
            Some(Ok(PipeRead::Truncated) | Err(())) => {
                stderr = None;
                stderr_truncated = true;
            },
            None => {},
        }
        if cancelled.load(Ordering::Acquire) || started.elapsed() >= COMMAND_TIMEOUT {
            timed_out = started.elapsed() >= COMMAND_TIMEOUT;
            cleanup_started.get_or_insert_with(Instant::now);
            if let Some(process_group) = process_group {
                terminate_process_group_id(process_group);
            }
            if let Ok(mut slot) = shared_child.lock()
                && let Some(child) = slot.as_mut()
            {
                terminate_process_group(child);
            }
        }
        if !leader_exited {
            leader_exited = process_group.is_some_and(child_exited_without_reaping);
            if leader_exited {
                cleanup_started.get_or_insert_with(Instant::now);
                if let Some(process_group) = process_group {
                    terminate_process_group_id(process_group);
                }
            }
        }
        if leader_exited && stdout.is_none() && stderr.is_none() {
            break;
        }
        if cleanup_started.is_some_and(|cleanup| cleanup.elapsed() >= PROCESS_CLEANUP_GRACE) {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    if let Some(process_group) = process_group {
        terminate_process_group_id(process_group);
    }
    if let Ok(mut slot) = shared_child.lock()
        && let Some(mut child) = slot.take()
    {
        if leader_exited {
            status = child.wait().ok();
        } else {
            let _ = child.kill();
            status = child.try_wait().ok().flatten();
        }
    }
    drop(stdout);
    drop(stderr);
    if cancelled.load(Ordering::Acquire) {
        return interrupted();
    }
    if timed_out {
        return ToolExecutionResult::new(
            ToolExecutionOutcome::Interrupted,
            "run_command timed out",
            false,
        );
    }
    let stdout = String::from_utf8_lossy(&stdout_bytes);
    let stderr = String::from_utf8_lossy(&stderr_bytes);
    let output = format!(
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        status
            .and_then(|status| status.code())
            .map_or_else(|| "signal".to_owned(), |code| code.to_string()),
        stdout,
        stderr
    );
    ToolExecutionResult::new(
        if status.is_some_and(|status| status.success()) {
            ToolExecutionOutcome::Completed
        } else {
            ToolExecutionOutcome::Failed
        },
        output,
        stdout_truncated || stderr_truncated,
    )
}

fn set_nonblocking(descriptor: &impl AsFd) -> Result<(), ()> {
    let flags = fcntl(descriptor, FcntlArg::F_GETFL).map_err(|_| ())?;
    fcntl(
        descriptor,
        FcntlArg::F_SETFL(OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK),
    )
    .map(|_| ())
    .map_err(|_| ())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PipeRead {
    Open,
    Closed,
    Truncated,
}

fn read_nonblocking_bounded(
    reader: &mut impl Read,
    output: &mut Vec<u8>,
    limit: usize,
) -> Result<PipeRead, ()> {
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let target = limit.saturating_add(1);
        if output.len() >= target {
            output.truncate(limit);
            return Ok(PipeRead::Truncated);
        }
        let read_len = chunk.len().min(target - output.len());
        match reader.read(&mut chunk[..read_len]) {
            Ok(0) => return Ok(PipeRead::Closed),
            Ok(count) => output.extend_from_slice(&chunk[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                return Ok(PipeRead::Open);
            },
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {},
            Err(_) => return Err(()),
        }
    }
}

fn terminate_process_group(child: &mut Child) {
    if let Ok(pid) = i32::try_from(child.id()) {
        terminate_process_group_id(Pid::from_raw(pid));
    }
    let _ = child.kill();
}

fn terminate_process_group_id(process_group: Pid) {
    let _ = kill(Pid::from_raw(-process_group.as_raw()), Signal::SIGKILL);
}

// WNOWAIT leaves the exited group leader as a zombie, so its numeric process-group ID
// remains pinned until the caller has killed any descendants and explicitly reaped it.
#[allow(unsafe_code)]
fn child_exited_without_reaping(child: Pid) -> bool {
    // SAFETY: waitid initializes siginfo_t on success. The zeroed value also makes the
    // WNOHANG/no-state-change result distinguishable by its zero si_pid on Linux and macOS.
    let mut information = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            child.as_raw() as libc::id_t,
            &mut information,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    result == 0 && unsafe { information.si_pid() } != 0
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{Read, Write},
        os::unix::fs::{PermissionsExt, symlink},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::{SystemTime, UNIX_EPOCH},
    };

    use yo_core::{SessionId, TurnId, TurnRef};

    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("yo-local-tools-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn request(registry: &ToolRegistry, name: &str, arguments: &str) -> ToolExecutionRequest {
        ToolExecutionRequest {
            turn: TurnRef::new(
                SessionId::new().unwrap(),
                TurnId::new(std::num::NonZeroU64::new(1).unwrap()),
            ),
            call: registry
                .freeze()
                .validate_call("call-1", name, arguments, 4096)
                .unwrap(),
            maximum_output_bytes: 4096,
        }
    }

    fn finish(execution: &mut dyn ToolExecution) -> ToolExecutionResult {
        for _ in 0..1_000 {
            if execution.poll().unwrap() == ToolExecutionPoll::Ready {
                let result = execution.take_result().unwrap();
                execution.shutdown().unwrap();
                return result;
            }
            thread::sleep(Duration::from_millis(1));
        }
        panic!("local tool did not finish")
    }

    #[cfg(target_vendor = "apple")]
    // Apple의 signed dev_t가 high bit를 가진 경우에도 MetadataExt::dev와 같은 u64
    // 비트 표현을 보존해 credential identity 비교가 정상 파일을 거절하지 않습니다.
    #[test]
    fn apple_device_identity_preserves_the_complete_signed_domain() {
        assert_eq!(normalize_device_id(-1), u64::MAX);
        assert_eq!(normalize_device_id(i32::MIN), i32::MIN as u64);
    }

    struct CountingReader {
        reads: Arc<AtomicUsize>,
    }

    impl Read for CountingReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            buffer.fill(b'x');
            Ok(buffer.len())
        }
    }

    // bounded reader는 limit+1 byte로 truncation을 판별한 즉시 멈춰 무한하거나 거대한
    // 입력을 끝까지 drain하지 않고 반환한다.
    #[test]
    fn bounded_reader_stops_after_the_truncation_probe() {
        let reads = Arc::new(AtomicUsize::new(0));
        let (output, truncated) = read_bounded(
            CountingReader {
                reads: Arc::clone(&reads),
            },
            16,
        )
        .unwrap();

        assert_eq!(output.len(), 16);
        assert!(truncated);
        assert_eq!(reads.load(Ordering::Relaxed), 1);
    }

    // 자동 file tool은 workspace 안의 일반 파일만 읽고 credential과 상위 경로는 거부한다.
    #[test]
    fn reads_workspace_files_but_denies_the_credential_file() {
        let directory = TestDirectory::new();
        let source = directory.0.join("source.txt");
        let credential = directory.0.join("credentials.yaml");
        fs::write(&source, "hello").unwrap();
        let mut file = fs::File::create(&credential).unwrap();
        file.write_all(b"secret").unwrap();
        fs::set_permissions(&credential, fs::Permissions::from_mode(0o600)).unwrap();
        let registry = registry().unwrap();
        let mut host = LocalToolHost::new(&directory.0, &credential).unwrap();

        let mut execution = host
            .start(request(&registry, "read_file", r#"{"path":"source.txt"}"#))
            .unwrap();
        assert_eq!(finish(execution.as_mut()).output(), "hello");
        assert!(
            host.start(request(
                &registry,
                "read_file",
                r#"{"path":"credentials.yaml"}"#
            ))
            .is_err()
        );
        assert!(
            host.start(request(&registry, "read_file", r#"{"path":"../outside"}"#))
                .is_err()
        );
    }

    // 요청 시 연 workspace-relative handle을 작업 thread까지 넘기므로 이후 경로가
    // workspace 밖 symlink로 교체되어도 read/list 대상이 바뀌지 않는다.
    #[test]
    fn opened_workspace_handles_resist_later_symlink_replacement() {
        let workspace = TestDirectory::new();
        let outside = TestDirectory::new();
        let source = workspace.0.join("source.txt");
        let original_source = workspace.0.join("source-original.txt");
        let outside_source = outside.0.join("outside.txt");
        fs::write(&source, "inside").unwrap();
        fs::write(&outside_source, "outside").unwrap();
        let listed = workspace.0.join("listed");
        let original_listed = workspace.0.join("listed-original");
        fs::create_dir(&listed).unwrap();
        fs::write(listed.join("inside.txt"), "inside").unwrap();
        let outside_listed = outside.0.join("listed");
        fs::create_dir(&outside_listed).unwrap();
        fs::write(outside_listed.join("outside.txt"), "outside").unwrap();
        let host = LocalToolHost::new(&workspace.0, &workspace.0.join("credentials.yaml")).unwrap();

        let file = host.open_file("source.txt").unwrap();
        let (directory, relative) = host.open_directory("listed").unwrap();
        fs::rename(&source, &original_source).unwrap();
        symlink(&outside_source, &source).unwrap();
        fs::rename(&listed, &original_listed).unwrap();
        symlink(&outside_listed, &listed).unwrap();

        assert_eq!(
            read_file(file, 1024, &AtomicBool::new(false)).output(),
            "inside"
        );
        let listing = list_files(directory, relative, 1024, &AtomicBool::new(false));
        assert!(listing.output().contains("listed/inside.txt"));
        assert!(!listing.output().contains("outside.txt"));
    }

    // process effect는 명시적 승인 대상으로 등록되고 완료와 취소가 bounded result로 닫힌다.
    #[test]
    fn command_execution_is_approval_bound_and_cancellable() {
        let directory = TestDirectory::new();
        let credential = directory.0.join("credentials.yaml");
        let registry = registry().unwrap();
        let frozen = registry.freeze();
        let run = frozen
            .definitions()
            .iter()
            .find(|definition| definition.wire_name() == "run_command")
            .unwrap();
        assert_eq!(run.approval(), ToolApprovalRequirement::Required);
        let mut host = LocalToolHost::new(&directory.0, &credential).unwrap();
        let mut execution = host
            .start(request(
                &registry,
                "run_command",
                r#"{"command":"printf done"}"#,
            ))
            .unwrap();
        let result = finish(execution.as_mut());
        assert_eq!(result.outcome(), ToolExecutionOutcome::Completed);
        assert!(result.output().contains("done"));

        let started = Instant::now();
        let mut background = host
            .start(request(
                &registry,
                "run_command",
                r#"{"command":"sleep 5 &"}"#,
            ))
            .unwrap();
        assert_eq!(
            finish(background.as_mut()).outcome(),
            ToolExecutionOutcome::Completed
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "a background descendant retained the output pipes"
        );

        let mut cancelled = host
            .start(request(
                &registry,
                "run_command",
                r#"{"command":"sleep 5 & wait"}"#,
            ))
            .unwrap();
        cancelled.cancel();
        assert_eq!(
            finish(cancelled.as_mut()).outcome(),
            ToolExecutionOutcome::Interrupted
        );
    }

    // 선택한 API key가 tool output에 섞이면 replay나 transcript에 들어가기 전에 거부한다.
    #[test]
    fn semantic_admission_rejects_selected_credential_material() {
        let admission = LocalSemanticAdmission::new(
            yo_core::CredentialStore::new([
                (
                    (
                        yo_core::ProviderId::new("openrouter").unwrap(),
                        yo_core::AccountId::new("default").unwrap(),
                    ),
                    yo_core::ApiCredential::new("sk-sensitive").unwrap(),
                ),
                (
                    (
                        yo_core::ProviderId::new("qwencloud").unwrap(),
                        yo_core::AccountId::new("default").unwrap(),
                    ),
                    yo_core::ApiCredential::new("sk-other-account").unwrap(),
                ),
            ])
            .unwrap(),
        );
        let definition = registry().unwrap().freeze().definitions()[0].clone();

        assert!(admission.admit_output(&definition, "safe").is_ok());
        assert!(
            admission
                .admit_output(&definition, "prefix sk-sensitive suffix")
                .is_err()
        );
        assert!(
            admission
                .admit_output(&definition, "prefix sk-other-account suffix")
                .is_err()
        );
    }
}
