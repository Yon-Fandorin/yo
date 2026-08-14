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
    sync::atomic::{AtomicBool, Ordering},
};

use nix::{
    dir::Dir,
    fcntl::{AtFlags, OFlag, openat},
    sys::stat::{Mode, SFlag, fstat, fstatat},
};
use serde_json::Value;
use yo_core::{
    ToolExecution, ToolExecutionError, ToolExecutionHost, ToolExecutionRequest,
    ToolExecutionResult, ToolId,
};

use super::{
    command::CommandExecution,
    execution::{ThreadExecution, completed, failed, interrupted},
};

const HOST_IDENTITY: &str = "yo.local-workspace-tools/v1";
const MAX_LIST_ENTRIES: usize = 100_000;

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
                    request.absolute_execution_timeout,
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
        LocalToolHost, list_files, read_bounded, read_file,
    };
    use crate::local_tools::registry::registry;

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
}
