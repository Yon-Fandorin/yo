//! Linux namespace 설정과 고정한 descriptor 전달만 소유한다.

use std::{
    env,
    fs::{self, File},
    io,
    os::{
        fd::AsRawFd,
        unix::{fs::DirBuilderExt, process::CommandExt},
    },
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use yo_core::ToolExecutionError;

use super::{ConfinedPlan, PRIVATE_HOME, PRIVATE_TMP, Payload, error};

const BWRAP: &str = "/usr/bin/bwrap";

fn base_command() -> Command {
    let mut command = Command::new(BWRAP);
    command
        .args([
            "--unshare-user",
            "--unshare-pid",
            "--unshare-ipc",
            "--unshare-net",
            "--unshare-uts",
            "--die-with-parent",
            "--new-session",
            "--cap-drop",
            "ALL",
            "--clearenv",
        ])
        .env_clear()
        .process_group(0);
    command
}

pub(super) fn available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let Ok(id) = yo_core::SessionId::new() else {
            return false;
        };
        let path = env::temp_dir().join(format!("yo-command-profile-{id}"));
        if fs::DirBuilder::new().mode(0o700).create(&path).is_err() {
            return false;
        }
        struct ProbeDirectory(PathBuf);
        impl Drop for ProbeDirectory {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _directory = ProbeDirectory(path.clone());
        let (Ok(support), Ok(writable)) = (File::open("/usr"), File::open(&path)) else {
            return false;
        };
        let mut probe = base_command();
        probe.args([
            "--ro-bind-fd",
            &support.as_raw_fd().to_string(),
            "/usr",
            "--bind-fd",
            &writable.as_raw_fd().to_string(),
            "/probe",
        ]);
        configure_descriptors(&mut probe, vec![support.as_raw_fd(), writable.as_raw_fd()]);
        probe
            .args([
                "--symlink",
                "usr/bin",
                "/bin",
                "--symlink",
                "usr/lib",
                "/lib",
                "--symlink",
                "usr/lib64",
                "/lib64",
                "--proc",
                "/proc",
                "--dev",
                "/dev",
                "--tmpfs",
                "/tmp",
                "--",
                "/bin/sh",
                "-c",
                "printf qualified > /probe/ok && test ! -w /usr/bin && test -d /proc/self/fd",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let Ok(mut child) = probe.spawn() else {
            return false;
        };
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return status.success(),
                Ok(None) if started.elapsed() < Duration::from_secs(2) => {
                    thread::sleep(Duration::from_millis(10))
                },
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                },
            }
        }
    })
}

pub(super) fn launch(
    plan: &ConfinedPlan,
    cancelled: &AtomicBool,
    deadline: Option<Instant>,
) -> Result<(Command, Vec<u8>), ToolExecutionError> {
    for hidden in &plan.hidden {
        hidden.verify()?;
    }
    let mut command = base_command();
    command.args([
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
        "--tmpfs",
        "/yo-private",
        "--dir",
        PRIVATE_HOME,
        "--dir",
        PRIVATE_TMP,
        "--dir",
        "/yo-private/home/.cargo",
    ]);
    let mut passed = Vec::new();
    for root in &plan.roots {
        let fd = root.file.as_raw_fd();
        passed.push(fd);
        command
            .arg(if root.writable {
                "--bind-fd"
            } else {
                "--ro-bind-fd"
            })
            .arg(fd.to_string())
            .arg(&root.destination);
    }
    for hidden in &plan.hidden {
        if hidden.directory {
            command
                .arg("--tmpfs")
                .arg(&hidden.path)
                .arg("--remount-ro")
                .arg(&hidden.path);
        } else {
            command.args(["--ro-bind", "/dev/null"]).arg(&hidden.path);
        }
    }
    for (name, value) in &plan.environment {
        command.args(["--setenv", name, value]);
    }
    command.arg("--chdir").arg(&plan.workspace).arg("--");
    let stdin = match &plan.payload {
        Payload::Shell(text) => {
            command.args(["/bin/sh", "-c", text]);
            Vec::new()
        },
        Payload::Configured(prepared, arguments) => {
            let verified =
                prepared.verify_for_launch(&mut || cancelled.load(Ordering::Acquire), deadline)?;
            command.arg(verified.executable).args(verified.args);
            let mut bytes = arguments.clone();
            bytes.push(b'\n');
            bytes
        },
    };
    command
        .stdin(if stdin.is_empty() {
            Stdio::null()
        } else {
            Stdio::piped()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_descriptors(&mut command, passed);
    if cancelled.load(Ordering::Acquire)
        || deadline.is_some_and(|deadline| Instant::now() >= deadline)
    {
        return Err(error());
    }
    Ok((command, stdin))
}

#[allow(unsafe_code)]
fn configure_descriptors(command: &mut Command, passed: Vec<i32>) {
    // SAFETY: fork 뒤에는 할당/잠금 없이 syscall만 호출한다. close_range는 닫지 않고
    // CLOEXEC만 설정하므로 Rust의 exec 오류 pipe를 보존한다. bwrap는 전달한 mount FD를
    // setup에서 소비하며 모델 명령은 stdio만 상속한다.
    unsafe {
        command.pre_exec(move || {
            if libc::syscall(libc::SYS_close_range, 3_u32, u32::MAX, 4_u32) != 0 {
                return Err(io::Error::last_os_error());
            }
            for fd in &passed {
                if libc::fcntl(*fd, libc::F_SETFD, 0) == -1 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
}
