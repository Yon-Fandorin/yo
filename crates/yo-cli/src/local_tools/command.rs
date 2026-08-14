use std::{
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, atomic::AtomicBool},
    thread,
    time::{Duration, Instant},
};

use limits::{CommandExecutionLimits, StopReason, expired_reason};
use nix::unistd::Pid;
use pipe::{PipeState, read_nonblocking_bounded, set_nonblocking};
use process::{
    child_exited_without_reaping, reap_child, terminate_process_group, terminate_process_group_id,
};
use yo_core::{
    ToolExecution, ToolExecutionError, ToolExecutionOutcome, ToolExecutionPoll, ToolExecutionResult,
};

use super::execution::{ThreadExecution, failed};

mod limits;
mod pipe;
mod process;

pub(super) struct CommandExecution {
    inner: ThreadExecution,
    child: Arc<Mutex<Option<Child>>>,
}

impl CommandExecution {
    pub(super) fn spawn(
        workspace: PathBuf,
        command: String,
        maximum_output_bytes: usize,
        absolute_execution_timeout: Option<Duration>,
    ) -> Result<Self, ToolExecutionError> {
        Self::spawn_with_limits(
            workspace,
            command,
            maximum_output_bytes,
            CommandExecutionLimits::for_agent(absolute_execution_timeout),
        )
    }

    fn spawn_with_limits(
        workspace: PathBuf,
        command: String,
        maximum_output_bytes: usize,
        limits: CommandExecutionLimits,
    ) -> Result<Self, ToolExecutionError> {
        if command.is_empty() || command.chars().any(|character| character == '\0') {
            return Err(ToolExecutionError::new(
                "run_command requires a non-empty command",
            ));
        }
        if !limits.is_valid() {
            return Err(ToolExecutionError::new(
                "run_command deadlines must be non-zero",
            ));
        }
        let child = Arc::new(Mutex::new(None));
        let worker_child = Arc::clone(&child);
        let inner = ThreadExecution::spawn(move |cancelled| {
            run_command(
                &workspace,
                &command,
                maximum_output_bytes,
                limits,
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
    limits: CommandExecutionLimits,
    cancelled: &AtomicBool,
    shared_child: &Mutex<Option<Child>>,
) -> ToolExecutionResult {
    let attempt_started = Instant::now();
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
        return fail_after_early_cleanup(
            &mut child,
            "run_command output pipes are unavailable",
            limits.cleanup_grace,
        );
    };
    if set_nonblocking(&stdout).is_err() || set_nonblocking(&stderr).is_err() {
        return fail_after_early_cleanup(
            &mut child,
            "run_command output pipes are unavailable",
            limits.cleanup_grace,
        );
    }
    let mut stdout = Some(stdout);
    let mut stderr = Some(stderr);
    {
        let Ok(mut slot) = shared_child.lock() else {
            return fail_after_early_cleanup(
                &mut child,
                "run_command state is unavailable",
                limits.cleanup_grace,
            );
        };
        *slot = Some(child);
    }

    let stdout_limit = maximum_output_bytes / 2;
    let stderr_limit = maximum_output_bytes.saturating_sub(stdout_limit);
    let mut stdout_bytes = Vec::with_capacity(stdout_limit.min(64 * 1024));
    let mut stderr_bytes = Vec::with_capacity(stderr_limit.min(64 * 1024));
    let mut stdout_truncated = false;
    let mut stderr_truncated = false;
    let mut last_output_progress = attempt_started;
    let mut stop_reason = None;
    let mut leader_exited = false;
    let mut status = None;
    let mut cleanup_started = None;
    let mut cleanup_failed = false;
    loop {
        let stdout_read = stdout
            .as_mut()
            .map(|pipe| read_nonblocking_bounded(pipe, &mut stdout_bytes, stdout_limit));
        match stdout_read {
            Some(Ok(read)) => {
                if read.progressed {
                    last_output_progress = Instant::now();
                }
                match read.state {
                    PipeState::Open => {},
                    PipeState::Closed => stdout = None,
                    PipeState::Truncated => {
                        stdout = None;
                        stdout_truncated = true;
                    },
                }
            },
            Some(Err(())) => {
                stdout = None;
                stdout_truncated = true;
            },
            None => {},
        }
        let stderr_read = stderr
            .as_mut()
            .map(|pipe| read_nonblocking_bounded(pipe, &mut stderr_bytes, stderr_limit));
        match stderr_read {
            Some(Ok(read)) => {
                if read.progressed {
                    last_output_progress = Instant::now();
                }
                match read.state {
                    PipeState::Open => {},
                    PipeState::Closed => stderr = None,
                    PipeState::Truncated => {
                        stderr = None;
                        stderr_truncated = true;
                    },
                }
            },
            Some(Err(())) => {
                stderr = None;
                stderr_truncated = true;
            },
            None => {},
        }
        if stop_reason.is_none() {
            stop_reason = expired_reason(
                cancelled,
                attempt_started,
                last_output_progress,
                limits,
                Instant::now(),
            );
        }
        if stop_reason.is_some() {
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
        if cleanup_started.is_some_and(|cleanup| cleanup.elapsed() >= limits.cleanup_grace) {
            cleanup_failed = !leader_exited || stdout.is_some() || stderr.is_some();
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    if let Some(process_group) = process_group {
        terminate_process_group_id(process_group);
    }
    if let Ok(mut slot) = shared_child.lock() {
        if let Some(mut child) = slot.take() {
            let (reaped_status, reap_failed) =
                reap_child(&mut child, leader_exited, limits.cleanup_grace);
            status = reaped_status;
            cleanup_failed |= reap_failed;
        }
    } else {
        cleanup_failed = true;
    }
    drop(stdout);
    drop(stderr);
    if cleanup_failed {
        let cause = stop_reason.map_or("normal process exit", StopReason::description);
        return ToolExecutionResult::new(
            ToolExecutionOutcome::Failed,
            format!("run_command cleanup failed after {cause}"),
            stdout_truncated || stderr_truncated,
        );
    }
    if let Some(reason) = stop_reason {
        return ToolExecutionResult::new(
            ToolExecutionOutcome::Interrupted,
            reason.description(),
            stdout_truncated || stderr_truncated,
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

fn fail_after_early_cleanup(
    child: &mut Child,
    cause: &'static str,
    cleanup_grace: Duration,
) -> ToolExecutionResult {
    terminate_process_group(child);
    let (_, cleanup_failed) = reap_child(child, false, cleanup_grace);
    if cleanup_failed {
        failed("run_command cleanup failed after setup failure")
    } else {
        failed(cause)
    }
}

#[cfg(test)]
mod tests;
