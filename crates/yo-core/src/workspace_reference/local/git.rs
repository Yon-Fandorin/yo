use std::{
    collections::{BTreeSet, HashSet},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use super::super::WorkspaceReferenceKind;

pub(super) fn is_git_workspace(root: &Path) -> Result<bool, String> {
    if !has_git_marker(root)? {
        return Ok(false);
    }
    let output = git_command(root)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map_err(|error| format!("starting Git workspace detection failed: {error}"))?;
    classify_git_workspace(output.status.success(), &output.stdout, &output.stderr)
}

pub(super) fn git_command(root: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(root)
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
                let output = git_command(root)
                    .arg("rev-parse")
                    .arg("--resolve-git-dir")
                    .arg(&marker)
                    .output()
                    .map_err(|error| format!("validating a Git marker failed: {error}"))?;
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
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
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
        let mut current = PathBuf::new();
        let mut valid = true;
        let components = relative.components().collect::<Vec<_>>();
        for (index, component) in components.iter().enumerate() {
            current.push(component.as_os_str());
            let metadata = match std::fs::symlink_metadata(root.join(&current)) {
                Ok(metadata) => metadata,
                Err(_) => {
                    valid = false;
                    break;
                },
            };
            if metadata.file_type().is_symlink() {
                valid = false;
                break;
            }
            let normalized = current
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            if index + 1 == components.len() {
                if metadata.is_file() {
                    visible.insert((normalized, WorkspaceReferenceKind::File));
                }
            } else if metadata.is_dir() {
                visible.insert((normalized, WorkspaceReferenceKind::Directory));
            } else {
                valid = false;
                break;
            }
        }
        if !valid {
            continue;
        }
    }
    Ok((visible, incomplete))
}

pub(super) fn ignored_paths(root: &Path, paths: &[PathBuf]) -> Result<HashSet<String>, String> {
    if paths.is_empty() {
        return Ok(HashSet::new());
    }
    let mut child = git_command(root)
        .args(["check-ignore", "--stdin", "-z"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("starting git check-ignore failed: {error}"))?;
    {
        let stdin = child.stdin.as_mut().expect("piped Git stdin is available");
        for path in paths {
            stdin
                .write_all(path.as_os_str().as_encoded_bytes())
                .and_then(|()| stdin.write_all(&[0]))
                .map_err(|error| format!("sending paths to git check-ignore failed: {error}"))?;
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("waiting for git check-ignore failed: {error}"))?;
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
    let output = git_command(root)
        .args(args)
        .output()
        .map_err(|error| format!("starting Git workspace discovery failed: {error}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}
