use std::{
    collections::VecDeque,
    error::Error,
    fs::File,
    io::{self, Read, Write},
    num::NonZeroU64,
    process::{Command, Stdio},
    sync::mpsc,
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
    unistd::{Pid, setpgid},
};
use yo_core::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentEvent,
    TranscriptRecord, TurnId, TurnRef,
};
use yo_tui::{
    AgentAction, AgentConnection, AgentPoll, DispatchOutcome, PendingDispatch, PresentationMode,
    TerminationEvent, TerminationSource,
};

use crate::process::termination::TerminationCoordinator;

const CHILD_MARKER: &str = "YO_PTY_CHILD";
const ENTER_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049h";
const LEAVE_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049l";

struct PendingTermination;

impl TerminationSource for PendingTermination {
    fn poll_termination(&mut self) -> TerminationEvent {
        TerminationEvent::None
    }
}

struct PendingAgent;

impl AgentConnection for PendingAgent {
    type Error = io::Error;

    fn dispatch(&mut self, _action: AgentAction) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Accepted)
    }

    fn retry(&mut self, _pending: PendingDispatch) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Accepted)
    }

    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        Ok(AgentPoll::Pending)
    }
}

struct RetainedChatAgent {
    records: VecDeque<TranscriptRecord>,
}

impl RetainedChatAgent {
    fn new() -> Self {
        let session_id = "01890f00-0000-7000-8000-000000000001"
            .parse()
            .expect("the fixture is a UUIDv7");
        let turn = TurnRef::new(session_id, id(TurnId::new));
        let activity = ActivityRef::new(turn, id(ActivityId::new));
        Self {
            records: [
                AgentEvent::ActivityStarted {
                    activity,
                    kind: ActivityKind::AgentMessage,
                },
                AgentEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot("YO_INLINE_RETAINED".to_owned()),
                },
                AgentEvent::ActivityFinished {
                    activity,
                    outcome: ActivityOutcome::Completed,
                },
            ]
            .map(TranscriptRecord::EventCommitted)
            .into(),
        }
    }
}

impl AgentConnection for RetainedChatAgent {
    type Error = io::Error;

    fn dispatch(&mut self, _action: AgentAction) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Accepted)
    }

    fn retry(&mut self, _pending: PendingDispatch) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Accepted)
    }

    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        Ok(self
            .records
            .pop_front()
            .map_or(AgentPoll::Pending, AgentPoll::Record))
    }
}

struct PtyChild {
    child: std::process::Child,
    input: File,
    output: thread::JoinHandle<Vec<u8>>,
    ready_events: mpsc::Receiver<()>,
    screen_events: mpsc::Receiver<ScreenEvent>,
    slave: std::os::fd::OwnedFd,
    original_termios: nix::sys::termios::Termios,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScreenEvent {
    Entered,
    Left,
}

impl PtyChild {
    fn spawn(test_name: &str, ready_marker: &'static [u8]) -> Self {
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

    fn wait_until_ready(&self) {
        self.ready_events
            .recv_timeout(Duration::from_secs(5))
            .expect("the child must emit its mode-specific ready marker");
    }

    fn wait_for_screen(&self, expected: ScreenEvent) {
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

    fn wait_until_stopped(&self) {
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

    fn finish(mut self) -> (std::process::ExitStatus, Vec<u8>) {
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

fn position(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .rposition(|candidate| candidate == needle)
        .expect("expected bytes must be present")
}

fn id<T>(constructor: impl FnOnce(NonZeroU64) -> T) -> T {
    constructor(NonZeroU64::MIN)
}

fn assert_fullscreen_pair(output: &[u8]) {
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

fn run_fullscreen(termination: &mut impl TerminationSource) -> Result<(), Box<dyn Error>> {
    let mut agent = PendingAgent;
    yo_tui::run_with_mode(termination, &mut agent, PresentationMode::Fullscreen)?;
    Ok(())
}

fn run_inline_with_retained_chat(
    termination: &mut impl TerminationSource,
) -> Result<(), Box<dyn Error>> {
    let mut agent = RetainedChatAgent::new();
    let outcome = yo_tui::run_with_mode(termination, &mut agent, PresentationMode::Inline)?;
    match outcome {
        yo_tui::TerminalOutcome::Exited(outcome) => {
            if let Some(output) = outcome.output() {
                super::write_session_output(output)?;
            }
        },
        yo_tui::TerminalOutcome::SuspendRequested => {
            return Err("unexpected suspension in retained-chat PTY helper".into());
        },
        _ => return Err("unsupported terminal outcome in retained-chat PTY helper".into()),
    }
    Ok(())
}

// 실제 Linux PTY에서 Inline viewport가 지워진 뒤 일반 대화 뷰의 텍스트가 같은 main
// screen에 다시 출력되어 native scrollback으로 남는지 확인한다.
#[test]
fn inline_normal_exit_retains_chat_after_viewport_restoration() {
    const RETAINED: &[u8] = "⏺ YO_INLINE_RETAINED\r\n".as_bytes();
    let mut child = PtyChild::spawn(
        "pty_tests::child_inline_retains_chat",
        b"YO_INLINE_RETAINED",
    );
    child.wait_until_ready();
    child.input.write_all(&[0x04]).unwrap();
    child.input.flush().unwrap();

    let (status, output) = child.finish();
    assert!(
        status.success(),
        "child failed:\n{}",
        String::from_utf8_lossy(&output)
    );
    assert!(
        position(&output, b"\x1b[2K") < position(&output, RETAINED),
        "plain chat output must follow viewport restoration:\n{}",
        String::from_utf8_lossy(&output)
    );
}

// 실제 Linux PTY에서 Ctrl+D 정상 종료가 대체 화면과 termios를 모두 원래 상태로 복구한다.
#[test]
fn fullscreen_normal_exit_restores_real_pty() {
    let mut child = PtyChild::spawn(
        "pty_tests::child_fullscreen_normal_exit",
        ENTER_ALTERNATE_SCREEN,
    );
    child.wait_until_ready();
    child.input.write_all(&[0x04]).unwrap();
    child.input.flush().unwrap();

    let (status, output) = child.finish();
    assert!(
        status.success(),
        "child failed:\n{}",
        String::from_utf8_lossy(&output)
    );
    assert_fullscreen_pair(&output);
}

// 실제 Linux PTY에서 SIGTERM을 받아도 화면과 termios를 먼저 복구한 뒤 같은 SIGTERM을 재생한다.
#[test]
fn fullscreen_termination_restores_real_pty_before_signal_replay() {
    use std::os::unix::process::ExitStatusExt;

    let child = PtyChild::spawn(
        "pty_tests::child_fullscreen_termination",
        ENTER_ALTERNATE_SCREEN,
    );
    child.wait_until_ready();
    kill(
        Pid::from_raw(i32::try_from(child.child.id()).unwrap()),
        Signal::SIGTERM,
    )
    .unwrap();

    let (status, output) = child.finish();
    assert_eq!(status.signal(), Some(Signal::SIGTERM as i32));
    assert_fullscreen_pair(&output);
}

// 실제 Linux PTY에서 두 번 연속 Ctrl+Z로 terminal을 완전히 복구해 프로세스를 멈추고,
// SIGCONT마다 같은 TUI state로 새 Fullscreen 세대를 획득한 뒤 정상 종료한다.
#[test]
fn fullscreen_repeated_suspend_resume_restores_each_terminal_generation() {
    let mut child = PtyChild::spawn(
        "pty_tests::child_fullscreen_repeated_suspend_resume",
        ENTER_ALTERNATE_SCREEN,
    );
    child.wait_until_ready();
    child.wait_for_screen(ScreenEvent::Entered);

    for _ in 0..2 {
        child.input.write_all(&[0x1a]).unwrap();
        child.input.flush().unwrap();
        child.wait_for_screen(ScreenEvent::Left);
        child.wait_until_stopped();
        assert_eq!(
            tcgetattr(&child.slave).unwrap(),
            child.original_termios,
            "each stopped interval must expose the original PTY termios"
        );
        kill(
            Pid::from_raw(i32::try_from(child.child.id()).unwrap()),
            Signal::SIGCONT,
        )
        .unwrap();
        child.wait_for_screen(ScreenEvent::Entered);
    }

    child.input.write_all(&[0x04]).unwrap();
    child.input.flush().unwrap();
    let (status, output) = child.finish();

    assert!(
        status.success(),
        "child failed:\n{}",
        String::from_utf8_lossy(&output)
    );
    assert_eq!(
        output
            .windows(ENTER_ALTERNATE_SCREEN.len())
            .filter(|candidate| *candidate == ENTER_ALTERNATE_SCREEN)
            .count(),
        3
    );
    assert_eq!(
        output
            .windows(LEAVE_ALTERNATE_SCREEN.len())
            .filter(|candidate| *candidate == LEAVE_ALTERNATE_SCREEN)
            .count(),
        3
    );
}

// 실제 Linux PTY의 Inline mode에서도 두 번 연속 일시정지할 때마다 viewport와 termios를
// 복구하고, SIGCONT 뒤 같은 대화 상태를 새 viewport의 첫 전체 frame으로 다시 그린다.
#[test]
fn inline_repeated_suspend_resume_reacquires_a_fresh_viewport() {
    const RETAINED: &[u8] = b"YO_INLINE_RETAINED";
    let mut child = PtyChild::spawn("pty_tests::child_inline_repeated_suspend_resume", RETAINED);
    child.wait_until_ready();

    for _ in 0..2 {
        child.input.write_all(&[0x1a]).unwrap();
        child.input.flush().unwrap();
        child.wait_until_stopped();
        assert_eq!(
            tcgetattr(&child.slave).unwrap(),
            child.original_termios,
            "each stopped interval must expose the original PTY termios"
        );
        kill(
            Pid::from_raw(i32::try_from(child.child.id()).unwrap()),
            Signal::SIGCONT,
        )
        .unwrap();
        child.wait_until_ready();
    }

    child.input.write_all(&[0x04]).unwrap();
    child.input.flush().unwrap();
    let (status, output) = child.finish();

    assert!(
        status.success(),
        "child failed:\n{}",
        String::from_utf8_lossy(&output)
    );
    assert_eq!(
        output
            .windows(RETAINED.len())
            .filter(|candidate| *candidate == RETAINED)
            .count(),
        4,
        "one frame per terminal generation plus one final plain transcript is expected"
    );
}

// 부모 테스트가 마련한 PTY 안에서 정상 Ctrl+D 종료 경로만 실행하는 자식 진입점이다.
#[test]
#[ignore]
fn child_fullscreen_normal_exit() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }
    run_fullscreen(&mut PendingTermination).unwrap();
}

// 부모 테스트가 마련한 PTY에서 deterministic 일반 대화를 그린 뒤 Inline 정상 종료한다.
#[test]
#[ignore]
fn child_inline_retains_chat() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }
    run_inline_with_retained_chat(&mut PendingTermination).unwrap();
}

// 부모 테스트가 마련한 PTY 안에서 실제 process coordinator와 Fullscreen을 함께 실행한다.
#[test]
#[ignore]
fn child_fullscreen_termination() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }
    let mut coordinator = TerminationCoordinator::install().unwrap();
    coordinator
        .with_active_session(run_fullscreen)
        .unwrap()
        .unwrap();
    coordinator.shutdown().unwrap();
}

// 부모가 제공한 실제 PTY에서 하나의 TUI session과 agent를 유지한 채 terminal 소유권만
// 반복해서 닫고 다시 여는 자식 진입점이다.
#[test]
#[ignore]
fn child_fullscreen_repeated_suspend_resume() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }
    setpgid(Pid::from_raw(0), Pid::from_raw(0)).unwrap();
    let mut coordinator = TerminationCoordinator::install().unwrap();
    let mut job_control = crate::process::job_control::JobControl::new();
    let mut agent = PendingAgent;
    let mut tui = yo_tui::TuiSession::new();

    loop {
        let outcome = coordinator
            .with_active_session(|termination| {
                yo_tui::run_session_with_mode(
                    termination,
                    &mut agent,
                    &mut tui,
                    PresentationMode::Fullscreen,
                )
            })
            .unwrap()
            .unwrap();
        match outcome {
            yo_tui::TerminalOutcome::SuspendRequested => job_control.suspend().unwrap(),
            yo_tui::TerminalOutcome::Exited(_) => break,
            _ => panic!("unsupported terminal outcome"),
        }
    }
    coordinator.shutdown().unwrap();
}

// 부모가 제공한 실제 PTY에서 Inline viewport만 반복해서 교체하며 TUI session과 agent
// 상태는 계속 보존하는 자식 진입점이다.
#[test]
#[ignore]
fn child_inline_repeated_suspend_resume() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }
    setpgid(Pid::from_raw(0), Pid::from_raw(0)).unwrap();
    let mut coordinator = TerminationCoordinator::install().unwrap();
    let mut job_control = crate::process::job_control::JobControl::new();
    let mut agent = RetainedChatAgent::new();
    let mut tui = yo_tui::TuiSession::new();

    loop {
        let outcome = coordinator
            .with_active_session(|termination| {
                yo_tui::run_session_with_mode(
                    termination,
                    &mut agent,
                    &mut tui,
                    PresentationMode::Inline,
                )
            })
            .unwrap()
            .unwrap();
        match outcome {
            yo_tui::TerminalOutcome::SuspendRequested => job_control.suspend().unwrap(),
            yo_tui::TerminalOutcome::Exited(outcome) => {
                if let Some(output) = outcome.output() {
                    super::write_session_output(output).unwrap();
                }
                break;
            },
            _ => panic!("unsupported terminal outcome"),
        }
    }
    coordinator.shutdown().unwrap();
}
