use std::{
    ffi::{OsStr, OsString},
    fs::{File, OpenOptions},
    io::Read,
    os::{
        fd::OwnedFd,
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
        },
    },
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use nix::{
    dir::Dir,
    errno::Errno,
    fcntl::{AtFlags, OFlag, openat},
    sys::stat::{Mode, SFlag, fstat, fstatat},
};
use serde_json::Value;
use yo_core::{
    ToolDefinition, ToolExecution, ToolExecutionError, ToolExecutionHost, ToolExecutionRequest,
    ToolExecutionResult, ToolId,
};

use super::{
    command::CommandExecution,
    execution::{ThreadExecution, completed, failed, interrupted},
};

mod mutation;
mod output;
mod read;

const HOST_IDENTITY: &str = "yo.local-workspace-tools/v1";
const MAX_LIST_ENTRIES: usize = 100_000;
// list_files는 private common-output owner와 이 exact marker의 공간만 공유합니다.
const LIST_TRUNCATION_MARKER: &str = "\n[yo: tool output truncated]";
static NEW_FILE_MODE: OnceLock<u32> = OnceLock::new();

pub(crate) fn initialize_process_file_mode() {
    NEW_FILE_MODE.get_or_init(capture_new_file_mode);
}

pub(super) fn validate_arguments(
    definition: &ToolDefinition,
    arguments: &Value,
) -> Result<(), ToolExecutionError> {
    match definition.id().as_str() {
        "list-files" => LocalToolHost::list_path(arguments, "path").map(drop),
        "read-files" => read::parse_requests(arguments, LocalToolHost::basic_path).map(drop),
        "edit-file" => mutation::parse_edit(arguments, LocalToolHost::basic_path).map(drop),
        "write-file" => mutation::parse_write(arguments, LocalToolHost::basic_path).map(drop),
        _ => Ok(()),
    }
}

pub(crate) struct LocalToolHost {
    workspace: PathBuf,
    workspace_directory: File,
    denied_credential: Option<FileIdentity>,
    mutation_lock: Arc<Mutex<()>>,
    new_file_mode: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct FileIdentity {
    device: u64,
    inode: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OpenRegularError {
    Unavailable,
    NotRegular,
}

fn permission_mode_u32(mode: impl Into<u32>) -> u32 {
    mode.into()
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
            mutation_lock: Arc::new(Mutex::new(())),
            new_file_mode: *NEW_FILE_MODE.get_or_init(capture_new_file_mode),
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

    fn admitted_path_components(value: &str) -> Result<Vec<OsString>, ToolExecutionError> {
        if value.len() > 1_024 || value.chars().any(char::is_control) {
            return Err(ToolExecutionError::new(
                "tool path exceeds its byte bound or contains a control character",
            ));
        }
        Self::path_components(value)
    }

    fn list_path(arguments: &Value, name: &str) -> Result<Vec<OsString>, ToolExecutionError> {
        Self::admitted_path_components(string_argument(arguments, name)?)
    }

    fn basic_path(value: &str) -> Result<read::AdmittedPath, ToolExecutionError> {
        let components = Self::admitted_path_components(value)?;
        if components.is_empty() {
            return Err(ToolExecutionError::new(
                "tool file path must not name the workspace root",
            ));
        }
        Ok(read::AdmittedPath::new(value.to_owned(), components))
    }

    fn open_directory(&self, value: &str) -> Result<(Dir, PathBuf), ToolExecutionError> {
        let components = Self::admitted_path_components(value)?;
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
        matches!(
            tool.as_str(),
            "read-file" | "list-files" | "read-files" | "edit-file" | "write-file" | "run-command"
        )
    }

    fn start(
        &mut self,
        request: ToolExecutionRequest,
    ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
        let maximum_output_bytes = request.maximum_output_bytes;
        match request.call.definition().id().as_str() {
            "read-file" => {
                let path = string_argument(request.call.arguments(), "path")?;
                let components = Self::path_components(path)?;
                if components.is_empty() {
                    return Err(ToolExecutionError::new(
                        "read_file path must not name the workspace root",
                    ));
                }
                let workspace = self
                    .workspace_directory
                    .try_clone()
                    .map_err(|_| ToolExecutionError::new("workspace handle is unavailable"))?;
                let denied = self.denied_credential;
                Ok(Box::new(ThreadExecution::spawn(move |cancelled| {
                    let Ok(file) = open_regular_file(&workspace, &components, denied) else {
                        return failed("tool execution failed");
                    };
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
                    request.absolute_execution_timeout,
                )?))
            },
            "read-files" => {
                let files = read::parse_requests(request.call.arguments(), Self::basic_path)?;
                let workspace = self
                    .workspace_directory
                    .try_clone()
                    .map_err(|_| ToolExecutionError::new("workspace handle is unavailable"))?;
                let denied = self.denied_credential;
                Ok(Box::new(ThreadExecution::spawn(move |cancelled| {
                    read::execute(workspace, denied, files, &cancelled)
                })))
            },
            "edit-file" => {
                let edit = mutation::parse_edit(request.call.arguments(), Self::basic_path)?;
                let result_path = edit.path().to_owned();
                let workspace = self
                    .workspace_directory
                    .try_clone()
                    .map_err(|_| ToolExecutionError::new("workspace handle is unavailable"))?;
                let denied = self.denied_credential;
                let lock = Arc::clone(&self.mutation_lock);
                Ok(Box::new(ThreadExecution::spawn(move |cancelled| {
                    let cleanup = mutation::UnwindCleanup::default();
                    mutation::catch_failure(&result_path, &cleanup, || {
                        mutation::execute_edit(
                            workspace,
                            denied,
                            lock,
                            edit,
                            &cancelled,
                            cleanup.clone(),
                        )
                    })
                })))
            },
            "write-file" => {
                let write = mutation::parse_write(request.call.arguments(), Self::basic_path)?;
                let result_path = write.path().to_owned();
                let workspace = self
                    .workspace_directory
                    .try_clone()
                    .map_err(|_| ToolExecutionError::new("workspace handle is unavailable"))?;
                let denied = self.denied_credential;
                let lock = Arc::clone(&self.mutation_lock);
                let mode = self.new_file_mode;
                Ok(Box::new(ThreadExecution::spawn(move |cancelled| {
                    let cleanup = mutation::UnwindCleanup::default();
                    mutation::catch_failure(&result_path, &cleanup, || {
                        mutation::execute_write(
                            workspace,
                            denied,
                            lock,
                            write,
                            mode,
                            &cancelled,
                            cleanup.clone(),
                        )
                    })
                })))
            },
            _ => Err(ToolExecutionError::new("unknown local tool")),
        }
    }

    fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
        Ok(())
    }
}

fn open_regular_file(
    workspace: &File,
    components: &[OsString],
    denied_credential: Option<FileIdentity>,
) -> Result<File, OpenRegularError> {
    let descriptor = open_beneath(workspace, components, OFlag::O_RDONLY | OFlag::O_NONBLOCK)
        .map_err(|_| OpenRegularError::Unavailable)?;
    let metadata = fstat(&descriptor).map_err(|_| OpenRegularError::Unavailable)?;
    if !SFlag::from_bits_truncate(metadata.st_mode).contains(SFlag::S_IFREG) {
        return Err(OpenRegularError::NotRegular);
    }
    if denied_credential
        == Some(FileIdentity {
            device: normalize_device_id(metadata.st_dev),
            inode: metadata.st_ino,
        })
    {
        return Err(OpenRegularError::Unavailable);
    }
    Ok(File::from(descriptor))
}

fn capture_new_file_mode() -> u32 {
    static UMASK_LOCK: Mutex<()> = Mutex::new(());
    let _guard = UMASK_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let current = nix::sys::stat::umask(Mode::empty());
    nix::sys::stat::umask(current);
    permission_mode_u32(0o666 & !current.bits())
}

fn string_argument<'a>(value: &'a Value, name: &str) -> Result<&'a str, ToolExecutionError> {
    value
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolExecutionError::new("validated local tool argument is unavailable"))
}

fn read_file(file: impl Read, limit: usize, cancelled: &AtomicBool) -> ToolExecutionResult {
    if cancelled.load(Ordering::Acquire) {
        return interrupted();
    }
    let result = read_bounded(file, limit);
    if cancelled.load(Ordering::Acquire) {
        return interrupted();
    }
    match result {
        Ok((mut bytes, truncated)) => match std::str::from_utf8(&bytes) {
            Ok(_) => completed(
                String::from_utf8(bytes).expect("validated UTF-8 remains valid"),
                truncated,
            ),
            Err(error) if truncated && error.error_len().is_none() => {
                bytes.truncate(error.valid_up_to());
                completed(
                    String::from_utf8(bytes).expect("valid UTF-8 prefix remains valid"),
                    true,
                )
            },
            Err(_) => failed("read_file supports UTF-8 text files only"),
        },
        Err(_) => failed("read_file failed"),
    }
}

fn list_files(
    mut directory: Dir,
    relative_directory: PathBuf,
    limit: usize,
    cancelled: &AtomicBool,
) -> ToolExecutionResult {
    let retained = retain_list_names(
        directory.iter().map(|entry| {
            entry.map(|entry| OsString::from(OsStr::from_bytes(entry.file_name().to_bytes())))
        }),
        MAX_LIST_ENTRIES,
        cancelled,
    );
    let RetainedListNames { names, truncated } = match retained {
        Ok(retained) => retained,
        Err(ListObservationError::Interrupted) => return interrupted(),
        Err(ListObservationError::Failed) => return failed("list_files failed"),
    };
    render_list_names(
        names,
        &relative_directory,
        limit,
        truncated,
        cancelled,
        |name| classify_list_entry(&directory, name),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ListObservationError {
    Interrupted,
    Failed,
}

#[derive(Debug, Eq, PartialEq)]
struct RetainedListNames {
    names: Vec<OsString>,
    truncated: bool,
}

fn retain_list_names(
    mut entries: impl Iterator<Item = Result<OsString, Errno>>,
    maximum: usize,
    cancelled: &AtomicBool,
) -> Result<RetainedListNames, ListObservationError> {
    let mut names = Vec::with_capacity(maximum.min(4_096));
    let truncated = loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(ListObservationError::Interrupted);
        }
        let entry = entries.next();
        if cancelled.load(Ordering::Acquire) {
            return Err(ListObservationError::Interrupted);
        }
        let Some(entry) = entry else {
            break false;
        };
        let name = entry.map_err(|_| ListObservationError::Failed)?;
        if name.as_bytes() == b"." || name.as_bytes() == b".." {
            continue;
        }
        if names.len() == maximum {
            break true;
        }
        names.push(name);
    };
    names.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    Ok(RetainedListNames { names, truncated })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ListedEntryKind {
    Directory,
    Regular,
    Excluded,
}

fn classify_list_entry(directory: &Dir, name: &OsStr) -> Result<ListedEntryKind, Errno> {
    let metadata = fstatat(directory, name, AtFlags::AT_SYMLINK_NOFOLLOW)?;
    let file_type = SFlag::from_bits_truncate(metadata.st_mode) & SFlag::S_IFMT;
    Ok(if file_type == SFlag::S_IFDIR {
        ListedEntryKind::Directory
    } else if file_type == SFlag::S_IFREG {
        ListedEntryKind::Regular
    } else {
        ListedEntryKind::Excluded
    })
}

fn render_list_names(
    names: Vec<OsString>,
    relative_directory: &Path,
    limit: usize,
    mut truncated: bool,
    cancelled: &AtomicBool,
    mut classify: impl FnMut(&OsStr) -> Result<ListedEntryKind, Errno>,
) -> ToolExecutionResult {
    let reserved_limit = limit.saturating_sub(LIST_TRUNCATION_MARKER.len());
    let mut complete_output = String::new();
    let mut reserved_output = String::new();
    let mut reserved_open = limit > LIST_TRUNCATION_MARKER.len();

    for name in names {
        if cancelled.load(Ordering::Acquire) {
            return interrupted();
        }
        if name.as_bytes() == b".git" {
            continue;
        }
        let Some(name) = name
            .to_str()
            .filter(|name| !name.chars().any(char::is_control))
        else {
            truncated = true;
            continue;
        };
        let kind = match classify(OsStr::new(name)) {
            Ok(kind) => kind,
            Err(Errno::ENOENT) => continue,
            Err(_) => return failed("list_files failed"),
        };
        if cancelled.load(Ordering::Acquire) {
            return interrupted();
        }
        let directory = matches!(kind, ListedEntryKind::Directory);
        if matches!(kind, ListedEntryKind::Excluded) {
            continue;
        }
        let relative = relative_directory.join(name);
        let token = relative
            .to_str()
            .expect("an admitted path joined with exact UTF-8 remains UTF-8");
        let token_len = token.len().saturating_add(usize::from(directory));
        if token_len > 1_024 || token.chars().any(char::is_control) {
            truncated = true;
            continue;
        }
        let line = if directory {
            format!("{token}/\n")
        } else {
            format!("{token}\n")
        };
        if reserved_open {
            if reserved_output.len().saturating_add(line.len()) <= reserved_limit {
                reserved_output.push_str(&line);
            } else {
                reserved_open = false;
            }
        }
        if complete_output.len().saturating_add(line.len()) > limit {
            if cancelled.load(Ordering::Acquire) {
                return interrupted();
            }
            return completed(reserved_output, true);
        }
        complete_output.push_str(&line);
    }
    if cancelled.load(Ordering::Acquire) {
        return interrupted();
    }
    if truncated {
        completed(reserved_output, true)
    } else {
        completed(complete_output, false)
    }
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

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        ffi::{OsStr, OsString},
        fs,
        io::{Read, Write},
        os::unix::{
            ffi::OsStringExt,
            fs::{PermissionsExt, symlink},
        },
        path::Path,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };

    use nix::errno::Errno;
    use yo_core::ToolExecutionHost;

    #[cfg(target_vendor = "apple")]
    use super::normalize_device_id;
    use super::{
        super::tests::{TestDirectory, finish, request},
        LIST_TRUNCATION_MARKER, ListObservationError, ListedEntryKind, LocalToolHost,
        MAX_LIST_ENTRIES, list_files, open_regular_file, read_bounded, read_file,
        render_list_names, retain_list_names,
    };
    use crate::local_tools::registry::{LocalToolRegistryRevision, registry};

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

    // legacy 4 MiB probe가 multi-byte scalar 한가운데서 끝나도 완전한 UTF-8 prefix를
    // Completed+truncated로 넘겨 common truncation marker가 붙을 수 있게 합니다.
    #[test]
    fn legacy_reader_truncates_only_an_incomplete_final_scalar() {
        let bytes = [b'a', 0xE2, 0x82, 0xAC];
        let result = read_file(&bytes[..], 3, &AtomicBool::new(false));
        assert_eq!(result.outcome(), yo_core::ToolExecutionOutcome::Completed);
        assert_eq!(result.output(), "a");
        assert!(result.truncated());

        let malformed = read_file(&[b'a', 0xFF, b'b'][..], 3, &AtomicBool::new(false));
        assert_eq!(malformed.outcome(), yo_core::ToolExecutionOutcome::Failed);
    }

    // legacy read_file은 workspace 안의 일반 파일을 읽되 credential path 실패를 고정된
    // execution result로 닫고, 잘못된 상위 경로는 worker 시작 전 거절합니다.
    #[test]
    fn reads_workspace_files_but_denies_the_credential_file() {
        let directory = TestDirectory::new();
        let source = directory.0.join("source.txt");
        let credential = directory.0.join("credentials.yaml");
        fs::write(&source, "hello").unwrap();
        let mut file = fs::File::create(&credential).unwrap();
        file.write_all(b"secret").unwrap();
        fs::set_permissions(&credential, fs::Permissions::from_mode(0o600)).unwrap();
        let registry = registry(LocalToolRegistryRevision::LegacyReadFile).unwrap();
        let mut host = LocalToolHost::new(&directory.0, &credential).unwrap();

        let mut execution = host
            .start(request(&registry, "read_file", r#"{"path":"source.txt"}"#))
            .unwrap();
        assert_eq!(finish(execution.as_mut()).output(), "hello");
        let mut denied = host
            .start(request(
                &registry,
                "read_file",
                r#"{"path":"credentials.yaml"}"#,
            ))
            .unwrap();
        let denied = finish(denied.as_mut());
        assert_eq!(denied.outcome(), yo_core::ToolExecutionOutcome::Failed);
        assert_eq!(denied.output(), "tool execution failed");
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

        let components = LocalToolHost::path_components("source.txt").unwrap();
        let file = open_regular_file(
            &host.workspace_directory,
            &components,
            host.denied_credential,
        )
        .unwrap();
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

    // list_files 경로는 file 경로와 같은 byte/control/traversal 경계를 사용하지만 `.`의
    // 정규화 결과인 workspace root는 허용해, root의 immediate child를 안전하게 나열합니다.
    #[test]
    fn list_path_admission_allows_root_and_rejects_ambiguous_inputs_before_open() {
        let workspace = TestDirectory::new();
        fs::write(workspace.0.join("root.txt"), "root").unwrap();
        fs::create_dir(workspace.0.join("nested")).unwrap();
        fs::write(workspace.0.join("nested/child.txt"), "child").unwrap();
        let host = LocalToolHost::new(&workspace.0, &workspace.0.join("credentials.yaml")).unwrap();
        let (directory, relative) = host.open_directory("./.").unwrap();
        let result = list_files(directory, relative, 1024, &AtomicBool::new(false));
        assert_eq!(result.output(), "nested/\nroot.txt\n");
        let (directory, relative) = host.open_directory("./nested/./").unwrap();
        let nested = list_files(directory, relative, 1024, &AtomicBool::new(false));
        assert_eq!(nested.output(), "nested/child.txt\n");
        assert!(LocalToolHost::basic_path(".").is_err());

        for rejected in ["", "../outside", "/absolute", "bad\nname"] {
            assert!(
                host.open_directory(rejected).is_err(),
                "accepted {rejected:?}"
            );
        }
        assert!(host.open_directory(&"a".repeat(1_025)).is_err());
    }

    // raw entry 한도는 정렬 전에 iteration 순서로 자르고 100001번째만 probe하므로,
    // 전역 정렬로 더 작은 뒤쪽 이름을 선택하거나 probe 뒤를 읽는 구현을 막습니다.
    #[test]
    fn raw_entry_budget_uses_one_probe_before_unsigned_sorting() {
        let cancelled = AtomicBool::new(false);
        let pulled = Cell::new(0_usize);
        let entries = [b".".as_slice(), b"z", b"a", b"b", b"unread"]
            .into_iter()
            .map(|name| {
                pulled.set(pulled.get() + 1);
                Ok(OsString::from_vec(name.to_vec()))
            });
        let retained = retain_list_names(entries, 2, &cancelled).unwrap();
        assert_eq!(retained.names, [OsString::from("a"), OsString::from("z")]);
        assert!(retained.truncated);
        assert_eq!(pulled.get(), 4);

        let exact = retain_list_names(
            (0..MAX_LIST_ENTRIES).map(|index| Ok(OsString::from(index.to_string()))),
            MAX_LIST_ENTRIES,
            &cancelled,
        )
        .unwrap();
        assert_eq!(exact.names.len(), MAX_LIST_ENTRIES);
        assert!(!exact.truncated);

        let probed = Cell::new(0_usize);
        let over = retain_list_names(
            (0..MAX_LIST_ENTRIES + 2).map(|index| {
                probed.set(probed.get() + 1);
                Ok(OsString::from(index.to_string()))
            }),
            MAX_LIST_ENTRIES,
            &cancelled,
        )
        .unwrap();
        assert_eq!(over.names.len(), MAX_LIST_ENTRIES);
        assert!(over.truncated);
        assert_eq!(probed.get(), MAX_LIST_ENTRIES + 1);
    }

    // `.`과 `..`만 budget 밖이며 `.git`은 retained slot을 소비한 뒤 분류 없이 빠져,
    // `.git`을 공짜 이름으로 취급해 다음 entry까지 노출하는 변형을 구분합니다.
    #[test]
    fn dot_git_consumes_the_raw_budget_without_classification() {
        let retained = retain_list_names(
            [".", "..", ".git", "visible"]
                .into_iter()
                .map(|name| Ok(OsString::from(name))),
            1,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(retained.names, [OsString::from(".git")]);
        assert!(retained.truncated);

        let calls = Cell::new(0_usize);
        let result = render_list_names(
            retained.names,
            Path::new(""),
            4096,
            retained.truncated,
            &AtomicBool::new(false),
            |_| {
                calls.set(calls.get() + 1);
                Ok(ListedEntryKind::Regular)
            },
        );
        assert_eq!(calls.get(), 0);
        assert!(result.output().is_empty());
        assert!(result.truncated());
    }

    // UTF-8로 표현할 수 없거나 control scalar가 든 raw name은 fstatat 전에 빠지고,
    // 정상 이름만 한 번 분류되어 lossy/escape 경로가 model output에 생기지 않습니다.
    #[test]
    fn unrepresentable_names_are_truncated_before_classification() {
        let classified = Cell::new(0_usize);
        let result = render_list_names(
            vec![
                OsString::from_vec(vec![0xff]),
                OsString::from("control\nname"),
                OsString::from("valid"),
            ],
            Path::new("selected"),
            4096,
            false,
            &AtomicBool::new(false),
            |name| {
                assert_eq!(name, OsStr::new("valid"));
                classified.set(classified.get() + 1);
                Ok(ListedEntryKind::Regular)
            },
        );
        assert_eq!(classified.get(), 1);
        assert_eq!(result.output(), "selected/valid\n");
        assert!(result.truncated());
    }

    // directory의 `/`까지 포함한 model-visible token은 1024 bytes를 허용하고 1025
    // bytes부터 생략+truncated로 바뀌어, LF만 제외한다는 경계를 고정합니다.
    #[test]
    fn rendered_directory_token_enforces_the_complete_byte_limit() {
        let at_limit = render_list_names(
            vec![OsString::from("b")],
            Path::new(&"a".repeat(1_021)),
            4096,
            false,
            &AtomicBool::new(false),
            |_| Ok(ListedEntryKind::Directory),
        );
        assert_eq!(at_limit.output().len(), 1_025);
        assert!(!at_limit.truncated());

        let over = render_list_names(
            vec![OsString::from("b")],
            Path::new(&"a".repeat(1_022)),
            4096,
            false,
            &AtomicBool::new(false),
            |_| Ok(ListedEntryKind::Directory),
        );
        assert!(over.output().is_empty());
        assert!(over.truncated());

        let regular = render_list_names(
            vec![OsString::from("é")],
            Path::new(&"a".repeat(1_021)),
            4096,
            false,
            &AtomicBool::new(false),
            |_| Ok(ListedEntryKind::Regular),
        );
        assert_eq!(regular.output().len(), 1_025);
        assert!(!regular.truncated());

        let regular_over = render_list_names(
            vec![OsString::from("é")],
            Path::new(&"a".repeat(1_022)),
            4096,
            false,
            &AtomicBool::new(false),
            |_| Ok(ListedEntryKind::Regular),
        );
        assert!(regular_over.output().is_empty());
        assert!(regular_over.truncated());
    }

    // ENOENT는 사라진 child 하나만 건너뛰지만, 이미 만든 줄 뒤의 EIO도 전체 결과를
    // exact Failed로 바꿔 partial output과 truncated 상태가 새지 않게 합니다.
    #[test]
    fn metadata_failure_discards_partial_output_but_enoent_skips_one_child() {
        let names = vec![OsString::from("a"), OsString::from("b")];
        let skipped = render_list_names(
            names.clone(),
            Path::new(""),
            4096,
            false,
            &AtomicBool::new(false),
            |name| {
                if name == "a" {
                    Err(Errno::ENOENT)
                } else {
                    Ok(ListedEntryKind::Regular)
                }
            },
        );
        assert_eq!(skipped.output(), "b\n");
        assert!(!skipped.truncated());

        let failed = render_list_names(
            names,
            Path::new(""),
            4096,
            false,
            &AtomicBool::new(false),
            |name| {
                if name == "b" {
                    Err(Errno::EIO)
                } else {
                    Ok(ListedEntryKind::Regular)
                }
            },
        );
        assert_eq!(failed.outcome(), yo_core::ToolExecutionOutcome::Failed);
        assert_eq!(failed.output(), "list_files failed");
        assert!(!failed.truncated());

        assert_eq!(
            retain_list_names(
                [Ok(OsString::from("a")), Err(Errno::EIO)].into_iter(),
                10,
                &AtomicBool::new(false),
            ),
            Err(ListObservationError::Failed)
        );
    }

    // 마지막 fstatat 동안 취소가 도착해도 publication 전 check가 이를 관찰하여,
    // 직전에 만든 정상 줄까지 버리고 exact Interrupted만 반환합니다.
    #[test]
    fn cancellation_after_the_last_classification_discards_output() {
        let cancelled = AtomicBool::new(false);
        let result = render_list_names(
            vec![OsString::from("first"), OsString::from("last")],
            Path::new(""),
            4096,
            false,
            &cancelled,
            |name| {
                if name == "last" {
                    cancelled.store(true, Ordering::Release);
                }
                Ok(ListedEntryKind::Regular)
            },
        );
        assert_eq!(result.outcome(), yo_core::ToolExecutionOutcome::Interrupted);
        assert_eq!(result.output(), "interrupted");
        assert!(!result.truncated());
    }

    // 불완전 결과는 common marker 전체를 먼저 예약하고 완전한 LF 줄만 넘기며,
    // tiny bound는 빈 worker prefix로 남겨 상위 bounded_output이 marker prefix만 만듭니다.
    #[test]
    fn incomplete_listing_reserves_the_exact_marker_without_cutting_lines() {
        assert_eq!(LIST_TRUNCATION_MARKER, "\n[yo: tool output truncated]");
        for limit in [0, 1, 27, 28, 29] {
            let result = render_list_names(
                vec![OsString::from("a")],
                Path::new(""),
                limit,
                true,
                &AtomicBool::new(false),
                |_| Ok(ListedEntryKind::Regular),
            );
            assert!(result.output().is_empty(), "limit {limit}");
            assert!(result.truncated());
        }
        for (limit, expected) in [
            (LIST_TRUNCATION_MARKER.len() + 1, ""),
            (LIST_TRUNCATION_MARKER.len() + 2, "a\n"),
            (LIST_TRUNCATION_MARKER.len() + 3, "a\n"),
        ] {
            let result = render_list_names(
                vec![OsString::from("a")],
                Path::new(""),
                limit,
                true,
                &AtomicBool::new(false),
                |_| Ok(ListedEntryKind::Regular),
            );
            assert_eq!(result.output(), expected, "limit {limit}");
            assert!(result.truncated());
        }

        let long = "x".repeat(20);
        let result = render_list_names(
            vec![
                OsString::from(format!("a{long}")),
                OsString::from(format!("b{long}")),
                OsString::from(format!("c{long}")),
            ],
            Path::new(""),
            LIST_TRUNCATION_MARKER.len() + 22,
            false,
            &AtomicBool::new(false),
            |_| Ok(ListedEntryKind::Regular),
        );
        assert_eq!(result.output(), format!("a{long}\n"));
        assert!(result.truncated());

        let exact = render_list_names(
            vec![OsString::from("a"), OsString::from("b")],
            Path::new(""),
            4,
            false,
            &AtomicBool::new(false),
            |_| Ok(ListedEntryKind::Regular),
        );
        assert_eq!(exact.output(), "a\nb\n");
        assert!(!exact.truncated());
    }

    // 선택 directory의 child는 fstatat만 한 번 호출해 regular와 directory를 표시하고,
    // nested content, symlink, FIFO, `.git`은 열거나 재귀 방문하지 않습니다.
    #[test]
    fn lists_only_immediate_children_without_opening_them() {
        let workspace = TestDirectory::new();
        let listed = workspace.0.join("listed");
        let nested = listed.join("nested");
        let visible = listed.join("visible.txt");
        fs::create_dir(&listed).unwrap();
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("hidden.txt"), "hidden").unwrap();
        fs::write(&visible, "visible").unwrap();
        fs::create_dir(listed.join(".git")).unwrap();
        symlink("visible.txt", listed.join("link")).unwrap();
        nix::unistd::mkfifo(&listed.join("pipe"), nix::sys::stat::Mode::S_IRUSR).unwrap();
        fs::set_permissions(&nested, fs::Permissions::from_mode(0o000)).unwrap();
        fs::set_permissions(&visible, fs::Permissions::from_mode(0o000)).unwrap();

        let host = LocalToolHost::new(&workspace.0, &workspace.0.join("credentials.yaml")).unwrap();
        let (directory, relative) = host.open_directory("listed").unwrap();
        let result = list_files(directory, relative, 1024, &AtomicBool::new(false));

        fs::set_permissions(&nested, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&visible, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(result.outcome(), yo_core::ToolExecutionOutcome::Completed);
        assert_eq!(result.output(), "listed/nested/\nlisted/visible.txt\n");
        assert!(!result.truncated());
    }
}
