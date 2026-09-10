//! Descriptor-pinned primary artifacts and one bounded verification pass.

use std::{
    collections::{HashMap, VecDeque},
    ffi::OsString,
    fs::{File, Metadata, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

use nix::{
    errno::Errno,
    fcntl::{OFlag, openat, readlinkat},
    sys::stat::Mode,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use yo_core::ToolExecutionError;

use super::{encoding::tagged_hash, invalid};

const EXECUTABLE_LIMIT: u64 = 128 * 1024 * 1024;
const SCRIPT_LIMIT: u64 = 16 * 1024 * 1024;
const STARTUP_LIMIT: u64 = 256 * 1024 * 1024;
const VERIFICATION_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Artifact {
    pub(super) configured: String,
    pub(super) resolved: String,
    sha256: String,
}

impl Artifact {
    pub(super) fn manifest(&self) -> Value {
        json!({"configured": self.configured, "resolved": self.resolved, "sha256": self.sha256})
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(super) struct FileIdentity(u64, u64);

#[derive(Clone, Debug, Eq, PartialEq)]
struct Signature {
    identity: FileIdentity,
    length: u64,
    mode: u32,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl Signature {
    fn new(metadata: &Metadata) -> Self {
        Self {
            identity: FileIdentity(metadata.dev(), metadata.ino()),
            length: metadata.len(),
            mode: metadata.mode(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        }
    }
}

struct CapturedFile {
    signature: Signature,
    bytes: u64,
    sha256: String,
}

pub(super) struct VerificationPass<'a> {
    cancelled: &'a mut dyn FnMut() -> bool,
    started: Instant,
    absolute_deadline: Option<Instant>,
    remaining: u64,
    captured: HashMap<FileIdentity, CapturedFile>,
}

impl<'a> VerificationPass<'a> {
    pub(super) fn startup(
        cancelled: &'a mut dyn FnMut() -> bool,
        absolute_deadline: Option<Instant>,
    ) -> Self {
        Self {
            cancelled,
            started: Instant::now(),
            absolute_deadline,
            remaining: STARTUP_LIMIT,
            captured: HashMap::new(),
        }
    }

    pub(super) fn call(
        cancelled: &'a mut dyn FnMut() -> bool,
        absolute_deadline: Option<Instant>,
    ) -> Self {
        Self {
            remaining: EXECUTABLE_LIMIT + SCRIPT_LIMIT,
            ..Self::startup(cancelled, absolute_deadline)
        }
    }

    pub(super) fn check(&mut self) -> Result<(), ToolExecutionError> {
        if (self.cancelled)() {
            return Err(invalid("command tool verification cancelled"));
        }
        if self.started.elapsed() >= VERIFICATION_TIMEOUT
            || self
                .absolute_deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(invalid("command tool verification deadline expired"));
        }
        Ok(())
    }

    pub(super) fn workspace(&mut self, path: &Path) -> Result<PathBuf, ToolExecutionError> {
        self.check()?;
        let resolved = path
            .canonicalize()
            .map_err(|_| invalid("command tool workspace is unavailable"))?;
        locator(&resolved)?;
        self.check()?;
        Ok(resolved)
    }

    pub(super) fn credential(
        &mut self,
        path: &Path,
    ) -> Result<Option<FileIdentity>, ToolExecutionError> {
        self.check()?;
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(invalid("command tool credential identity is unavailable")),
        };
        let metadata = file
            .metadata()
            .map_err(|_| invalid("command tool credential identity is unavailable"))?;
        if !metadata.is_file() {
            return Err(invalid("command tool credential identity is unavailable"));
        }
        self.check()?;
        Ok(Some(FileIdentity(metadata.dev(), metadata.ino())))
    }

    pub(super) fn capture(
        &mut self,
        configured: &str,
        workspace: &Path,
        executable: bool,
        denied: Option<FileIdentity>,
    ) -> Result<Artifact, ToolExecutionError> {
        self.check()?;
        let (mut file, resolved) = self.open(configured, workspace, executable)?;
        let before = file
            .metadata()
            .map_err(|_| invalid("command tool artifact metadata is unavailable"))?;
        if !before.is_file() || (executable && before.mode() & 0o111 == 0) {
            return Err(invalid(
                "command tool artifact has an invalid file type or mode",
            ));
        }
        let signature = Signature::new(&before);
        if Some(signature.identity) == denied {
            return Err(invalid("command tool artifact is unavailable"));
        }
        let limit = if executable {
            EXECUTABLE_LIMIT
        } else {
            SCRIPT_LIMIT
        };
        let sha256 = if let Some(previous) = self.captured.get(&signature.identity) {
            if previous.signature != signature || previous.bytes > limit {
                return Err(invalid(
                    "command tool artifact changed or exceeds its bound",
                ));
            }
            previous.sha256.clone()
        } else {
            let (bytes, sha256) = self.hash_file(&mut file, limit)?;
            let after = file
                .metadata()
                .map_err(|_| invalid("command tool artifact metadata is unavailable"))?;
            if Signature::new(&after) != signature || bytes != signature.length {
                return Err(invalid("command tool artifact changed during verification"));
            }
            self.captured.insert(
                signature.identity,
                CapturedFile {
                    signature: signature.clone(),
                    bytes,
                    sha256: sha256.clone(),
                },
            );
            sha256
        };
        // Reobserve the configured mapping after reading; a changed symlink/entry cannot be frozen.
        let (again, resolved_again) = self.open(configured, workspace, executable)?;
        let after = again
            .metadata()
            .map_err(|_| invalid("command tool artifact metadata is unavailable"))?;
        if resolved != resolved_again || Signature::new(&after) != signature {
            return Err(invalid(
                "command tool artifact mapping changed during verification",
            ));
        }
        self.check()?;
        Ok(Artifact {
            configured: configured.to_owned(),
            resolved,
            sha256,
        })
    }

    fn hash_file(
        &mut self,
        file: &mut impl Read,
        limit: u64,
    ) -> Result<(u64, String), ToolExecutionError> {
        let mut hash = Sha256::new();
        let mut total = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            self.check()?;
            let next = (limit - total)
                .min(self.remaining)
                .saturating_add(1)
                .min(buffer.len() as u64) as usize;
            let count = file
                .read(&mut buffer[..next])
                .map_err(|_| invalid("command tool artifact could not be read"))?;
            self.check()?;
            if count == 0 {
                break;
            }
            total += count as u64;
            if total > limit || count as u64 > self.remaining {
                return Err(invalid("command tool artifact exceeds its byte budget"));
            }
            self.remaining -= count as u64;
            hash.update(&buffer[..count]);
        }
        Ok((total, tagged_hash(hash)))
    }

    fn open(
        &mut self,
        configured: &str,
        workspace: &Path,
        executable: bool,
    ) -> Result<(File, String), ToolExecutionError> {
        locator(Path::new(configured))?;
        if executable && !Path::new(configured).is_absolute() {
            return Err(invalid("command tool executable must be absolute"));
        }
        let relative_script = !Path::new(configured).is_absolute();
        let mut resolved = PathBuf::from("/");
        let floor = if relative_script {
            workspace.components().count()
        } else {
            1
        };
        let mut directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(&resolved)
            .map_err(|_| invalid("command tool artifact path is unavailable"))?;
        let mut pending = if relative_script {
            path_parts(workspace)
        } else {
            VecDeque::new()
        };
        pending.extend(path_parts(Path::new(configured)));
        let mut links = 0usize;
        while let Some(component) = pending.pop_front() {
            self.check()?;
            if component == "." {
                continue;
            }
            if component == ".." {
                if relative_script && resolved.components().count() <= floor {
                    return Err(invalid("command tool script escapes its workspace"));
                }
                let fd = openat(
                    &directory,
                    Path::new(".."),
                    OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
                    Mode::empty(),
                )
                .map_err(|_| invalid("command tool artifact path is unavailable"))?;
                directory = File::from(fd);
                resolved.pop();
                continue;
            }
            match readlinkat(&directory, component.as_os_str()) {
                Ok(target) => {
                    if !executable || links == 40 {
                        return Err(invalid("command tool artifact symlink is not admitted"));
                    }
                    links += 1;
                    let target = Path::new(&target);
                    if target.is_absolute() {
                        directory = File::open("/")
                            .map_err(|_| invalid("command tool artifact path is unavailable"))?;
                        resolved = PathBuf::from("/");
                    }
                    let mut parts = path_parts(target);
                    parts.append(&mut pending);
                    pending = parts;
                },
                Err(Errno::EINVAL) => {
                    let final_component = pending.is_empty();
                    let flags = OFlag::O_RDONLY
                        | OFlag::O_CLOEXEC
                        | OFlag::O_NOFOLLOW
                        | OFlag::O_NONBLOCK
                        | if final_component {
                            OFlag::empty()
                        } else {
                            OFlag::O_DIRECTORY
                        };
                    let fd = openat(&directory, component.as_os_str(), flags, Mode::empty())
                        .map_err(|_| invalid("command tool artifact path is unavailable"))?;
                    resolved.push(&component);
                    locator(&resolved)?;
                    if final_component {
                        return Ok((File::from(fd), locator(&resolved)?.to_owned()));
                    }
                    directory = File::from(fd);
                },
                Err(_) => return Err(invalid("command tool artifact path is unavailable")),
            }
        }
        Err(invalid("command tool artifact must name a regular file"))
    }
}

fn path_parts(path: &Path) -> VecDeque<OsString> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_owned()),
            Component::ParentDir => Some(OsString::from("..")),
            Component::CurDir => Some(OsString::from(".")),
            Component::RootDir | Component::Prefix(_) => None,
        })
        .collect()
}

fn locator(path: &Path) -> Result<&str, ToolExecutionError> {
    path.to_str()
        .filter(|value| {
            !value.is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control)
        })
        .ok_or_else(|| invalid("command tool artifact locator is invalid"))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    // 실제 read budget은 한도까지와 EOF를 허용하고 첫 초과 byte에서 멈춘다.
    #[test]
    fn verification_counts_first_excess_and_shares_the_pass_budget() {
        let mut cancelled = || false;
        let mut pass = VerificationPass::startup(&mut cancelled, None);
        pass.remaining = 4;
        let mut exact = Cursor::new(b"abcd");
        assert!(pass.hash_file(&mut exact, 4).is_ok());
        assert_eq!(pass.remaining, 0);
        let mut excess = Cursor::new(b"xy");
        assert!(pass.hash_file(&mut excess, 4).is_err());
        assert_eq!(excess.position(), 1);
        let mut pass = VerificationPass::startup(&mut cancelled, None);
        let mut excess = Cursor::new(b"abcdef");
        assert!(pass.hash_file(&mut excess, 4).is_err());
        assert_eq!(excess.position(), 5);
    }

    // 검증 30초와 호출 전체 deadline은 다음 read 전에 검사하고 cancellation을 우선한다.
    #[test]
    fn verification_deadlines_and_cancellation_prevent_reads() {
        let mut cancelled = || false;
        let mut pass = VerificationPass::call(&mut cancelled, Some(Instant::now()));
        let mut input = Cursor::new(b"data");
        assert!(pass.hash_file(&mut input, 4).is_err());
        assert_eq!(input.position(), 0);
        let mut pass = VerificationPass::startup(&mut cancelled, None);
        pass.started = Instant::now() - VERIFICATION_TIMEOUT;
        assert!(pass.hash_file(&mut input, 4).is_err());
        assert_eq!(input.position(), 0);
        let mut cancelled = || true;
        let mut pass = VerificationPass::startup(&mut cancelled, None);
        assert!(pass.hash_file(&mut input, 4).is_err());
        assert_eq!(input.position(), 0);
    }
}
