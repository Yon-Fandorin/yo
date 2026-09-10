use std::{
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

use limits::{CommandExecutionLimits, StopReason, expired_reason};
use nix::unistd::Pid;
use pipe::{PipeDrain, PipeKind, ProgressSnapshot, spawn_pipe_reader};
use process::{ChildWaiter, WaiterTestHooks, terminate_process_group_id};
use yo_core::{
    ToolExecution, ToolExecutionError, ToolExecutionOutcome, ToolExecutionPoll,
    ToolExecutionProgress, ToolExecutionResult,
};

use super::execution::{ThreadExecution, failed};

mod input;
mod limits;
mod manifest;
mod pipe;
mod process;

pub(crate) use manifest::{PreparedCommand, PreparedCommandTools};

enum CommandPlan {
    Shell(String),
    Prepared(Box<PreparedCommand>, Vec<u8>),
}

// Leave room for the status/stream framing and the common native-backend
// truncation marker so both stream-local head/tail views survive publication.
const OUTPUT_FRAMING_RESERVE: usize = 96;

pub(super) struct CommandExecution {
    inner: ThreadExecution,
    process_group: Arc<Mutex<Option<Pid>>>,
    progress: Arc<Mutex<ProgressSnapshot>>,
    last_progress: Option<Instant>,
}

impl CommandExecution {
    pub(super) fn spawn_prepared(
        workspace: PathBuf,
        command: PreparedCommand,
        arguments: Vec<u8>,
        maximum_output_bytes: usize,
        absolute_execution_timeout: Option<Duration>,
        maximum_retained_output_bytes: Option<usize>,
    ) -> Result<Self, ToolExecutionError> {
        if arguments
            .len()
            .checked_add(1)
            .is_none_or(|bytes| bytes > 4 * 1024 * 1024)
        {
            return Err(ToolExecutionError::new(
                "command tool arguments exceed their admitted bound",
            ));
        }
        Self::spawn_plan(
            workspace,
            CommandPlan::Prepared(Box::new(command), arguments),
            maximum_output_bytes,
            CommandExecutionLimits::for_agent(absolute_execution_timeout),
            WaiterTestHooks::default(),
            maximum_retained_output_bytes,
        )
    }

    pub(super) fn spawn(
        workspace: PathBuf,
        command: String,
        maximum_output_bytes: usize,
        absolute_execution_timeout: Option<Duration>,
        maximum_retained_output_bytes: Option<usize>,
    ) -> Result<Self, ToolExecutionError> {
        Self::spawn_with_limits_and_hooks(
            workspace,
            command,
            maximum_output_bytes,
            CommandExecutionLimits::for_agent(absolute_execution_timeout),
            WaiterTestHooks::default(),
            maximum_retained_output_bytes,
        )
    }

    #[cfg(test)]
    fn spawn_with_limits(
        workspace: PathBuf,
        command: String,
        maximum_output_bytes: usize,
        limits: CommandExecutionLimits,
    ) -> Result<Self, ToolExecutionError> {
        Self::spawn_with_limits_and_hooks(
            workspace,
            command,
            maximum_output_bytes,
            limits,
            WaiterTestHooks::default(),
            None,
        )
    }

    fn spawn_with_limits_and_hooks(
        workspace: PathBuf,
        command: String,
        maximum_output_bytes: usize,
        limits: CommandExecutionLimits,
        waiter_hooks: WaiterTestHooks,
        maximum_retained_output_bytes: Option<usize>,
    ) -> Result<Self, ToolExecutionError> {
        if command.is_empty() || command.chars().any(|character| character == '\0') {
            return Err(ToolExecutionError::new(
                "run_command requires a non-empty command",
            ));
        }
        Self::spawn_plan(
            workspace,
            CommandPlan::Shell(command),
            maximum_output_bytes,
            limits,
            waiter_hooks,
            maximum_retained_output_bytes,
        )
    }

    fn spawn_plan(
        workspace: PathBuf,
        command: CommandPlan,
        maximum_output_bytes: usize,
        limits: CommandExecutionLimits,
        waiter_hooks: WaiterTestHooks,
        maximum_retained_output_bytes: Option<usize>,
    ) -> Result<Self, ToolExecutionError> {
        if !limits.is_valid() {
            return Err(ToolExecutionError::new(
                "run_command deadlines must be non-zero",
            ));
        }
        let process_group = Arc::new(Mutex::new(None));
        let worker_process_group = Arc::clone(&process_group);
        let progress = Arc::new(Mutex::new(ProgressSnapshot::default()));
        let worker_progress = Arc::clone(&progress);
        let inner = ThreadExecution::spawn(move |cancelled| {
            run_command(
                &workspace,
                &command,
                CommandOutput {
                    maximum_output_bytes,
                    maximum_retained_output_bytes,
                    progress: worker_progress,
                },
                limits,
                &cancelled,
                &worker_process_group,
                waiter_hooks,
            )
        });
        Ok(Self {
            inner,
            process_group,
            progress,
            last_progress: None,
        })
    }
}

impl ToolExecution for CommandExecution {
    fn take_progress(&mut self) -> Option<ToolExecutionProgress> {
        if self
            .last_progress
            .is_some_and(|last| last.elapsed() < Duration::from_millis(100))
        {
            return None;
        }
        let progress = self.progress.lock().ok()?.take()?;
        self.last_progress = Some(Instant::now());
        Some(progress)
    }

    fn poll(&mut self) -> Result<ToolExecutionPoll, ToolExecutionError> {
        let result = self.inner.poll();
        if !matches!(result, Ok(ToolExecutionPoll::Pending))
            && let Ok(mut progress) = self.progress.lock()
        {
            progress.close();
        }
        result
    }

    fn take_result(&mut self) -> Option<ToolExecutionResult> {
        self.inner.take_result()
    }

    fn cancel(&self) {
        if let Ok(mut progress) = self.progress.lock() {
            progress.close();
        }
        self.inner.cancel();
        if let Ok(process_group) = self.process_group.lock()
            && let Some(process_group) = *process_group
        {
            terminate_process_group_id(process_group);
        }
    }

    fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
        self.cancel();
        self.inner.join()
    }
}

struct CommandOutput {
    maximum_output_bytes: usize,
    maximum_retained_output_bytes: Option<usize>,
    progress: Arc<Mutex<ProgressSnapshot>>,
}

fn run_command(
    workspace: &Path,
    command: &CommandPlan,
    output: CommandOutput,
    limits: CommandExecutionLimits,
    cancelled: &AtomicBool,
    shared_process_group: &Mutex<Option<Pid>>,
    waiter_hooks: WaiterTestHooks,
) -> ToolExecutionResult {
    let CommandOutput {
        maximum_output_bytes,
        maximum_retained_output_bytes,
        progress,
    } = output;
    let attempt_started = Instant::now();
    let (mut launch, stdin_bytes) = match command {
        CommandPlan::Shell(command) => (shell_command(workspace, command), Vec::new()),
        CommandPlan::Prepared(command, arguments) => {
            let deadline = limits
                .absolute_execution_timeout
                .and_then(|timeout| attempt_started.checked_add(timeout));
            let verified =
                command.verify_for_launch(&mut || cancelled.load(Ordering::Acquire), deadline);
            let verified = match verified {
                Ok(verified) => verified,
                Err(error) => {
                    return if cancelled.load(Ordering::Acquire)
                        || deadline.is_some_and(|deadline| Instant::now() >= deadline)
                    {
                        ToolExecutionResult::new(
                            ToolExecutionOutcome::Interrupted,
                            "command tool attempt interrupted before spawn",
                            false,
                        )
                    } else {
                        failed(&error.to_string())
                    };
                },
            };
            let mut launch = Command::new(&verified.executable);
            launch
                .args(&verified.args)
                .current_dir(workspace)
                .process_group(0)
                .env_clear()
                .env("PATH", "/usr/local/bin:/usr/bin:/bin")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let mut bytes = arguments.clone();
            bytes.push(b'\n');
            (launch, bytes)
        },
    };
    let Ok(mut waiter) = ChildWaiter::spawn(waiter_hooks) else {
        return failed("run_command waiter is unavailable");
    };
    if let Some(reason) = expired_reason(
        cancelled,
        attempt_started,
        Instant::now(),
        limits,
        Instant::now(),
    ) {
        return ToolExecutionResult::new(
            ToolExecutionOutcome::Interrupted,
            reason.description(),
            false,
        );
    }
    let spawned = launch.spawn();
    let Ok(mut child) = spawned else {
        return failed("run_command could not start");
    };
    let spawned_at = Instant::now();
    let mut stdin = match input::CommandInput::new(child.stdin.take(), stdin_bytes) {
        Ok(stdin) => stdin,
        Err(()) => return fail_without_waiter(&mut child, "command tool stdin is unavailable"),
    };
    let Some(process_group) = i32::try_from(child.id()).ok().map(Pid::from_raw) else {
        return fail_without_waiter(&mut child, "run_command process identity is unavailable");
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let Ok(mut process_group_lease) =
        ProcessGroupLease::publish(shared_process_group, process_group)
    else {
        return fail_without_waiter(&mut child, "run_command state is unavailable");
    };
    if let Err(mut child) = waiter.handoff(child) {
        process_group_lease.terminate();
        let clear_failed = process_group_lease.clear().is_err();
        let _ = child.kill();
        let wait_failed = child.wait().is_err();
        return if clear_failed || wait_failed {
            failed("run_command cleanup failed after waiter handoff failure")
        } else {
            failed("run_command waiter is unavailable")
        };
    }
    let (Some(stdout), Some(stderr)) = (stdout, stderr) else {
        return fail_after_setup_cleanup(
            waiter,
            process_group_lease,
            "run_command output pipes are unavailable",
            limits.cleanup_grace,
        );
    };

    let retained_budget =
        maximum_retained_output_bytes.map(|limit| limit.saturating_sub(OUTPUT_FRAMING_RESERVE));
    let stream_budget = maximum_output_bytes.saturating_sub(OUTPUT_FRAMING_RESERVE);
    let stdout_limit = stream_budget / 2;
    let stderr_limit = stream_budget.saturating_sub(stdout_limit);
    let (progress_sender, progress_receiver) = mpsc::sync_channel(1);
    let (drain_sender, drain_receiver) = mpsc::sync_channel(2);
    let Ok(mut stdout_reader) = spawn_pipe_reader(
        PipeKind::Stdout,
        stdout,
        stdout_limit,
        progress_sender.clone(),
        drain_sender.clone(),
        Some(Arc::clone(&progress)),
        retained_budget.map(|limit| limit / 2),
    ) else {
        return fail_after_setup_cleanup(
            waiter,
            process_group_lease,
            "run_command output reader is unavailable",
            limits.cleanup_grace,
        );
    };
    let stderr_reader = match spawn_pipe_reader(
        PipeKind::Stderr,
        stderr,
        stderr_limit,
        progress_sender,
        drain_sender.clone(),
        Some(Arc::clone(&progress)),
        retained_budget.map(|limit| limit - limit / 2),
    ) {
        Ok(reader) => reader,
        Err(()) => {
            let reader_cleanup_failed = stdout_reader.shutdown_and_join().is_err();
            let result = fail_after_setup_cleanup(
                waiter,
                process_group_lease,
                "run_command output reader is unavailable",
                limits.cleanup_grace,
            );
            return if reader_cleanup_failed {
                failed("run_command cleanup failed after setup failure")
            } else {
                result
            };
        },
    };
    let mut pipe_readers = [stdout_reader, stderr_reader];
    drop(drain_sender);

    let mut stdout = None;
    let mut stderr = None;
    let mut last_output_progress = spawned_at;
    let mut stdin_failed = false;
    let mut stop_reason = None;
    let mut leader_observed = false;
    let mut leader_observation_finished = false;
    let mut status = None;
    let mut cleanup_started = None;
    let mut termination_requested = false;
    let mut reap_requested = false;
    let mut cleanup_failed = false;
    let mut forced_pipe_shutdown = false;
    loop {
        if cleanup_started.is_none() && stdin.poll().is_err() {
            stdin_failed = true;
            cleanup_started.get_or_insert_with(Instant::now);
        }
        while progress_receiver.try_recv().is_ok() {
            last_output_progress = Instant::now();
        }
        loop {
            match drain_receiver.try_recv() {
                Ok(drain) => {
                    if drain.failed {
                        cleanup_failed = true;
                        cleanup_started.get_or_insert_with(Instant::now);
                    }
                    match drain.kind {
                        PipeKind::Stdout => stdout = Some(drain),
                        PipeKind::Stderr => stderr = Some(drain),
                    }
                },
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if stdout.is_none() || stderr.is_none() {
                        cleanup_failed = true;
                        cleanup_started.get_or_insert_with(Instant::now);
                    }
                    break;
                },
            }
        }
        if stop_reason.is_none() {
            stop_reason = expired_reason(
                cancelled,
                attempt_started,
                last_output_progress,
                limits,
                Instant::now(),
            );
            if stop_reason.is_some() {
                cleanup_started.get_or_insert_with(Instant::now);
            }
        }
        if !leader_observation_finished {
            match waiter.try_leader_exit() {
                Ok(true) => {
                    stdin_failed |= !stdin.complete();
                    leader_observed = true;
                    leader_observation_finished = true;
                    cleanup_started.get_or_insert_with(Instant::now);
                },
                Ok(false) => {},
                Err(()) => {
                    leader_observation_finished = true;
                    cleanup_failed = true;
                    cleanup_started.get_or_insert_with(Instant::now);
                },
            }
        }
        if cleanup_started.is_some() && !termination_requested {
            stdin.close();
            process_group_lease.terminate();
            termination_requested = true;
        }
        if !reap_requested
            && (leader_observed || (leader_observation_finished && cleanup_failed))
            && stdout.is_some()
            && stderr.is_some()
        {
            cleanup_failed |= process_group_lease.clear().is_err();
            cleanup_failed |= waiter.request_reap().is_err();
            reap_requested = true;
        }
        if reap_requested && status.is_none() {
            match waiter.try_status() {
                Ok(waiter_status) => status = waiter_status,
                Err(()) => cleanup_failed = true,
            }
        }
        if status.is_some() && stdout.is_some() && stderr.is_some() {
            break;
        }
        if cleanup_started.is_some_and(|started| started.elapsed() >= limits.cleanup_grace) {
            process_group_lease.terminate();
            forced_pipe_shutdown = stdout.is_none() || stderr.is_none();
            if forced_pipe_shutdown {
                for reader in &mut pipe_readers {
                    reader.request_shutdown();
                }
            }
            if !reap_requested {
                cleanup_failed |= process_group_lease.clear().is_err();
                cleanup_failed |= waiter.request_reap().is_err();
            }
            if status.is_none()
                && let Ok(waiter_status) = waiter.try_status()
            {
                status = waiter_status;
            }
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    for reader in &mut pipe_readers {
        cleanup_failed |= reader.join().is_err();
    }
    for drain in drain_receiver.try_iter() {
        cleanup_failed |= drain.failed;
        match drain.kind {
            PipeKind::Stdout => stdout = Some(drain),
            PipeKind::Stderr => stderr = Some(drain),
        }
    }
    cleanup_failed |=
        forced_pipe_shutdown || status.is_none() || stdout.is_none() || stderr.is_none();
    cleanup_failed |= process_group_lease.clear().is_err();

    let truncated = forced_pipe_shutdown
        || stdout.as_ref().is_some_and(PipeDrain::truncated)
        || stderr.as_ref().is_some_and(PipeDrain::truncated);
    let retained = match (&stdout, &stderr) {
        (Some(stdout), Some(stderr)) => stdout
            .retained_output
            .as_ref()
            .zip(stderr.retained_output.as_ref())
            .map(|(out, err)| {
                let reported_status = if cleanup_failed {
                    format!(
                        "cleanup failed after {}",
                        stop_reason.map_or("normal process exit", StopReason::description)
                    )
                } else if let Some(reason) = stop_reason {
                    reason.description().to_owned()
                } else if stdin_failed {
                    "command tool stdin delivery failed".to_owned()
                } else {
                    status.as_ref().map_or_else(
                        || "not reported".to_owned(),
                        |status| {
                            status
                                .code()
                                .map_or_else(|| "signal".to_owned(), |code| code.to_string())
                        },
                    )
                };
                let output = format!(
                    "status: {}\nstdout:\n{}\nstderr:\n{}",
                    reported_status,
                    String::from_utf8_lossy(&out.clone().render()),
                    String::from_utf8_lossy(&err.clone().render())
                );
                (
                    output,
                    forced_pipe_shutdown
                        || stdout.failed
                        || stderr.failed
                        || out.truncated()
                        || err.truncated(),
                )
            }),
        _ => None,
    };
    let retain = |result: ToolExecutionResult| match retained {
        Some((text, truncated)) => result.with_retained_output(text, truncated),
        None => result,
    };
    if cleanup_failed {
        let cause = stop_reason.map_or("normal process exit", StopReason::description);
        return retain(ToolExecutionResult::new(
            ToolExecutionOutcome::Failed,
            format!("run_command cleanup failed after {cause}"),
            truncated,
        ));
    }
    if let Some(reason) = stop_reason {
        return retain(ToolExecutionResult::new(
            ToolExecutionOutcome::Interrupted,
            reason.description(),
            truncated,
        ));
    }
    let (Some(status), Some(stdout), Some(stderr)) = (status, stdout, stderr) else {
        return failed("run_command cleanup failed after normal process exit");
    };
    if stdin_failed {
        return retain(ToolExecutionResult::new(
            ToolExecutionOutcome::Failed,
            format!(
                "status: command tool stdin delivery failed\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&stdout.output.render()),
                String::from_utf8_lossy(&stderr.output.render())
            ),
            truncated,
        ));
    }
    retain(render_command_result(status, stdout, stderr, truncated))
}

fn shell_command(workspace: &Path, command: &str) -> Command {
    let mut shell = Command::new("/bin/sh");
    shell
        .arg("-c")
        .arg(command)
        .current_dir(workspace)
        .process_group(0)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    shell
}

fn render_command_result(
    status: ExitStatus,
    stdout: PipeDrain,
    stderr: PipeDrain,
    truncated: bool,
) -> ToolExecutionResult {
    let stdout = stdout.output.render();
    let stderr = stderr.output.render();
    let stdout = String::from_utf8_lossy(&stdout);
    let stderr = String::from_utf8_lossy(&stderr);
    let output = format!(
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        status
            .code()
            .map_or_else(|| "signal".to_owned(), |code| code.to_string()),
        stdout,
        stderr
    );
    ToolExecutionResult::new(
        if status.success() {
            ToolExecutionOutcome::Completed
        } else {
            ToolExecutionOutcome::Failed
        },
        output,
        truncated,
    )
}

fn fail_after_setup_cleanup(
    mut waiter: ChildWaiter,
    mut process_group: ProcessGroupLease<'_>,
    cause: &'static str,
    cleanup_grace: Duration,
) -> ToolExecutionResult {
    process_group.terminate();
    let mut cleanup_failed = process_group.clear().is_err() || waiter.request_reap().is_err();
    let started = Instant::now();
    loop {
        match waiter.try_status() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < cleanup_grace => {
                thread::sleep(Duration::from_millis(10));
            },
            Ok(None) | Err(()) => {
                cleanup_failed = true;
                break;
            },
        }
    }
    if cleanup_failed {
        failed("run_command cleanup failed after setup failure")
    } else {
        failed(cause)
    }
}

fn fail_without_waiter(child: &mut Child, cause: &'static str) -> ToolExecutionResult {
    if let Ok(pid) = i32::try_from(child.id()) {
        terminate_process_group_id(Pid::from_raw(pid));
    }
    let _ = child.kill();
    if child.wait().is_err() {
        failed("run_command cleanup failed after setup failure")
    } else {
        failed(cause)
    }
}

struct ProcessGroupLease<'a> {
    shared: &'a Mutex<Option<Pid>>,
    process_group: Pid,
    active: bool,
}

impl<'a> ProcessGroupLease<'a> {
    fn publish(shared: &'a Mutex<Option<Pid>>, process_group: Pid) -> Result<Self, ()> {
        let mut slot = shared.lock().map_err(|_| ())?;
        if slot.is_some() {
            return Err(());
        }
        *slot = Some(process_group);
        Ok(Self {
            shared,
            process_group,
            active: true,
        })
    }

    fn terminate(&self) {
        self.terminate_with(terminate_process_group_id);
    }

    fn terminate_with(&self, terminate: impl FnOnce(Pid)) {
        if self.active {
            terminate(self.process_group);
        }
    }

    fn clear(&mut self) -> Result<(), ()> {
        if !self.active {
            return Ok(());
        }
        let mut slot = self.shared.lock().map_err(|_| ())?;
        if *slot != Some(self.process_group) {
            return Err(());
        }
        *slot = None;
        self.active = false;
        Ok(())
    }
}

impl Drop for ProcessGroupLease<'_> {
    fn drop(&mut self) {
        let _ = self.clear();
    }
}

#[cfg(test)]
mod tests;
