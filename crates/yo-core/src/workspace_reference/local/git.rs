use std::{
    collections::{BTreeSet, HashSet},
    io::{self, Read, Write},
    os::unix::process::CommandExt,
    path::{Component, Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use rustix::{
    fd::{AsFd, OwnedFd},
    fs::{AtFlags, FileType, OFlags, fcntl_getfl, fcntl_setfl, statat},
    process::{Pid, Signal, kill_process_group},
};

use super::{super::WorkspaceReferenceKind, DiscoveryBudget, pin_directory};

pub(super) fn is_git_workspace(root: &Path) -> Result<bool, String> {
    if !has_git_marker(root)? {
        return Ok(false);
    }
    let output = run_git(
        git_command(root).args(["rev-parse", "--is-inside-work-tree"]),
        &[],
    )?;
    classify_git_workspace(output.status.success(), &output.stdout, &output.stderr)
}

pub(super) fn git_command(root: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .args(["-c", "core.fsmonitor=false"])
        .env("LC_ALL", "C")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_IMPLICIT_WORK_TREE")
        .env_remove("GIT_GRAFT_FILE")
        .env_remove("GIT_NO_REPLACE_OBJECTS")
        .env_remove("GIT_REPLACE_REF_BASE")
        .env_remove("GIT_PREFIX")
        .env_remove("GIT_SHALLOW_FILE")
        .env_remove("GIT_CEILING_DIRECTORIES")
        .env_remove("GIT_DISCOVERY_ACROSS_FILESYSTEM");
    command
}

fn has_git_marker(root: &Path) -> Result<bool, String> {
    for ancestor in root.ancestors() {
        let marker = ancestor.join(".git");
        match std::fs::symlink_metadata(&marker) {
            Ok(_) => {
                let output = run_git(
                    git_command(root)
                        .args(["rev-parse", "--resolve-git-dir"])
                        .arg(&marker),
                    &[],
                )
                .map_err(|error| {
                    format!(
                        "validating the Git marker at {} failed: {error}",
                        marker.display()
                    )
                })?;
                if !output.status.success() {
                    let diagnostic = String::from_utf8_lossy(&output.stderr);
                    let diagnostic = diagnostic.trim();
                    return Err(if diagnostic.is_empty() {
                        format!("the Git marker at {} is invalid", marker.display())
                    } else {
                        format!(
                            "validating the Git marker at {} failed: {diagnostic}",
                            marker.display()
                        )
                    });
                }
                return Ok(true);
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
            Err(error) => {
                return Err(format!(
                    "checking the Git marker at {} failed: {error}",
                    marker.display()
                ));
            },
        }
    }
    Ok(false)
}

pub(super) fn classify_git_workspace(
    success: bool,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<bool, String> {
    if success {
        return match stdout.trim_ascii() {
            b"true" => Ok(true),
            b"false" => Ok(false),
            other => Err(format!(
                "Git workspace detection returned an unexpected value: {}",
                String::from_utf8_lossy(other)
            )),
        };
    }
    let diagnostic = String::from_utf8_lossy(stderr);
    let diagnostic = diagnostic.trim();
    Err(if diagnostic.is_empty() {
        "Git workspace detection failed without a diagnostic".to_owned()
    } else {
        format!("Git workspace detection failed: {diagnostic}")
    })
}

pub(super) fn discover_tracked_entries(
    root: &Path,
    root_descriptor: &OwnedFd,
    budget: &mut DiscoveryBudget,
) -> Result<(BTreeSet<(String, WorkspaceReferenceKind)>, bool), String> {
    let output = git_output(root, ["ls-files", "-z", "--cached"])?;
    let mut visible = BTreeSet::new();
    let mut incomplete = false;
    for raw_path in output
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let Ok(path) = std::str::from_utf8(raw_path) else {
            incomplete = true;
            continue;
        };
        let relative = Path::new(path);
        if !budget.admit(relative) {
            incomplete = true;
            break;
        }
        let components = relative.components().collect::<Vec<_>>();
        if components.is_empty()
            || components
                .iter()
                .any(|component| !matches!(component, Component::Normal(name) if *name != ".git"))
        {
            incomplete = true;
            continue;
        }
        let parent = relative.parent().unwrap_or_else(|| Path::new(""));
        let descriptor = match pin_directory(root_descriptor, parent) {
            Ok(descriptor) => descriptor,
            Err(_) => {
                incomplete = true;
                continue;
            },
        };
        let metadata = match statat(
            &descriptor,
            relative.file_name().expect("validated basename"),
            AtFlags::SYMLINK_NOFOLLOW,
        ) {
            Ok(metadata) => metadata,
            Err(_) => {
                incomplete = true;
                continue;
            },
        };
        if FileType::from_raw_mode(metadata.st_mode) != FileType::RegularFile {
            continue;
        }
        visible.insert((path.to_owned(), WorkspaceReferenceKind::File));
        // Publish parents only after the entire tracked file path is verified.
        for ancestor in parent
            .ancestors()
            .filter(|path| !path.as_os_str().is_empty())
        {
            if !budget.admit(ancestor) {
                incomplete = true;
                break;
            }
            visible.insert((
                ancestor.to_string_lossy().into_owned(),
                WorkspaceReferenceKind::Directory,
            ));
        }
        if budget.exhausted {
            break;
        }
    }
    Ok((visible, incomplete))
}

pub(super) fn ignored_paths(root: &Path, paths: &[PathBuf]) -> Result<HashSet<String>, String> {
    if paths.is_empty() {
        return Ok(HashSet::new());
    }
    let output = run_git(
        git_command(root).args(["check-ignore", "--stdin", "-z"]),
        paths,
    )?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter_map(|path| std::str::from_utf8(path).ok())
        .map(|path| path.replace(std::path::MAIN_SEPARATOR, "/"))
        .collect())
}

fn git_output<const N: usize>(root: &Path, args: [&str; N]) -> Result<Vec<u8>, String> {
    let output = run_git(git_command(root).args(args), &[])?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

struct GitProcess(Child);

impl Drop for GitProcess {
    fn drop(&mut self) {
        if let Ok(raw) = i32::try_from(self.0.id())
            && let Some(pid) = Pid::from_raw(raw)
        {
            let _ = kill_process_group(pid, Signal::KILL);
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn nonblocking(fd: impl AsFd) -> io::Result<()> {
    fcntl_setfl(&fd, fcntl_getfl(&fd)? | OFlags::NONBLOCK)?;
    Ok(())
}

// Each pipe gets bounded work per poll, so a chatty process cannot starve the deadline.
fn drain(reader: &mut impl Read, bytes: &mut Vec<u8>, limit: usize) -> io::Result<bool> {
    let mut buffer = [0; 8192];
    for _ in 0..8 {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                if bytes.len() + count > limit {
                    return Err(io::Error::other(
                        "Git discovery output exceeds its byte limit",
                    ));
                }
                bytes.extend_from_slice(&buffer[..count]);
            },
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(false)
}

fn run_git(command: &mut Command, paths: &[PathBuf]) -> Result<Output, String> {
    let mut run = || -> io::Result<Output> {
        let mut input = Vec::new();
        for path in paths {
            let bytes = path.as_os_str().as_encoded_bytes();
            if input.len() + bytes.len() + 1 > 32 * 1024 * 1024 {
                return Err(io::Error::other(
                    "Git discovery input exceeds its byte limit",
                ));
            }
            input.extend_from_slice(bytes);
            input.push(0);
        }
        let mut child = GitProcess(
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .process_group(0)
                .spawn()?,
        );
        let mut stdout = child.0.stdout.take().expect("piped Git stdout");
        let mut stderr = child.0.stderr.take().expect("piped Git stderr");
        nonblocking(&stdout)?;
        nonblocking(&stderr)?;
        nonblocking(child.0.stdin.as_ref().expect("piped Git stdin"))?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut written = 0;
        let mut input_error = None;
        let mut stdout_bytes = Vec::new();
        let mut stderr_bytes = Vec::new();
        let mut stdout_closed = false;
        let mut stderr_closed = false;
        let mut status = None;
        loop {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Git discovery exceeded 5 seconds",
                ));
            }
            let progress = (written, stdout_bytes.len(), stderr_bytes.len());
            if written == input.len() {
                child.0.stdin.take();
            } else if let Some(stdin) = child.0.stdin.as_mut() {
                let end = input.len().min(written + 8192);
                match stdin.write(&input[written..end]) {
                    Ok(count) => written += count,
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) => {},
                    Err(error) => {
                        input_error = Some(error);
                        child.0.stdin.take();
                    },
                }
            }
            if !stdout_closed {
                stdout_closed = drain(&mut stdout, &mut stdout_bytes, 16 * 1024 * 1024)?;
            }
            if !stderr_closed {
                stderr_closed = drain(&mut stderr, &mut stderr_bytes, 64 * 1024)?;
            }
            if status.is_none() {
                status = child.0.try_wait()?;
            }
            if let Some(status) = status
                && stdout_closed
                && stderr_closed
            {
                if status.success()
                    && let Some(error) = input_error
                {
                    return Err(error);
                }
                return Ok(Output {
                    status,
                    stdout: stdout_bytes,
                    stderr: stderr_bytes,
                });
            }
            if progress == (written, stdout_bytes.len(), stderr_bytes.len()) {
                thread::sleep(Duration::from_millis(2));
            }
        }
    };
    run().map_err(|error| format!("Git workspace discovery failed: {error}"))
}
