use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io,
    io::{Read, Write},
    os::unix::fs::MetadataExt,
    panic, str,
    sync::{
        Arc, Mutex, MutexGuard, TryLockError,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use nix::{
    errno::Errno,
    fcntl::{AtFlags, OFlag, openat, renameat},
    sys::stat::{Mode, SFlag, fchmod, fstat, fstatat},
    unistd::{UnlinkatFlags, close, unlinkat},
};
use serde_json::Value;
use yo_core::{ToolExecutionError, ToolExecutionResult};

use super::{
    descriptor::{
        FileIdentity, normalize_device_id, open_beneath, open_regular_file, permission_mode_u32,
    },
    output::{error, json_string},
    path::AdmittedPath,
};
use crate::execution::tools::execution::{completed, failed, interrupted};

const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
const MAX_EDITS: usize = 256;
const SCRATCH_ATTEMPTS: usize = 16;

#[derive(Clone, Default)]
pub(super) struct UnwindCleanup {
    failed: Arc<AtomicBool>,
    #[cfg(test)]
    force_failure: Arc<AtomicBool>,
}

impl UnwindCleanup {
    fn record(&self, cleanup_succeeded: bool) {
        #[cfg(test)]
        let cleanup_succeeded = cleanup_succeeded && !self.force_failure.load(Ordering::Acquire);
        if !cleanup_succeeded {
            self.failed.store(true, Ordering::Release);
        }
    }

    fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    #[cfg(test)]
    fn force_failure(&self) {
        self.force_failure.store(true, Ordering::Release);
    }
}

pub(super) fn catch_failure(
    path: &str,
    cleanup: &UnwindCleanup,
    operation: impl FnOnce() -> ToolExecutionResult,
) -> ToolExecutionResult {
    panic::catch_unwind(panic::AssertUnwindSafe(operation)).unwrap_or_else(|_| {
        mutation_error(
            path,
            if cleanup.failed() {
                "cleanup_failed"
            } else {
                "operation_failed"
            },
        )
    })
}

#[derive(Clone, Debug)]
pub(super) struct EditRequest {
    path: AdmittedPath,
    edits: Vec<super::mutation_plan::ExactEdit>,
}

impl EditRequest {
    pub(super) fn path(&self) -> &str {
        self.path.display()
    }
}

#[derive(Clone, Debug)]
pub(super) struct WriteRequest {
    path: AdmittedPath,
    content: String,
}

impl WriteRequest {
    pub(super) fn path(&self) -> &str {
        self.path.display()
    }
}

pub(super) fn parse_edit(
    arguments: &Value,
    admit_path: fn(&str) -> Result<AdmittedPath, ToolExecutionError>,
) -> Result<EditRequest, ToolExecutionError> {
    let path = string(arguments, "path")?;
    let edits = arguments
        .get("edits")
        .and_then(Value::as_array)
        .ok_or_else(|| ToolExecutionError::new("validated edit_file edits are unavailable"))?;
    if edits.is_empty() || edits.len() > MAX_EDITS {
        return Err(ToolExecutionError::new(
            "edit_file requires between 1 and 256 edits",
        ));
    }
    let edits = edits
        .iter()
        .map(|edit| {
            let old_text = string(edit, "oldText")?;
            if old_text.is_empty() {
                return Err(ToolExecutionError::new(
                    "edit_file oldText must be non-empty",
                ));
            }
            Ok(super::mutation_plan::ExactEdit::new(
                old_text.to_owned(),
                string(edit, "newText")?.to_owned(),
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(EditRequest {
        path: admit_path(path)?,
        edits,
    })
}

pub(super) fn parse_write(
    arguments: &Value,
    admit_path: fn(&str) -> Result<AdmittedPath, ToolExecutionError>,
) -> Result<WriteRequest, ToolExecutionError> {
    let path = string(arguments, "path")?;
    let content = string(arguments, "content")?;
    if content.len() > MAX_FILE_BYTES {
        return Err(ToolExecutionError::new(
            "write_file content exceeds its byte bound",
        ));
    }
    Ok(WriteRequest {
        path: admit_path(path)?,
        content: content.to_owned(),
    })
}

fn string<'a>(value: &'a Value, name: &str) -> Result<&'a str, ToolExecutionError> {
    value
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolExecutionError::new("validated local tool argument is unavailable"))
}

pub(super) fn execute_edit(
    workspace: File,
    denied_credential: Option<FileIdentity>,
    lock: Arc<Mutex<()>>,
    request: EditRequest,
    cancelled: &AtomicBool,
    unwind_cleanup: UnwindCleanup,
) -> ToolExecutionResult {
    execute_edit_after_capture(
        workspace,
        denied_credential,
        lock,
        request,
        cancelled,
        unwind_cleanup,
        || {},
    )
}

#[allow(clippy::too_many_arguments)]
fn execute_edit_after_capture(
    workspace: File,
    denied_credential: Option<FileIdentity>,
    lock: Arc<Mutex<()>>,
    request: EditRequest,
    cancelled: &AtomicBool,
    unwind_cleanup: UnwindCleanup,
    after_capture: impl FnOnce(),
) -> ToolExecutionResult {
    let _guard = match lock_mutation(&lock, cancelled) {
        Ok(Some(guard)) => guard,
        Ok(None) => return interrupted(),
        Err(()) => return mutation_error(request.path.display(), "operation_failed"),
    };
    let target = match open_regular_file(&workspace, request.path.components(), denied_credential) {
        Ok(file) => file,
        Err(_) => return mutation_error(request.path.display(), "unavailable"),
    };
    let before = match Capture::from_file(&target) {
        Ok(capture) => capture,
        Err(_) => return mutation_error(request.path.display(), "unavailable"),
    };
    if before.size > MAX_FILE_BYTES as u64 {
        return mutation_error(request.path.display(), "too_large");
    }
    after_capture();
    let mut original = Vec::with_capacity(before.size as usize);
    let mut reader = target;
    if Read::by_ref(&mut reader)
        .take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut original)
        .is_err()
    {
        return mutation_error(request.path.display(), "unavailable");
    }
    let after = match Capture::from_file(&reader) {
        Ok(capture) => capture,
        Err(_) => return mutation_error(request.path.display(), "changed_during_read"),
    };
    if before != after || after.size != original.len() as u64 {
        return mutation_error(request.path.display(), "changed_during_read");
    }
    if original.len() > MAX_FILE_BYTES {
        return mutation_error(request.path.display(), "too_large");
    }
    if str::from_utf8(&original).is_err() {
        return mutation_error(request.path.display(), "non_utf8");
    }
    let replacements = match super::mutation_plan::plan_replacements(&original, &request.edits) {
        Ok(replacements) => replacements,
        Err(class) => return mutation_error(request.path.display(), class),
    };
    let planned_len = request
        .edits
        .iter()
        .try_fold(original.len(), |length, edit| {
            length
                .checked_sub(edit.old_len())?
                .checked_add(edit.new_len())
        });
    if planned_len.is_none_or(|length| length > MAX_FILE_BYTES) {
        return mutation_error(request.path.display(), "too_large");
    }
    let planned =
        super::mutation_plan::apply_replacements(&original, &request.edits, &replacements);
    if planned == original {
        return mutation_error(request.path.display(), "no_change");
    }
    if planned.len() > MAX_FILE_BYTES {
        return mutation_error(request.path.display(), "too_large");
    }
    publish(
        &workspace,
        denied_credential,
        &request.path,
        &planned,
        before.mode,
        cancelled,
        Success::Edit(request.edits.len()),
        unwind_cleanup,
    )
}

pub(super) fn execute_write(
    workspace: File,
    denied_credential: Option<FileIdentity>,
    lock: Arc<Mutex<()>>,
    request: WriteRequest,
    new_file_mode: u32,
    cancelled: &AtomicBool,
    unwind_cleanup: UnwindCleanup,
) -> ToolExecutionResult {
    execute_write_after_mode(
        workspace,
        denied_credential,
        lock,
        request,
        new_file_mode,
        cancelled,
        unwind_cleanup,
        |_| {},
    )
}

#[allow(clippy::too_many_arguments)]
fn execute_write_after_mode(
    workspace: File,
    denied_credential: Option<FileIdentity>,
    lock: Arc<Mutex<()>>,
    request: WriteRequest,
    new_file_mode: u32,
    cancelled: &AtomicBool,
    unwind_cleanup: UnwindCleanup,
    after_mode: impl FnOnce(&mut Scratch),
) -> ToolExecutionResult {
    let _guard = match lock_mutation(&lock, cancelled) {
        Ok(Some(guard)) => guard,
        Ok(None) => return interrupted(),
        Err(()) => return mutation_error(request.path.display(), "operation_failed"),
    };
    let (parent, name) = match open_parent(&workspace, request.path.components()) {
        Ok(value) => value,
        Err(_) => return mutation_error(request.path.display(), "unavailable"),
    };
    let mode = match existing_target_mode(&parent, &name, denied_credential) {
        Ok(Some(mode)) => mode,
        Ok(None) => new_file_mode,
        Err(_) => return mutation_error(request.path.display(), "unavailable"),
    };
    publish_in_parent_after_mode(
        parent,
        name,
        denied_credential,
        &request.path,
        request.content.as_bytes(),
        mode,
        cancelled,
        Success::Write(request.content.len()),
        unwind_cleanup,
        after_mode,
    )
}

fn lock_mutation<'a>(
    lock: &'a Mutex<()>,
    cancelled: &AtomicBool,
) -> Result<Option<MutexGuard<'a, ()>>, ()> {
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Ok(None);
        }
        match lock.try_lock() {
            Ok(guard) => return Ok(Some(guard)),
            Err(TryLockError::Poisoned(poisoned)) => return Ok(Some(poisoned.into_inner())),
            Err(TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(1)),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn publish(
    workspace: &File,
    denied_credential: Option<FileIdentity>,
    path: &AdmittedPath,
    bytes: &[u8],
    mode: u32,
    cancelled: &AtomicBool,
    success: Success,
    unwind_cleanup: UnwindCleanup,
) -> ToolExecutionResult {
    let (parent, name) = match open_parent(workspace, path.components()) {
        Ok(value) => value,
        Err(_) => return mutation_error(path.display(), "unavailable"),
    };
    publish_in_parent(
        parent,
        name,
        denied_credential,
        path,
        bytes,
        mode,
        cancelled,
        success,
        unwind_cleanup,
    )
}

#[allow(clippy::too_many_arguments)]
fn publish_in_parent(
    parent: File,
    target_name: OsString,
    denied_credential: Option<FileIdentity>,
    path: &AdmittedPath,
    bytes: &[u8],
    mode: u32,
    cancelled: &AtomicBool,
    success: Success,
    unwind_cleanup: UnwindCleanup,
) -> ToolExecutionResult {
    publish_in_parent_after_mode(
        parent,
        target_name,
        denied_credential,
        path,
        bytes,
        mode,
        cancelled,
        success,
        unwind_cleanup,
        |_| {},
    )
}

#[allow(clippy::too_many_arguments)]
fn publish_in_parent_after_mode(
    parent: File,
    target_name: OsString,
    denied_credential: Option<FileIdentity>,
    path: &AdmittedPath,
    bytes: &[u8],
    mode: u32,
    cancelled: &AtomicBool,
    success: Success,
    unwind_cleanup: UnwindCleanup,
    after_mode: impl FnOnce(&mut Scratch),
) -> ToolExecutionResult {
    let mut scratch = match Scratch::create(parent, unwind_cleanup) {
        Ok(scratch) => scratch,
        Err(class) => return mutation_error(path.display(), class),
    };
    let write_result = scratch
        .file
        .as_mut()
        .expect("new scratch retains its descriptor")
        .write_all(bytes)
        .and_then(|()| {
            fchmod(
                scratch
                    .file
                    .as_ref()
                    .expect("scratch descriptor remains present"),
                Mode::from_bits_truncate(mode as _),
            )
            .map_err(io::Error::other)
        });
    if write_result.is_err() {
        return scratch.finish(path.display(), Terminal::Failed("write_failed"));
    }
    after_mode(&mut scratch);
    if cancelled.load(Ordering::Acquire) {
        return scratch.finish(path.display(), Terminal::Interrupted);
    }
    if !scratch.identity_matches(denied_credential) {
        return scratch.finish(path.display(), Terminal::Failed("scratch_changed"));
    }
    if renameat(
        &scratch.parent,
        scratch.name.as_os_str(),
        &scratch.parent,
        target_name.as_os_str(),
    )
    .is_err()
    {
        return scratch.finish(path.display(), Terminal::Failed("publication_failed"));
    }
    scratch.cleanup_pending = false;
    scratch.file.take();
    match success {
        Success::Edit(count) => completed(
            format!(
                "{{\"path\":{},\"status\":\"ok\",\"replacements\":{count}}}",
                json_string(path.display())
            ),
            false,
        ),
        Success::Write(count) => completed(
            format!(
                "{{\"path\":{},\"status\":\"ok\",\"bytes\":{count}}}",
                json_string(path.display())
            ),
            false,
        ),
    }
}

enum Success {
    Edit(usize),
    Write(usize),
}

enum Terminal {
    Failed(&'static str),
    Interrupted,
}

struct Scratch {
    parent: File,
    name: OsString,
    file: Option<File>,
    identity: FileIdentity,
    cleanup_pending: bool,
    unwind_cleanup: UnwindCleanup,
}

impl Scratch {
    fn create(parent: File, unwind_cleanup: UnwindCleanup) -> Result<Self, &'static str> {
        for _ in 0..SCRATCH_ATTEMPTS {
            let mut random = [0_u8; 16];
            getrandom::fill(&mut random).map_err(|_| "scratch_unavailable")?;
            let name = OsString::from(format!(
                ".yo-write-{}",
                random
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            ));
            match openat(
                &parent,
                name.as_os_str(),
                OFlag::O_WRONLY
                    | OFlag::O_CREAT
                    | OFlag::O_EXCL
                    | OFlag::O_NOFOLLOW
                    | OFlag::O_CLOEXEC,
                Mode::from_bits_truncate(0o600),
            ) {
                Ok(descriptor) => {
                    let metadata = match fstat(&descriptor) {
                        Ok(metadata) => metadata,
                        Err(_) => {
                            drop(descriptor);
                            return Err("cleanup_failed");
                        },
                    };
                    let mut scratch = Self {
                        parent,
                        name,
                        file: Some(File::from(descriptor)),
                        identity: FileIdentity {
                            device: normalize_device_id(metadata.st_dev),
                            inode: metadata.st_ino,
                        },
                        cleanup_pending: true,
                        unwind_cleanup,
                    };
                    if fchmod(
                        scratch
                            .file
                            .as_ref()
                            .expect("new scratch retains its descriptor"),
                        Mode::from_bits_truncate(0o600),
                    )
                    .is_err()
                    {
                        return Err(if scratch.cleanup_once() {
                            "scratch_unavailable"
                        } else {
                            "cleanup_failed"
                        });
                    }
                    return Ok(scratch);
                },
                Err(Errno::EEXIST) => {},
                Err(_) => return Err("scratch_unavailable"),
            }
        }
        Err("scratch_unavailable")
    }

    fn identity_matches(&self, denied_credential: Option<FileIdentity>) -> bool {
        let Ok(metadata) = fstatat(
            &self.parent,
            self.name.as_os_str(),
            AtFlags::AT_SYMLINK_NOFOLLOW,
        ) else {
            return false;
        };
        let identity = FileIdentity {
            device: normalize_device_id(metadata.st_dev),
            inode: metadata.st_ino,
        };
        SFlag::from_bits_truncate(metadata.st_mode) & SFlag::S_IFMT == SFlag::S_IFREG
            && identity == self.identity
            && denied_credential != Some(identity)
    }

    fn finish(mut self, path: &str, terminal: Terminal) -> ToolExecutionResult {
        if !self.cleanup_once() {
            return mutation_error(path, "cleanup_failed");
        }
        match terminal {
            Terminal::Failed(class) => mutation_error(path, class),
            Terminal::Interrupted => interrupted(),
        }
    }

    fn cleanup_once(&mut self) -> bool {
        if !self.cleanup_pending {
            return true;
        }
        self.cleanup_pending = false;
        self.cleanup()
    }

    fn cleanup(&mut self) -> bool {
        let close_failed = self.file.take().is_some_and(|file| close(file).is_err());
        let namespace_failed = match fstatat(
            &self.parent,
            self.name.as_os_str(),
            AtFlags::AT_SYMLINK_NOFOLLOW,
        ) {
            Err(Errno::ENOENT) => false,
            Err(_) => true,
            Ok(metadata) => {
                let observed = FileIdentity {
                    device: normalize_device_id(metadata.st_dev),
                    inode: metadata.st_ino,
                };
                observed == self.identity
                    && unlinkat(
                        &self.parent,
                        self.name.as_os_str(),
                        UnlinkatFlags::NoRemoveDir,
                    )
                    .is_err()
            },
        };
        !close_failed && !namespace_failed
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if self.cleanup_pending {
            let cleanup_succeeded = self.cleanup();
            self.cleanup_pending = false;
            self.unwind_cleanup.record(cleanup_succeeded);
        }
    }
}

fn open_parent(workspace: &File, components: &[OsString]) -> Result<(File, OsString), ()> {
    let (name, parents) = components.split_last().ok_or(())?;
    let descriptor =
        open_beneath(workspace, parents, OFlag::O_RDONLY | OFlag::O_DIRECTORY).map_err(|_| ())?;
    Ok((File::from(descriptor), name.clone()))
}

fn existing_target_mode(
    parent: &File,
    name: &OsStr,
    denied_credential: Option<FileIdentity>,
) -> Result<Option<u32>, ()> {
    let metadata = match fstatat(parent, name, AtFlags::AT_SYMLINK_NOFOLLOW) {
        Ok(metadata) => metadata,
        Err(Errno::ENOENT) => return Ok(None),
        Err(_) => return Err(()),
    };
    let identity = FileIdentity {
        device: normalize_device_id(metadata.st_dev),
        inode: metadata.st_ino,
    };
    if SFlag::from_bits_truncate(metadata.st_mode) & SFlag::S_IFMT != SFlag::S_IFREG
        || denied_credential == Some(identity)
    {
        return Err(());
    }
    Ok(Some(permission_mode_u32(metadata.st_mode & 0o7777)))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Capture {
    identity: FileIdentity,
    size: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
    mode: u32,
}

impl Capture {
    fn from_file(file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        Ok(Self {
            identity: FileIdentity {
                device: metadata.dev(),
                inode: metadata.ino(),
            },
            size: metadata.len(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
            mode: metadata.mode() & 0o7777,
        })
    }
}

fn mutation_error(path: &str, class: &str) -> ToolExecutionResult {
    failed(&error(path, class))
}

#[cfg(test)]
mod tests;
