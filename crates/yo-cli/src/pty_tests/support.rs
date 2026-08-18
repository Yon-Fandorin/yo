use std::{
    collections::VecDeque,
    error::Error,
    fs::File,
    io::{self, Read},
    num::NonZeroU64,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    task::{Context, Poll},
    thread,
    time::Duration,
};

use nix::{
    pty::{Winsize, openpty},
    sys::{
        signal::{Signal, kill},
        termios::tcgetattr,
        wait::{WaitPidFlag, WaitStatus, waitpid},
    },
    unistd::Pid,
};
use yo_core::{AgentCommand, TranscriptRecord, TurnId, TurnRef, UserInput};
use yo_tui::{
    AgentAction, AgentConnection, AgentPoll, DispatchOutcome, PendingDispatch, PresentationMode,
    TerminationSource,
};

pub(super) const CHILD_MARKER: &str = "YO_PTY_CHILD";
pub(super) const ENTER_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049h";
pub(super) const LEAVE_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049l";

pub(super) struct PendingAgent;

impl AgentConnection for PendingAgent {
    type Error = io::Error;

    fn dispatch(&mut self, _action: AgentAction) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Queued)
    }

    fn retry(&mut self, _pending: PendingDispatch) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Queued)
    }

    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        Ok(AgentPoll::Pending)
    }

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<()> {
        Poll::Pending
    }
}

pub(super) struct RetainedChatAgent {
    records: VecDeque<TranscriptRecord>,
}

impl RetainedChatAgent {
    pub(super) fn new() -> Self {
        Self::with_input("YO_INLINE_RETAINED")
    }

    pub(super) fn new_with_large_publication() -> Self {
        Self::with_input(format!("YO_INLINE_RETAINED{}", " x".repeat(48 * 1024)))
    }

    fn with_input(input: impl Into<UserInput>) -> Self {
        let session_id = "01890f00-0000-7000-8000-000000000001"
            .parse()
            .expect("the fixture is a UUIDv7");
        let turn = TurnRef::new(session_id, id(TurnId::new));
        Self {
            records: [TranscriptRecord::CommandCommitted(
                AgentCommand::StartTurn {
                    turn,
                    input: input.into(),
                },
            )]
            .into(),
        }
    }
}

impl AgentConnection for RetainedChatAgent {
    type Error = io::Error;

    fn dispatch(&mut self, _action: AgentAction) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Queued)
    }

    fn retry(&mut self, _pending: PendingDispatch) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Queued)
    }

    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        Ok(self
            .records
            .pop_front()
            .map_or(AgentPoll::Pending, AgentPoll::Record))
    }

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<()> {
        if self.records.is_empty() {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}

pub(super) struct PtyChild {
    child: Option<std::process::Child>,
    input: Option<File>,
    output: Option<thread::JoinHandle<Vec<u8>>>,
    captured_output: Arc<Mutex<Vec<u8>>>,
    capture_release: Option<mpsc::Sender<()>>,
    ready_events: Vec<mpsc::Receiver<usize>>,
    output_events: mpsc::Receiver<usize>,
    #[cfg(target_os = "linux")]
    screen_events: mpsc::Receiver<ScreenEvent>,
    slave: Option<std::os::fd::OwnedFd>,
    pub(super) original_termios: nix::sys::termios::Termios,
}

struct SpawnedChildGuard {
    child: Option<std::process::Child>,
}

impl SpawnedChildGuard {
    fn new(child: std::process::Child) -> Self {
        Self { child: Some(child) }
    }

    fn pid(&self) -> Pid {
        child_pid(
            self.child
                .as_ref()
                .expect("the setup guard must still own the PTY child"),
        )
        .expect("the PTY child process ID must fit pid_t")
    }

    fn transfer(mut self) -> std::process::Child {
        self.child
            .take()
            .expect("the setup guard must transfer its exact PTY child")
    }
}

impl Drop for SpawnedChildGuard {
    fn drop(&mut self) {
        let _ = terminate_owned_child(&mut self.child);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ScreenEvent {
    Entered,
    Left,
}

#[derive(Debug)]
pub(super) enum ChildReapReceipt {
    Waitpid(WaitStatus),
    AlreadyReaped,
}

#[derive(Debug)]
pub(super) struct ReadinessWaitFailure {
    cause: mpsc::RecvTimeoutError,
    cleanup: Result<ChildReapReceipt, String>,
    output: Vec<u8>,
}

impl ReadinessWaitFailure {
    pub(super) fn cleanup(&self) -> Result<&ChildReapReceipt, &str> {
        self.cleanup.as_ref().map_err(String::as_str)
    }

    pub(super) fn output(&self) -> &[u8] {
        &self.output
    }
}

impl std::fmt::Display for ReadinessWaitFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "the readiness marker wait failed: {}; child cleanup: {}; PTY output:\n{}",
            self.cause,
            cleanup_diagnostic(&self.cleanup),
            String::from_utf8_lossy(&self.output)
        )
    }
}

impl PtyChild {
    pub(super) fn spawn(test_name: &str, ready_marker: &'static [u8]) -> Self {
        Self::spawn_with_ready_markers(test_name, &[ready_marker])
    }

    pub(super) fn spawn_with_ready_markers(
        test_name: &str,
        ready_markers: &[&'static [u8]],
    ) -> Self {
        Self::spawn_configured(test_name, ready_markers, None)
    }

    pub(super) fn spawn_with_capture_pause(
        test_name: &str,
        ready_markers: &[&'static [u8]],
        pause_after_marker: usize,
    ) -> Self {
        Self::spawn_configured(test_name, ready_markers, Some(pause_after_marker))
    }

    pub(super) fn spawn_with_injected_post_spawn_failure(
        test_name: &str,
        ready_marker: &'static [u8],
        child_spawned: mpsc::Sender<Pid>,
        capture_started: Arc<AtomicBool>,
    ) -> Self {
        Self::spawn_configured_inner(
            test_name,
            &[ready_marker],
            None,
            move |pid| {
                child_spawned
                    .send(pid)
                    .expect("the setup-failure test must retain its PID receiver");
                panic!("injected PTY setup failure after child spawn");
            },
            Some(capture_started),
        )
    }

    fn spawn_configured(
        test_name: &str,
        ready_markers: &[&'static [u8]],
        pause_after_marker: Option<usize>,
    ) -> Self {
        Self::spawn_configured_inner(test_name, ready_markers, pause_after_marker, |_| {}, None)
    }

    fn spawn_configured_inner(
        test_name: &str,
        ready_markers: &[&'static [u8]],
        pause_after_marker: Option<usize>,
        after_spawn: impl FnOnce(Pid),
        capture_started: Option<Arc<AtomicBool>>,
    ) -> Self {
        if let Some(marker) = pause_after_marker {
            assert!(
                marker < ready_markers.len(),
                "capture pause marker must exist"
            );
        }
        let pty = openpty(
            Some(&Winsize {
                ws_row: 24,
                ws_col: 80,
                ws_xpixel: 0,
                ws_ypixel: 0,
            }),
            None,
        )
        .unwrap();
        let original_termios = tcgetattr(&pty.slave).unwrap();
        let stdin = File::from(pty.slave.try_clone().unwrap());
        let stdout = File::from(pty.slave.try_clone().unwrap());
        let stderr = File::from(pty.slave.try_clone().unwrap());
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", test_name, "--nocapture"])
            .env(CHILD_MARKER, "1")
            .stdin(Stdio::from(stdin))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .unwrap();
        let child = SpawnedChildGuard::new(child);
        after_spawn(child.pid());

        let master = File::from(pty.master);
        let input = master.try_clone().unwrap();
        let (ready_senders, ready_events): (Vec<_>, Vec<_>) = ready_markers
            .iter()
            .map(|marker| {
                let (sender, receiver) = mpsc::channel();
                ((sender, *marker), receiver)
            })
            .unzip();
        let (screen_tx, screen_events) = mpsc::channel();
        #[cfg(not(target_os = "linux"))]
        drop(screen_events);
        let (output_tx, output_events) = mpsc::channel();
        let (capture_release, capture_pause) = pause_after_marker.map_or_else(
            || (None, None),
            |marker| {
                let (release, paused) = mpsc::channel();
                (Some(release), Some((marker, paused)))
            },
        );
        let captured_output = Arc::new(Mutex::new(Vec::new()));
        let capture_snapshot = Arc::clone(&captured_output);
        let output = thread::Builder::new()
            .name("yo-pty-test-capture".to_owned())
            .spawn(move || {
                if let Some(capture_started) = capture_started {
                    capture_started.store(true, Ordering::Release);
                }
                capture_pty(
                    master,
                    ready_senders,
                    output_tx,
                    screen_tx,
                    capture_snapshot,
                    capture_pause,
                )
            })
            .unwrap();

        Self {
            child: Some(child.transfer()),
            input: Some(input),
            output: Some(output),
            captured_output,
            capture_release,
            ready_events,
            output_events,
            #[cfg(target_os = "linux")]
            screen_events,
            slave: Some(pty.slave),
            original_termios,
        }
    }

    pub(super) fn pid(&self) -> Pid {
        child_pid(
            self.child
                .as_ref()
                .expect("the PTY child must still be owned"),
        )
        .expect("the PTY child process ID must fit pid_t")
    }

    pub(super) fn input(&mut self) -> &mut File {
        self.input
            .as_mut()
            .expect("the PTY input must still be owned")
    }

    pub(super) fn slave(&self) -> &std::os::fd::OwnedFd {
        self.slave
            .as_ref()
            .expect("the PTY slave must still be owned")
    }

    pub(super) fn wait_until_ready(&mut self) {
        self.wait_until_ready_marker(0);
    }

    pub(super) fn wait_until_ready_marker(&mut self, marker: usize) -> usize {
        self.wait_until_ready_marker_after_with_timeout(marker, 0, Duration::from_secs(5))
            .unwrap_or_else(|error| panic!("the child must emit ready marker {marker}: {error}"))
    }

    pub(super) fn wait_until_ready_marker_after(&mut self, marker: usize, offset: usize) -> usize {
        self.wait_until_ready_marker_after_with_timeout(marker, offset, Duration::from_secs(5))
            .unwrap_or_else(|error| {
                panic!(
                    "the child must emit ready marker {marker} after output offset {offset}: {error}"
                )
            })
    }

    pub(super) fn wait_until_ready_marker_with_timeout(
        mut self,
        marker: usize,
        timeout: Duration,
    ) -> Result<(Self, usize), ReadinessWaitFailure> {
        let end_offset = self.wait_until_ready_marker_after_with_timeout(marker, 0, timeout)?;
        Ok((self, end_offset))
    }

    fn wait_until_ready_marker_after_with_timeout(
        &mut self,
        marker: usize,
        offset: usize,
        timeout: Duration,
    ) -> Result<usize, ReadinessWaitFailure> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            match self.ready_events[marker].recv_timeout(remaining) {
                Ok(end_offset) if end_offset > offset => return Ok(end_offset),
                Ok(_) => continue,
                Err(error) => {
                    return Err(ReadinessWaitFailure {
                        cause: error,
                        cleanup: self.terminate_child(),
                        output: self.captured_output(),
                    });
                },
            }
        }
    }

    pub(super) fn wait_until_output_reaches(&mut self, offset: usize) -> usize {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            match self.output_events.recv_timeout(remaining) {
                Ok(end_offset) if end_offset >= offset => return end_offset,
                Ok(_) => continue,
                Err(error) => {
                    let cleanup = self
                        .terminate_child()
                        .map_or_else(|failure| failure, |receipt| format!("{receipt:?}"));
                    panic!(
                        "the child must emit output past offset {offset}: {error}; child cleanup: {cleanup}"
                    );
                },
            }
        }
    }

    #[cfg(target_os = "linux")]
    pub(super) fn wait_for_screen(&mut self, expected: ScreenEvent) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let event = match self.screen_events.recv_timeout(remaining) {
                Ok(event) => event,
                Err(error) => {
                    let cleanup = self
                        .terminate_child()
                        .map_or_else(|failure| failure, |receipt| format!("{receipt:?}"));
                    panic!(
                        "the child must complete the expected screen transition: {error}; child cleanup: {cleanup}"
                    );
                },
            };
            if event == expected {
                return;
            }
        }
    }

    #[cfg(target_os = "linux")]
    pub(super) fn wait_until_stopped(&mut self) {
        let pid = self.pid();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match waitpid(pid, Some(WaitPidFlag::WUNTRACED | WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::Stopped(stopped, Signal::SIGTSTP)) if stopped == pid => return,
                Ok(WaitStatus::StillAlive) | Err(nix::errno::Errno::EINTR) => {},
                Ok(status @ WaitStatus::Exited(..)) | Ok(status @ WaitStatus::Signaled(..)) => {
                    panic!("the child terminated before entering the stopped state: {status:?}");
                },
                Err(nix::errno::Errno::ECHILD) => {
                    panic!("the child disappeared before entering the stopped state");
                },
                status => {
                    let cleanup = self
                        .terminate_child()
                        .map_or_else(|failure| failure, |receipt| format!("{receipt:?}"));
                    panic!(
                        "expected SIGTSTP-stopped child, got {status:?}; child cleanup: {cleanup}"
                    );
                },
            }
            if std::time::Instant::now() >= deadline {
                let cleanup = self
                    .terminate_child()
                    .map_or_else(|failure| failure, |receipt| format!("{receipt:?}"));
                panic!(
                    "the child did not enter the SIGTSTP-stopped state; child cleanup: {cleanup}"
                );
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn terminate_child(&mut self) -> Result<ChildReapReceipt, String> {
        terminate_owned_child(&mut self.child)
    }

    fn captured_output(&self) -> Vec<u8> {
        self.captured_output
            .lock()
            .expect("the PTY output snapshot mutex must remain usable")
            .clone()
    }

    pub(super) fn release_capture(&mut self) {
        self.capture_release
            .take()
            .expect("the configured capture pause must still be active")
            .send(())
            .expect("the PTY capture thread must await its release");
    }

    pub(super) fn resize(&self, columns: u16, rows: u16) {
        let size = rustix::termios::Winsize {
            ws_row: rows,
            ws_col: columns,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        rustix::termios::tcsetwinsize(self.slave(), size)
            .expect("updating the PTY window size must succeed");
        assert_eq!(
            rustix::termios::tcgetwinsize(self.slave())
                .expect("reading back the PTY window size must succeed"),
            size,
            "the PTY kernel state must expose the requested resize before capture resumes"
        );
    }

    pub(super) fn finish(mut self) -> (std::process::ExitStatus, Vec<u8>) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let (status, timeout_cleanup) = loop {
            if let Some(status) = self
                .child
                .as_mut()
                .expect("finish requires a live PTY child owner")
                .try_wait()
                .unwrap()
            {
                self.child.take();
                break (Some(status), None);
            }
            if std::time::Instant::now() >= deadline {
                break (None, Some(self.terminate_child()));
            }
            thread::sleep(Duration::from_millis(10));
        };
        if let Some(Err(cleanup)) = &timeout_cleanup {
            panic!(
                "the PTY child exceeded its cleanup deadline; child cleanup failed without joining the reader: {cleanup}; PTY output:\n{}",
                String::from_utf8_lossy(&self.captured_output())
            );
        }
        let restored_termios = tcgetattr(self.slave()).unwrap();
        if let Some(release) = self.capture_release.take() {
            let _ = release.send(());
        }
        self.input.take();
        self.slave.take();
        let output = self
            .output
            .take()
            .expect("finish must still own the PTY capture thread")
            .join()
            .expect("the PTY capture thread must not panic");
        if let Some(Ok(receipt)) = timeout_cleanup {
            panic!(
                "the PTY child exceeded its cleanup deadline; child cleanup completed with {receipt:?}:\n{}",
                String::from_utf8_lossy(&output)
            );
        }
        assert_eq!(
            restored_termios, self.original_termios,
            "the child must restore the PTY termios state"
        );
        (
            status.expect("a child that did not time out has an exit status"),
            output,
        )
    }

    fn join_output(&mut self, timeout: Duration) -> Result<Vec<u8>, String> {
        let Some(output) = self.output.take() else {
            return Ok(self.captured_output());
        };
        let (result_tx, result_rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = result_tx.send(output.join());
        });
        match result_rx.recv_timeout(timeout) {
            Ok(Ok(output)) => Ok(output),
            Ok(Err(_)) => Err("PTY capture thread panicked".to_owned()),
            Err(error) => Err(format!(
                "PTY capture join exceeded the bounded deadline: {error}"
            )),
        }
    }
}

impl Drop for PtyChild {
    fn drop(&mut self) {
        if self.child.is_some() {
            let _ = self.terminate_child();
        }
        if let Some(release) = self.capture_release.take() {
            let _ = release.send(());
        }
        self.input.take();
        self.slave.take();
        let _ = self.join_output(Duration::from_secs(1));
    }
}

fn child_pid(child: &std::process::Child) -> Result<Pid, String> {
    i32::try_from(child.id())
        .map(Pid::from_raw)
        .map_err(|_| format!("PTY child process ID {} does not fit pid_t", child.id()))
}

fn terminate_owned_child(
    child: &mut Option<std::process::Child>,
) -> Result<ChildReapReceipt, String> {
    let Some(owned_child) = child.as_ref() else {
        return Ok(ChildReapReceipt::AlreadyReaped);
    };
    let pid = child_pid(owned_child)?;
    let signal_error = match kill(pid, Signal::SIGKILL) {
        Ok(()) | Err(nix::errno::Errno::ESRCH) => None,
        Err(error) => Some(error),
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(1);

    loop {
        match waitpid(pid, Some(WaitPidFlag::WNOHANG)) {
            Ok(status @ WaitStatus::Exited(reaped, _))
            | Ok(status @ WaitStatus::Signaled(reaped, _, _))
                if reaped == pid =>
            {
                child.take();
                if let Some(error) = signal_error {
                    return Err(format!(
                        "sending SIGKILL failed before the child exited: {error}"
                    ));
                }
                return Ok(ChildReapReceipt::Waitpid(status));
            },
            Err(nix::errno::Errno::ECHILD) => {
                child.take();
                if let Some(error) = signal_error {
                    return Err(format!(
                        "sending SIGKILL failed before another owner reaped the child: {error}"
                    ));
                }
                return Ok(ChildReapReceipt::AlreadyReaped);
            },
            Ok(WaitStatus::StillAlive) | Err(nix::errno::Errno::EINTR) => {},
            result => {
                return Err(format!(
                    "waiting for SIGKILL termination returned {result:?}"
                ));
            },
        }
        if std::time::Instant::now() >= deadline {
            return Err(
                "waiting for SIGKILL termination exceeded the one-second reap deadline".to_owned(),
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn cleanup_diagnostic(cleanup: &Result<ChildReapReceipt, String>) -> String {
    cleanup
        .as_ref()
        .map_or_else(|failure| failure.clone(), |receipt| format!("{receipt:?}"))
}

pub(super) fn assert_child_is_gone(pid: Pid) {
    assert_eq!(kill(pid, None), Err(nix::errno::Errno::ESRCH));
    assert_eq!(
        waitpid(pid, Some(WaitPidFlag::WNOHANG)),
        Err(nix::errno::Errno::ECHILD)
    );
}

fn capture_pty(
    mut master: File,
    ready_events: Vec<(mpsc::Sender<usize>, &'static [u8])>,
    output_events: mpsc::Sender<usize>,
    screen_events: mpsc::Sender<ScreenEvent>,
    captured_output: Arc<Mutex<Vec<u8>>>,
    mut capture_pause: Option<(usize, mpsc::Receiver<()>)>,
) -> Vec<u8> {
    let mut output = Vec::new();
    let mut ready_counts = vec![0; ready_events.len()];
    let mut enter_count = 0;
    let mut leave_count = 0;
    let mut chunk = [0; 4096];
    loop {
        match master.read(&mut chunk) {
            Ok(0) => break,
            Ok(length) => {
                output.extend_from_slice(&chunk[..length]);
                let mut snapshot = captured_output
                    .lock()
                    .expect("the PTY output snapshot mutex must remain usable");
                snapshot.extend_from_slice(&chunk[..length]);
                if snapshot.len() > 8 * 1024 {
                    let overflow = snapshot.len() - 8 * 1024;
                    snapshot.drain(..overflow);
                }
                drop(snapshot);
                let mut should_pause = false;
                for (marker_index, ((sender, marker), ready_count)) in
                    ready_events.iter().zip(&mut ready_counts).enumerate()
                {
                    let previous_ready_count = *ready_count;
                    let mut next_ready_count = 0;
                    for (start, candidate) in output.windows(marker.len()).enumerate() {
                        if candidate != *marker {
                            continue;
                        }
                        if next_ready_count >= *ready_count {
                            let _ = sender.send(start + marker.len());
                        }
                        next_ready_count += 1;
                    }
                    *ready_count = next_ready_count;
                    should_pause |= capture_pause
                        .as_ref()
                        .is_some_and(|(pause_marker, _)| *pause_marker == marker_index)
                        && next_ready_count > previous_ready_count;
                }
                if should_pause && let Some((_, release)) = capture_pause.take() {
                    let _ = release.recv();
                }
                let next_enter_count = output
                    .windows(ENTER_ALTERNATE_SCREEN.len())
                    .filter(|candidate| *candidate == ENTER_ALTERNATE_SCREEN)
                    .count();
                for _ in enter_count..next_enter_count {
                    let _ = screen_events.send(ScreenEvent::Entered);
                }
                enter_count = next_enter_count;
                let next_leave_count = output
                    .windows(LEAVE_ALTERNATE_SCREEN.len())
                    .filter(|candidate| *candidate == LEAVE_ALTERNATE_SCREEN)
                    .count();
                for _ in leave_count..next_leave_count {
                    let _ = screen_events.send(ScreenEvent::Left);
                }
                leave_count = next_leave_count;
                let _ = output_events.send(output.len());
            },
            Err(error) if error.raw_os_error() == Some(nix::libc::EIO) => break,
            Err(error) => panic!("reading PTY output failed: {error}"),
        }
    }
    output
}

fn id<T>(constructor: impl FnOnce(NonZeroU64) -> T) -> T {
    constructor(NonZeroU64::MIN)
}

pub(super) fn assert_fullscreen_pair(output: &[u8]) {
    let enter = output
        .windows(ENTER_ALTERNATE_SCREEN.len())
        .position(|candidate| candidate == ENTER_ALTERNATE_SCREEN)
        .expect("fullscreen output must enter the alternate screen");
    let leave = output
        .windows(LEAVE_ALTERNATE_SCREEN.len())
        .position(|candidate| candidate == LEAVE_ALTERNATE_SCREEN)
        .expect("fullscreen output must leave the alternate screen");

    assert!(enter < leave, "alternate-screen cleanup must follow entry");
    assert_eq!(
        output
            .windows(ENTER_ALTERNATE_SCREEN.len())
            .filter(|candidate| *candidate == ENTER_ALTERNATE_SCREEN)
            .count(),
        1
    );
    assert_eq!(
        output
            .windows(LEAVE_ALTERNATE_SCREEN.len())
            .filter(|candidate| *candidate == LEAVE_ALTERNATE_SCREEN)
            .count(),
        1
    );
}

pub(super) fn run_fullscreen(
    termination: &mut impl TerminationSource,
) -> Result<(), Box<dyn Error>> {
    let mut agent = PendingAgent;
    yo_tui::run_with_mode(
        termination,
        &mut agent,
        PresentationMode::Fullscreen,
        yo_tui::ColorCapability::Unknown,
        yo_tui::MotionPreference::Standard,
    )?;
    Ok(())
}
