use std::{
    collections::BTreeMap,
    env,
    ffi::OsStr,
    fs,
    io::ErrorKind,
    os::unix::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
    process::Command,
    str,
    time::{Duration, Instant},
};

use rustix::{
    fd::OwnedFd,
    fs::{AtFlags, Dir, FileType, Mode, OFlags, open, statat},
};

use super::{LocalWorkspaceReferenceProvider, discovery::pin_directory, git::git_command};

impl LocalWorkspaceReferenceProvider {
    /// Execution-host discovery command for local output navigation, never run by rendering.
    pub fn output_link_command(root: &Path) -> Command {
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
        for (key, _) in env::vars_os() {
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
    ) -> Result<BTreeMap<String, PathBuf>, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        if cancelled() {
            return Err("link discovery cancelled".into());
        }
        if fs::canonicalize(root).ok().as_deref() != Some(root) {
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
                    str::from_utf8(candidate).map_err(|_| "non-UTF-8 link inventory")?,
                ));
                if paths.len() > 8192 {
                    return Err("link inventory exceeds entry limit".into());
                }
            }
        } else {
            for ancestor in root.ancestors() {
                match fs::symlink_metadata(ancestor.join(".git")) {
                    Ok(_) => return Err("Git discovery unavailable".into()),
                    Err(error) if error.kind() == ErrorKind::NotFound => {},
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
                let Ok(metadata) = fs::symlink_metadata(&path) else {
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
            let Ok(canonical) = fs::canonicalize(&path) else {
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
        if fs::canonicalize(root).ok().as_deref() != Some(root) {
            return Err("workspace root changed".into());
        }
        Ok(links)
    }

    fn output_directory(root: &OwnedFd, relative: &Path) -> Result<OwnedFd, String> {
        pin_directory(root, relative).map_err(|error| error.to_string())
    }
}
