use std::{
    io::Read,
    os::{fd::AsFd, unix::process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use nix::{
    fcntl::{FcntlArg, OFlag, fcntl},
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use yo_core::{
    ToolExecution, ToolExecutionError, ToolExecutionOutcome, ToolExecutionPoll, ToolExecutionResult,
};

use super::execution::{ThreadExecution, failed, interrupted};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const PROCESS_CLEANUP_GRACE: Duration = Duration::from_secs(1);

pub(super) struct CommandExecution {
    inner: ThreadExecution,
    child: Arc<Mutex<Option<Child>>>,
}

impl CommandExecution {
    pub(super) fn spawn(
        workspace: PathBuf,
        command: String,
        maximum_output_bytes: usize,
    ) -> Result<Self, ToolExecutionError> {
        if command.is_empty() || command.chars().any(|character| character == '\0') {
            return Err(ToolExecutionError::new(
                "run_command requires a non-empty command",
            ));
        }
        let child = Arc::new(Mutex::new(None));
        let worker_child = Arc::clone(&child);
        let inner = ThreadExecution::spawn(move |cancelled| {
            run_command(
                &workspace,
                &command,
                maximum_output_bytes,
                &cancelled,
                &worker_child,
            )
        });
        Ok(Self { inner, child })
    }
}

impl ToolExecution for CommandExecution {
    fn poll(&mut self) -> Result<ToolExecutionPoll, ToolExecutionError> {
        self.inner.poll()
    }

    fn take_result(&mut self) -> Option<ToolExecutionResult> {
        self.inner.take_result()
    }

    fn cancel(&self) {
        self.inner.cancel();
        if let Ok(mut child) = self.child.lock()
            && let Some(child) = child.as_mut()
        {
            terminate_process_group(child);
        }
    }

    fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
        self.cancel();
        self.inner.join()
    }
}

fn run_command(
    workspace: &Path,
    command: &str,
    maximum_output_bytes: usize,
    cancelled: &AtomicBool,
    shared_child: &Mutex<Option<Child>>,
) -> ToolExecutionResult {
    let spawned = Command::new("/bin/sh")
        .arg("-lc")
        .arg(command)
        .current_dir(workspace)
        .process_group(0)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut child) = spawned else {
        return failed("run_command could not start");
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let process_group = i32::try_from(child.id()).ok().map(Pid::from_raw);
    let (Some(stdout), Some(stderr)) = (stdout, stderr) else {
        terminate_process_group(&mut child);
        return failed("run_command output pipes are unavailable");
    };
    if set_nonblocking(&stdout).is_err() || set_nonblocking(&stderr).is_err() {
        terminate_process_group(&mut child);
        return failed("run_command output pipes are unavailable");
    }
    let mut stdout = Some(stdout);
    let mut stderr = Some(stderr);
    {
        let Ok(mut slot) = shared_child.lock() else {
            terminate_process_group(&mut child);
            return failed("run_command state is unavailable");
        };
        *slot = Some(child);
    }

    let stdout_limit = maximum_output_bytes / 2;
    let stderr_limit = maximum_output_bytes.saturating_sub(stdout_limit);
    let mut stdout_bytes = Vec::with_capacity(stdout_limit.min(64 * 1024));
    let mut stderr_bytes = Vec::with_capacity(stderr_limit.min(64 * 1024));
    let mut stdout_truncated = false;
    let mut stderr_truncated = false;
    let started = Instant::now();
    let mut timed_out = false;
    let mut leader_exited = false;
    let mut status = None;
    let mut cleanup_started = None;
    loop {
        let stdout_read = stdout
            .as_mut()
            .map(|pipe| read_nonblocking_bounded(pipe, &mut stdout_bytes, stdout_limit));
        match stdout_read {
            Some(Ok(PipeRead::Open)) => {},
            Some(Ok(PipeRead::Closed)) => stdout = None,
            Some(Ok(PipeRead::Truncated) | Err(())) => {
                stdout = None;
                stdout_truncated = true;
            },
            None => {},
        }
        let stderr_read = stderr
            .as_mut()
            .map(|pipe| read_nonblocking_bounded(pipe, &mut stderr_bytes, stderr_limit));
        match stderr_read {
            Some(Ok(PipeRead::Open)) => {},
            Some(Ok(PipeRead::Closed)) => stderr = None,
            Some(Ok(PipeRead::Truncated) | Err(())) => {
                stderr = None;
                stderr_truncated = true;
            },
            None => {},
        }
        if cancelled.load(Ordering::Acquire) || started.elapsed() >= COMMAND_TIMEOUT {
            timed_out = started.elapsed() >= COMMAND_TIMEOUT;
            cleanup_started.get_or_insert_with(Instant::now);
            if let Some(process_group) = process_group {
                terminate_process_group_id(process_group);
            }
            if let Ok(mut slot) = shared_child.lock()
                && let Some(child) = slot.as_mut()
            {
                terminate_process_group(child);
            }
        }
        if !leader_exited {
            leader_exited = process_group.is_some_and(child_exited_without_reaping);
            if leader_exited {
                cleanup_started.get_or_insert_with(Instant::now);
                if let Some(process_group) = process_group {
                    terminate_process_group_id(process_group);
                }
            }
        }
        if leader_exited && stdout.is_none() && stderr.is_none() {
            break;
        }
        if cleanup_started.is_some_and(|cleanup| cleanup.elapsed() >= PROCESS_CLEANUP_GRACE) {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    if let Some(process_group) = process_group {
        terminate_process_group_id(process_group);
    }
    if let Ok(mut slot) = shared_child.lock()
        && let Some(mut child) = slot.take()
    {
        if leader_exited {
            status = child.wait().ok();
        } else {
            let _ = child.kill();
            status = child.try_wait().ok().flatten();
        }
    }
    drop(stdout);
    drop(stderr);
    if cancelled.load(Ordering::Acquire) {
        return interrupted();
    }
    if timed_out {
        return ToolExecutionResult::new(
            ToolExecutionOutcome::Interrupted,
            "run_command timed out",
            false,
        );
    }
    let stdout = String::from_utf8_lossy(&stdout_bytes);
    let stderr = String::from_utf8_lossy(&stderr_bytes);
    let output = format!(
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        status
            .and_then(|status| status.code())
            .map_or_else(|| "signal".to_owned(), |code| code.to_string()),
        stdout,
        stderr
    );
    ToolExecutionResult::new(
        if status.is_some_and(|status| status.success()) {
            ToolExecutionOutcome::Completed
        } else {
            ToolExecutionOutcome::Failed
        },
        output,
        stdout_truncated || stderr_truncated,
    )
}

fn set_nonblocking(descriptor: &impl AsFd) -> Result<(), ()> {
    let flags = fcntl(descriptor, FcntlArg::F_GETFL).map_err(|_| ())?;
    fcntl(
        descriptor,
        FcntlArg::F_SETFL(OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK),
    )
    .map(|_| ())
    .map_err(|_| ())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PipeRead {
    Open,
    Closed,
    Truncated,
}

fn read_nonblocking_bounded(
    reader: &mut impl Read,
    output: &mut Vec<u8>,
    limit: usize,
) -> Result<PipeRead, ()> {
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let target = limit.saturating_add(1);
        if output.len() >= target {
            output.truncate(limit);
            return Ok(PipeRead::Truncated);
        }
        let read_len = chunk.len().min(target - output.len());
        match reader.read(&mut chunk[..read_len]) {
            Ok(0) => return Ok(PipeRead::Closed),
            Ok(count) => output.extend_from_slice(&chunk[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                return Ok(PipeRead::Open);
            },
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {},
            Err(_) => return Err(()),
        }
    }
}

fn terminate_process_group(child: &mut Child) {
    if let Ok(pid) = i32::try_from(child.id()) {
        terminate_process_group_id(Pid::from_raw(pid));
    }
    let _ = child.kill();
}

fn terminate_process_group_id(process_group: Pid) {
    let _ = kill(Pid::from_raw(-process_group.as_raw()), Signal::SIGKILL);
}

// WNOWAIT leaves the exited group leader as a zombie, so its numeric process-group ID
// remains pinned until the caller has killed any descendants and explicitly reaped it.
#[allow(unsafe_code)]
fn child_exited_without_reaping(child: Pid) -> bool {
    // SAFETY: waitid initializes siginfo_t on success. The zeroed value also makes the
    // WNOHANG/no-state-change result distinguishable by its zero si_pid on Linux and macOS.
    let mut information = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            child.as_raw() as libc::id_t,
            &mut information,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    result == 0 && unsafe { information.si_pid() } != 0
}
