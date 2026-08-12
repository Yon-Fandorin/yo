use std::{
    collections::VecDeque,
    error::Error,
    fs::File,
    io::{self, Read},
    num::NonZeroU64,
    process::{Command, Stdio},
    sync::mpsc,
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
        let session_id = "01890f00-0000-7000-8000-000000000001"
            .parse()
            .expect("the fixture is a UUIDv7");
        let turn = TurnRef::new(session_id, id(TurnId::new));
        Self {
            records: [TranscriptRecord::CommandCommitted(
                AgentCommand::StartTurn {
                    turn,
                    input: UserInput::from("YO_INLINE_RETAINED"),
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
    pub(super) child: std::process::Child,
    pub(super) input: File,
    output: thread::JoinHandle<Vec<u8>>,
    ready_events: mpsc::Receiver<()>,
    screen_events: mpsc::Receiver<ScreenEvent>,
    pub(super) slave: std::os::fd::OwnedFd,
    pub(super) original_termios: nix::sys::termios::Termios,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ScreenEvent {
    Entered,
    Left,
}

impl PtyChild {
    pub(super) fn spawn(test_name: &str, ready_marker: &'static [u8]) -> Self {
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

        let master = File::from(pty.master);
        let input = master.try_clone().unwrap();
        let (ready_tx, ready_events) = mpsc::channel();
        let (screen_tx, screen_events) = mpsc::channel();
        let output = thread::spawn(move || capture_pty(master, ready_tx, screen_tx, ready_marker));

        Self {
            child,
            input,
            output,
            ready_events,
            screen_events,
            slave: pty.slave,
            original_termios,
        }
    }

    pub(super) fn wait_until_ready(&self) {
        self.ready_events
            .recv_timeout(Duration::from_secs(5))
            .expect("the child must emit its mode-specific ready marker");
    }

    pub(super) fn wait_for_screen(&self, expected: ScreenEvent) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let event = self
                .screen_events
                .recv_timeout(remaining)
                .expect("the child must complete the expected screen transition");
            if event == expected {
                return;
            }
        }
    }

    pub(super) fn wait_until_stopped(&self) {
        let pid = Pid::from_raw(i32::try_from(self.child.id()).unwrap());
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match waitpid(pid, Some(WaitPidFlag::WUNTRACED | WaitPidFlag::WNOHANG)).unwrap() {
                WaitStatus::Stopped(stopped, Signal::SIGTSTP) if stopped == pid => return,
                WaitStatus::StillAlive => {},
                status => panic!("expected SIGTSTP-stopped child, got {status:?}"),
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the child did not enter the SIGTSTP-stopped state"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    pub(super) fn resize(&self, columns: u16, rows: u16) {
        let size = rustix::termios::Winsize {
            ws_row: rows,
            ws_col: columns,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        rustix::termios::tcsetwinsize(&self.slave, size)
            .expect("updating the PTY window size must succeed");
        kill(
            Pid::from_raw(i32::try_from(self.child.id()).unwrap()),
            Signal::SIGWINCH,
        )
        .expect("the child must receive the PTY resize notification");
    }

    pub(super) fn finish(mut self) -> (std::process::ExitStatus, Vec<u8>) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let (status, timed_out) = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break (status, false);
            }
            if std::time::Instant::now() >= deadline {
                self.child.kill().unwrap();
                break (self.child.wait().unwrap(), true);
            }
            thread::sleep(Duration::from_millis(10));
        };
        let restored_termios = tcgetattr(&self.slave).unwrap();
        drop(self.input);
        drop(self.slave);
        let output = self.output.join().unwrap();
        assert!(
            !timed_out,
            "the PTY child exceeded its cleanup deadline:\n{}",
            String::from_utf8_lossy(&output)
        );
        assert_eq!(
            restored_termios, self.original_termios,
            "the child must restore the PTY termios state"
        );
        (status, output)
    }
}

fn capture_pty(
    mut master: File,
    ready_events: mpsc::Sender<()>,
    screen_events: mpsc::Sender<ScreenEvent>,
    ready_marker: &[u8],
) -> Vec<u8> {
    let mut output = Vec::new();
    let mut ready_count = 0;
    let mut enter_count = 0;
    let mut leave_count = 0;
    let mut chunk = [0; 4096];
    loop {
        match master.read(&mut chunk) {
            Ok(0) => break,
            Ok(length) => {
                output.extend_from_slice(&chunk[..length]);
                let next_ready_count = output
                    .windows(ready_marker.len())
                    .filter(|candidate| *candidate == ready_marker)
                    .count();
                for _ in ready_count..next_ready_count {
                    let _ = ready_events.send(());
                }
                ready_count = next_ready_count;
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
