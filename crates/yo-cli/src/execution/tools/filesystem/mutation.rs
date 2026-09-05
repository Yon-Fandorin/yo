use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{Read, Write},
    os::unix::fs::MetadataExt,
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
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)).unwrap_or_else(|_| {
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
    if std::str::from_utf8(&original).is_err() {
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
            .map_err(std::io::Error::other)
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
        SFlag::from_bits_truncate(metadata.st_mode).contains(SFlag::S_IFREG)
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
    if !SFlag::from_bits_truncate(metadata.st_mode).contains(SFlag::S_IFREG)
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
    fn from_file(file: &File) -> std::io::Result<Self> {
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
mod tests {
    use std::{
        ffi::OsString,
        fs::{self, OpenOptions},
        io::Write,
        sync::{Arc, Mutex, atomic::AtomicBool},
    };

    use super::{
        EditRequest, MAX_FILE_BYTES, Scratch, Success, Terminal, UnwindCleanup, WriteRequest,
        catch_failure, execute_edit_after_capture, execute_write, execute_write_after_mode,
        lock_mutation, publish_in_parent,
    };
    use crate::execution::tools::{filesystem::path::AdmittedPath, tests::TestDirectory};

    // waiting mutation은 같은 host lock을 우회하지 않으며 cancellation이 이미 보이면
    // filesystem phase에 들어가지 않고 Interrupted 경로를 선택할 수 있습니다.
    #[test]
    fn mutation_lock_wait_observes_cancellation_without_interleaving() {
        let lock = Arc::new(Mutex::new(()));
        let _held = lock.lock().unwrap();
        assert!(
            lock_mutation(&lock, &AtomicBool::new(true))
                .unwrap()
                .is_none()
        );
    }

    // rename failure와 prepublication cancellation은 owned scratch를 제거하고 target을
    // publish하지 않으며 각각 stable failure와 Interrupted를 구분합니다.
    #[test]
    fn publication_failure_and_cancellation_cleanup_owned_scratch() {
        let directory = TestDirectory::new();
        fs::create_dir(directory.0.join("target")).unwrap();
        let parent = fs::File::open(&directory.0).unwrap();
        let path = AdmittedPath::new("target".to_owned(), vec![OsString::from("target")]);
        let result = publish_in_parent(
            parent,
            OsString::from("target"),
            None,
            &path,
            b"content",
            0o600,
            &AtomicBool::new(false),
            Success::Write(7),
            UnwindCleanup::default(),
        );
        assert_eq!(
            result.output(),
            r#"{"path":"target","status":"error","error":"publication_failed"}"#
        );

        let parent = fs::File::open(&directory.0).unwrap();
        let cancelled_path =
            AdmittedPath::new("cancelled".to_owned(), vec![OsString::from("cancelled")]);
        let cancelled = publish_in_parent(
            parent,
            OsString::from("cancelled"),
            None,
            &cancelled_path,
            b"content",
            0o600,
            &AtomicBool::new(true),
            Success::Write(7),
            UnwindCleanup::default(),
        );
        assert_eq!(
            cancelled.outcome(),
            yo_core::ToolExecutionOutcome::Interrupted
        );
        assert!(!directory.0.join("cancelled").exists());
        assert!(fs::read_dir(&directory.0).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".yo-write-")
        }));
    }

    // 외부 same-UID publisher가 scratch 이름을 바꿔치기하면 cleanup은 foreign entry를
    // unlink하지 않고 원래 scratch_changed 결과를 유지합니다.
    #[test]
    fn cleanup_never_unlinks_a_foreign_scratch_replacement() {
        let directory = TestDirectory::new();
        let parent = fs::File::open(&directory.0).unwrap();
        let scratch = Scratch::create(parent, UnwindCleanup::default()).unwrap();
        let scratch_path = directory.0.join(&scratch.name);
        fs::rename(&scratch_path, directory.0.join("moved-owned")).unwrap();
        fs::write(&scratch_path, "foreign").unwrap();

        let result = scratch.finish("target", Terminal::Failed("scratch_changed"));

        assert_eq!(
            result.output(),
            r#"{"path":"target","status":"error","error":"scratch_changed"}"#
        );
        assert_eq!(fs::read_to_string(scratch_path).unwrap(), "foreign");
    }

    // 첫 capture 뒤 source가 한 byte 커지면 size bound를 먼저 반환하지 않고 fixed phase
    // order에 따라 metadata/observed-length 변화가 changed_during_read를 선택합니다.
    #[test]
    fn edit_growth_precedes_the_stable_size_limit() {
        let directory = TestDirectory::new();
        let source = directory.0.join("growing.txt");
        fs::write(&source, vec![b'x'; MAX_FILE_BYTES]).unwrap();
        let workspace = fs::File::open(&directory.0).unwrap();
        let request = EditRequest {
            path: AdmittedPath::new(
                "growing.txt".to_owned(),
                vec![OsString::from("growing.txt")],
            ),
            edits: vec![super::super::mutation_plan::ExactEdit::new(
                "x".into(),
                "y".into(),
            )],
        };

        let result = execute_edit_after_capture(
            workspace,
            None,
            Arc::new(Mutex::new(())),
            request,
            &AtomicBool::new(false),
            UnwindCleanup::default(),
            || {
                OpenOptions::new()
                    .append(true)
                    .open(&source)
                    .unwrap()
                    .write_all(b"y")
                    .unwrap();
            },
        );

        assert_eq!(
            result.output(),
            r#"{"path":"growing.txt","status":"error","error":"changed_during_read"}"#
        );
    }

    // final mode 적용 뒤 panic이 발생해도 Scratch Drop이 exact owned pathname을 한 번
    // 정리하고 operation_failed를 보존하며, poisoned lock은 다음 mutation에 재사용됩니다.
    #[test]
    fn panic_after_final_mode_cleans_scratch_and_does_not_disable_mutation() {
        let directory = TestDirectory::new();
        let lock = Arc::new(Mutex::new(()));
        let cleanup = UnwindCleanup::default();
        let result = catch_failure("panic.txt", &cleanup, || {
            execute_write_after_mode(
                fs::File::open(&directory.0).unwrap(),
                None,
                Arc::clone(&lock),
                WriteRequest {
                    path: AdmittedPath::new(
                        "panic.txt".to_owned(),
                        vec![OsString::from("panic.txt")],
                    ),
                    content: "complete content".to_owned(),
                },
                0o600,
                &AtomicBool::new(false),
                cleanup.clone(),
                |_| panic!("injected after-mode cut"),
            )
        });
        assert_eq!(
            result.output(),
            r#"{"path":"panic.txt","status":"error","error":"operation_failed"}"#
        );
        assert!(!directory.0.join("panic.txt").exists());
        assert!(fs::read_dir(&directory.0).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".yo-write-")
        }));

        let next = execute_write(
            fs::File::open(&directory.0).unwrap(),
            None,
            lock,
            WriteRequest {
                path: AdmittedPath::new("next.txt".to_owned(), vec![OsString::from("next.txt")]),
                content: "next".to_owned(),
            },
            0o600,
            &AtomicBool::new(false),
            UnwindCleanup::default(),
        );
        assert_eq!(next.outcome(), yo_core::ToolExecutionOutcome::Completed);
        assert_eq!(
            fs::read_to_string(directory.0.join("next.txt")).unwrap(),
            "next"
        );
    }

    // panic cleanup 자체가 실패했다고 주입하면 primary operation_failed보다
    // cleanup_failed가 우선하며, 성공 cleanup 경로와 결과 선택을 구분합니다.
    #[test]
    fn panic_cleanup_failure_overrides_the_internal_failure() {
        let directory = TestDirectory::new();
        let cleanup = UnwindCleanup::default();
        cleanup.force_failure();
        let result = catch_failure("panic.txt", &cleanup, || {
            execute_write_after_mode(
                fs::File::open(&directory.0).unwrap(),
                None,
                Arc::new(Mutex::new(())),
                WriteRequest {
                    path: AdmittedPath::new(
                        "panic.txt".to_owned(),
                        vec![OsString::from("panic.txt")],
                    ),
                    content: "complete content".to_owned(),
                },
                0o600,
                &AtomicBool::new(false),
                cleanup.clone(),
                |_| panic!("injected after-mode cut"),
            )
        });
        assert_eq!(
            result.output(),
            r#"{"path":"panic.txt","status":"error","error":"cleanup_failed"}"#
        );
    }
}
