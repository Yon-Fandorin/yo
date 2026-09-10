//! Explicit, read-only clipboard acquisition on the existing image worker.

use std::{
    env, fs,
    io::{self, Read},
    net::Shutdown,
    num::NonZeroU16,
    os::{
        fd::{AsFd, AsRawFd},
        unix::{
            fs::{FileTypeExt, MetadataExt},
            net::UnixStream,
            process::CommandExt as _,
        },
    },
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use nix::{
    errno::Errno,
    fcntl::{FcntlArg, FdFlag, OFlag, fcntl},
    sys::{
        signal::{Signal, killpg},
        socket::{AddressFamily, SockFlag, SockType, UnixAddr, connect, socket},
    },
    unistd::{Pid, geteuid},
};

use super::{AppError, MAX_SOURCE_BYTES, PreparedInputImage, check_cancelled, prepare_source};
use crate::state::config::{ClipboardReader, ClipboardSource};

const ACQUISITION_TIMEOUT: Duration = Duration::from_secs(3);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(super) fn prepare(
    source: Option<&ClipboardSource>,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<PreparedInputImage, AppError> {
    check_cancelled(cancelled)?;
    let deadline = Instant::now() + ACQUISITION_TIMEOUT;
    let bytes = match env::var_os("YO_CLIPBOARD_IMAGE_SOCKET") {
        Some(path) => read_socket(Path::new(&path), deadline, cancelled)?,
        None => read_selected(source, deadline, cancelled)?,
    };
    prepare_png(&bytes, cancelled)
}

fn read_selected(
    source: Option<&ClipboardSource>,
    deadline: Instant,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<Vec<u8>, AppError> {
    match source {
        None | Some(ClipboardSource::Native) => read_native(deadline, cancelled),
        Some(ClipboardSource::Socket { path }) => read_socket(path, deadline, cancelled),
        Some(ClipboardSource::Ssh {
            host,
            reader,
            identity_file,
            known_hosts_file,
            port,
        }) => {
            let mut command = ssh_command(
                host,
                *reader,
                identity_file.as_deref(),
                known_hosts_file.as_deref(),
                *port,
            );
            match read_command(&mut command, deadline, cancelled) {
                Ok(bytes) => Ok(bytes),
                Err(CommandError::Missing) => Err(AppError::message(
                    "SSH clipboard needs the ssh client installed on the yo host.",
                )),
                Err(CommandError::Failed(error)) => Err(AppError::message(format!(
                    "SSH clipboard: {error} Check key authentication, the trusted host key, and Python 3 plus the selected clipboard reader on the source computer."
                ))),
            }
        },
    }
}

fn ssh_command(
    host: &str,
    reader: ClipboardReader,
    identity_file: Option<&Path>,
    known_hosts_file: Option<&Path>,
    port: Option<NonZeroU16>,
) -> Command {
    let mut command = Command::new("ssh");
    command.args([
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        "ClearAllForwardings=yes",
        "-o",
        "ForwardAgent=no",
        "-o",
        "ForwardX11=no",
        "-o",
        "ControlMaster=no",
        "-o",
        "ControlPath=none",
        "-o",
        "ForkAfterAuthentication=no",
        "-o",
        "SessionType=default",
        "-o",
        "RemoteCommand=none",
        "-o",
        "PermitLocalCommand=no",
        "-o",
        "ConnectionAttempts=1",
        "-o",
        "ConnectTimeout=2",
    ]);
    if let Some(identity) = identity_file {
        command
            .args(["-o", "IdentitiesOnly=yes", "-o", "IdentityAgent=none", "-i"])
            .arg(identity);
    }
    if let Some(path) = known_hosts_file {
        // OpenSSH parses this option as a filename list even inside one argv.
        let quoted = path
            .as_os_str()
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        command
            .arg("-o")
            .arg(format!("UserKnownHostsFile=\"{quoted}\""));
        command.args(["-o", "GlobalKnownHostsFile=/dev/null"]);
    }
    if let Some(port) = port {
        command.arg("-p").arg(port.to_string());
    }
    let reader = match reader {
        ClipboardReader::Macos => "macos",
        ClipboardReader::Wayland => "wayland",
        ClipboardReader::X11 => "x11",
    };
    let supervisor = include_str!("clipboard/ssh_capture.py").replace('\'', "'\\''");
    // Only fixed program text and a closed reader tag enter the remote shell.
    // Each explicit paste makes a fresh connection; no helper or tunnel persists.
    command.arg("--").arg(host).arg(format!(
        "exec env PATH=/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin python3 -c '{supervisor}' {reader} {MAX_SOURCE_BYTES}"
    ));
    command
}

fn prepare_png(
    bytes: &[u8],
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<PreparedInputImage, AppError> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(AppError::message(
            "Clipboard did not provide a PNG image. Copy an image and try again.",
        ));
    }
    prepare_source(bytes, cancelled)
}

fn read_native(
    deadline: Instant,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<Vec<u8>, AppError> {
    #[cfg(target_os = "linux")]
    let candidates = [
        (
            env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty()),
            "wl-paste",
            &["--no-newline", "--type", "image/png"][..],
        ),
        (
            env::var_os("DISPLAY").is_some_and(|value| !value.is_empty()),
            "xclip",
            &["-selection", "clipboard", "-t", "image/png", "-o"][..],
        ),
    ];
    #[cfg(target_os = "macos")]
    let candidates = [(true, "pngpaste", &["-"][..])];
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let candidates: [(bool, &str, &[&str]); 0] = [];

    let mut missing = false;
    for (enabled, program, args) in candidates {
        if !enabled {
            continue;
        }
        check_deadline(deadline, cancelled)?;
        match read_command(Command::new(program).args(args), deadline, cancelled) {
            Err(CommandError::Missing) => missing = true,
            Err(CommandError::Failed(error)) => return Err(error),
            Ok(bytes) => return Ok(bytes),
        }
    }
    Err(AppError::message(if missing {
        "Clipboard image reader is missing. Install wl-clipboard (Wayland), xclip (X11), or pngpaste (macOS), then try again."
    } else {
        "No desktop clipboard is available in this process. For SSH, configure YO_CLIPBOARD_IMAGE_SOCKET with your forwarded clipboard socket."
    }))
}

enum CommandError {
    Missing,
    Failed(AppError),
}

impl From<AppError> for CommandError {
    fn from(error: AppError) -> Self {
        Self::Failed(error)
    }
}

// Drop owns the child even when nonblocking setup, reads, or cancellation fail.
struct ReaderChild(Child);

impl Drop for ReaderChild {
    fn drop(&mut self) {
        // SSH proxy commands and other descendants share this owned group.
        let _ = killpg(Pid::from_raw(self.0.id() as i32), Signal::SIGKILL);
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn read_command(
    command: &mut Command,
    deadline: Instant,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<Vec<u8>, CommandError> {
    check_deadline(deadline, cancelled)?;
    let child = command
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                CommandError::Missing
            } else {
                CommandError::Failed(AppError::message(
                    "Clipboard image reader could not start. Check the desktop clipboard setup.",
                ))
            }
        })?;
    let mut child = ReaderChild(child);
    let mut stdout = child.0.stdout.take().expect("clipboard stdout is piped");
    set_nonblocking(&stdout).map_err(|_| read_error())?;
    let bytes = read_bounded(&mut stdout, deadline, cancelled)?;
    loop {
        check_deadline(deadline, cancelled)?;
        match child.0.try_wait().map_err(|_| read_error())? {
            Some(status) if status.success() && !bytes.is_empty() => return Ok(bytes),
            Some(status) if status.success() => return Err(AppError::message("Clipboard contains no PNG image. Copy an image and try again.").into()),
            Some(_) => return Err(AppError::message("Clipboard reader could not provide a PNG image. Copy an image and check desktop clipboard access, then try again.").into()),
            None => thread::park_timeout(POLL_INTERVAL),
        }
    }
}

fn set_nonblocking(fd: &impl AsFd) -> io::Result<()> {
    let flags = fcntl(fd, FcntlArg::F_GETFL)?;
    fcntl(
        fd,
        FcntlArg::F_SETFL(OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK),
    )?;
    Ok(())
}

fn read_bounded(
    reader: &mut impl Read,
    deadline: Instant,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<Vec<u8>, AppError> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        check_deadline(deadline, cancelled)?;
        let remaining = (MAX_SOURCE_BYTES - bytes.len() + 1).min(chunk.len());
        match reader.read(&mut chunk[..remaining]) {
            Ok(0) => return Ok(bytes),
            Ok(count) if count > MAX_SOURCE_BYTES - bytes.len() => {
                return Err(AppError::message(
                    "Clipboard image exceeds the 4 MiB source limit.",
                ));
            },
            Ok(count) => bytes.extend_from_slice(&chunk[..count]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {},
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::park_timeout(POLL_INTERVAL)
            },
            Err(_) => return Err(read_error()),
        }
    }
}

fn read_socket(
    path: &Path,
    deadline: Instant,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<Vec<u8>, AppError> {
    check_deadline(deadline, cancelled)?;
    validate_socket(path)?;
    let address = UnixAddr::new(path).map_err(|_| socket_error())?;
    #[cfg(target_os = "linux")]
    let flags = SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK;
    #[cfg(not(target_os = "linux"))]
    let flags = SockFlag::empty();
    let fd =
        socket(AddressFamily::Unix, SockType::Stream, flags, None).map_err(|_| socket_error())?;
    fcntl(&fd, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC)).map_err(|_| socket_error())?;
    set_nonblocking(&fd).map_err(|_| socket_error())?;
    let pending = match connect(fd.as_raw_fd(), &address) {
        Ok(()) => false,
        Err(Errno::EINPROGRESS) => true,
        Err(_) => return Err(socket_error()),
    };
    let mut stream = UnixStream::from(fd);
    if pending {
        loop {
            check_deadline(deadline, cancelled)?;
            if stream.take_error().map_err(|_| socket_error())?.is_some() {
                return Err(socket_error());
            }
            if stream.peer_addr().is_ok() {
                break;
            }
            thread::park_timeout(POLL_INTERVAL);
        }
    }
    // The explicit endpoint returns one raw PNG followed by EOF; no commands or
    // terminal identifiers cross this connection.
    stream
        .shutdown(Shutdown::Write)
        .map_err(|_| socket_error())?;
    read_bounded(&mut stream, deadline, cancelled)
}

fn validate_socket(path: &Path) -> Result<(), AppError> {
    if !path.is_absolute() {
        return Err(AppError::message(
            "YO_CLIPBOARD_IMAGE_SOCKET must be an absolute path to a private Unix socket.",
        ));
    }
    let parent = path.parent().ok_or_else(socket_error)?;
    let uid = geteuid().as_raw();
    validate_ancestors(parent, uid)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| socket_error())?;
    let parent_metadata = fs::symlink_metadata(parent).map_err(|_| socket_error())?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != uid
        || !parent_metadata.is_dir()
        || parent_metadata.uid() != uid
        || parent_metadata.mode() & 0o077 != 0
    {
        return Err(AppError::message(
            "Clipboard socket and parent directory must belong to this user, with a private parent directory (chmod 700).",
        ));
    }
    Ok(())
}

fn validate_ancestors(parent: &Path, uid: u32) -> Result<(), AppError> {
    // A mapped namespace can expose its filesystem root under a nonzero UID.
    // Only that observed root owner and this user may replace path components.
    let root_uid = fs::symlink_metadata("/").map_err(|_| socket_error())?.uid();
    let mut ancestor = PathBuf::new();
    for component in parent.components() {
        match component {
            Component::RootDir | Component::Normal(_) => ancestor.push(component),
            _ => return Err(ancestor_error()),
        }
        let metadata = fs::symlink_metadata(&ancestor).map_err(|_| socket_error())?;
        let namespace_sticky = metadata.uid() == root_uid && metadata.mode() & 0o1000 != 0;
        if !metadata.is_dir()
            || (metadata.uid() != uid && metadata.uid() != root_uid)
            || (metadata.mode() & 0o022 != 0 && !namespace_sticky)
        {
            return Err(ancestor_error());
        }
    }
    Ok(())
}

fn ancestor_error() -> AppError {
    AppError::message(
        "Clipboard socket ancestors must be directories owned by this user or the filesystem root owner, without symlinks, '..', or group/other write access (except root-owner sticky directories such as /tmp).",
    )
}

fn check_deadline(deadline: Instant, cancelled: &mut dyn FnMut() -> bool) -> Result<(), AppError> {
    check_cancelled(cancelled)?;
    if Instant::now() >= deadline {
        return Err(AppError::message(
            "Clipboard image read timed out. Check the clipboard reader or forwarded socket and try again.",
        ));
    }
    Ok(())
}

fn read_error() -> AppError {
    AppError::message("Clipboard image could not be read. Copy an image and try again.")
}

fn socket_error() -> AppError {
    AppError::message(
        "Configured clipboard socket is unavailable. Check YO_CLIPBOARD_IMAGE_SOCKET and the SSH forwarding helper.",
    )
}

#[cfg(test)]
mod tests;
