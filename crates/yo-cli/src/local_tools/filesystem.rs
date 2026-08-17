use std::{
    ffi::OsString,
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
static NEW_FILE_MODE: OnceLock<u32> = OnceLock::new();

pub(crate) fn initialize_process_file_mode() {
    NEW_FILE_MODE.get_or_init(capture_new_file_mode);
}

pub(super) fn validate_arguments(
    definition: &ToolDefinition,
    arguments: &Value,
) -> Result<(), ToolExecutionError> {
    match definition.id().as_str() {
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

    fn basic_path(value: &str) -> Result<read::AdmittedPath, ToolExecutionError> {
        if value.len() > 1_024 || value.chars().any(char::is_control) {
            return Err(ToolExecutionError::new(
                "tool path exceeds its byte bound or contains a control character",
            ));
        }
        let components = Self::path_components(value)?;
        if components.is_empty() {
            return Err(ToolExecutionError::new(
                "tool file path must not name the workspace root",
            ));
        }
        Ok(read::AdmittedPath::new(value.to_owned(), components))
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

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{Read, Write},
        os::unix::fs::{PermissionsExt, symlink},
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };

    use yo_core::ToolExecutionHost;

    #[cfg(target_vendor = "apple")]
    use super::normalize_device_id;
    use super::{
        super::tests::{TestDirectory, finish, request},
        LocalToolHost, list_files, open_regular_file, read_bounded, read_file,
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
}
