use std::{
    error::Error,
    fs::File,
    io::{self, Read, Write},
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
    },
    unistd::Pid,
};
use yo_core::RuntimePoll;
use yo_tui::{
    AgentAction, AgentConnection, DispatchOutcome, PendingDispatch, PresentationMode,
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

    fn poll(&mut self) -> Result<RuntimePoll, Self::Error> {
        Ok(RuntimePoll::Pending)
    }
}

struct PtyChild {
    child: std::process::Child,
    input: File,
    output: thread::JoinHandle<Vec<u8>>,
    entered: mpsc::Receiver<()>,
    slave: std::os::fd::OwnedFd,
    original_termios: nix::sys::termios::Termios,
}

impl PtyChild {
    fn spawn(test_name: &str) -> Self {
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
        let (entered_tx, entered) = mpsc::sync_channel(1);
        let output = thread::spawn(move || capture_pty(master, entered_tx));

        Self {
            child,
            input,
            output,
            entered,
            slave: pty.slave,
            original_termios,
        }
    }

    fn wait_for_fullscreen(&self) {
        self.entered
            .recv_timeout(Duration::from_secs(5))
            .expect("the child must enter alternate-screen mode");
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

fn capture_pty(mut master: File, entered: mpsc::SyncSender<()>) -> Vec<u8> {
    let mut output = Vec::new();
    let mut announced = false;
    let mut chunk = [0; 4096];
    loop {
        match master.read(&mut chunk) {
            Ok(0) => break,
            Ok(length) => {
                output.extend_from_slice(&chunk[..length]);
                if !announced && contains(&output, ENTER_ALTERNATE_SCREEN) {
                    announced = true;
                    let _ = entered.send(());
                }
                if contains(&output, LEAVE_ALTERNATE_SCREEN) {
                    break;
                }
            },
            Err(error) if error.raw_os_error() == Some(nix::libc::EIO) => break,
            Err(error) => panic!("reading PTY output failed: {error}"),
        }
    }
    output
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|candidate| candidate == needle)
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

// 실제 Linux PTY에서 Ctrl+D 정상 종료가 대체 화면과 termios를 모두 원래 상태로 복구한다.
#[test]
fn fullscreen_normal_exit_restores_real_pty() {
    let mut child = PtyChild::spawn("pty_tests::child_fullscreen_normal_exit");
    child.wait_for_fullscreen();
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

    let child = PtyChild::spawn("pty_tests::child_fullscreen_termination");
    child.wait_for_fullscreen();
    kill(
        Pid::from_raw(i32::try_from(child.child.id()).unwrap()),
        Signal::SIGTERM,
    )
    .unwrap();

    let (status, output) = child.finish();
    assert_eq!(status.signal(), Some(Signal::SIGTERM as i32));
    assert_fullscreen_pair(&output);
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
