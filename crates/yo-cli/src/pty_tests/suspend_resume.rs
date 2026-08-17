use std::{io::Write, panic::AssertUnwindSafe};

use nix::{
    sys::{
        signal::{Signal, kill},
        termios::tcgetattr,
    },
    unistd::{Pid, setpgid},
};
use yo_tui::PresentationMode;

use super::support::{
    CHILD_MARKER, ENTER_ALTERNATE_SCREEN, LEAVE_ALTERNATE_SCREEN, PendingAgent, PtyChild,
    RetainedChatAgent, ScreenEvent, assert_child_is_gone,
};
use crate::process::termination::TerminationCoordinator;

// 실제 Linux PTY에서 두 번 연속 Ctrl+Z로 terminal을 완전히 복구해 프로세스를 멈추고,
// SIGCONT마다 같은 TUI state로 새 Fullscreen 세대를 획득한 뒤 정상 종료한다.
#[test]
fn fullscreen_repeated_suspend_resume_restores_each_terminal_generation() {
    let mut child = PtyChild::spawn(
        "pty_tests::suspend_resume::child_fullscreen_repeated_suspend_resume",
        ENTER_ALTERNATE_SCREEN,
    );
    child.wait_until_ready();
    child.wait_for_screen(ScreenEvent::Entered);

    for _ in 0..2 {
        child.input().write_all(&[0x1a]).unwrap();
        child.input().flush().unwrap();
        child.wait_for_screen(ScreenEvent::Left);
        child.wait_until_stopped();
        assert_eq!(
            tcgetattr(child.slave()).unwrap(),
            child.original_termios,
            "each stopped interval must expose the original PTY termios"
        );
        kill(child.pid(), Signal::SIGCONT).unwrap();
        child.wait_for_screen(ScreenEvent::Entered);
    }

    child.input().write_all(&[0x04]).unwrap();
    child.input().flush().unwrap();
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
// 복구한다. SIGCONT 뒤에는 publication cursor를 보존해 acknowledged history를 새 live
// viewport나 최종 exit suffix로 재생하지 않는다.
#[test]
fn inline_repeated_suspend_resume_reacquires_a_fresh_viewport() {
    const RETAINED: &[u8] = b"YO_INLINE_RETAINED";
    let mut child = PtyChild::spawn(
        "pty_tests::suspend_resume::child_inline_repeated_suspend_resume",
        b"\x1b[?25l",
    );
    child.wait_until_ready();
    child.wait_until_ready();

    for _ in 0..2 {
        child.input().write_all(&[0x1a]).unwrap();
        child.input().flush().unwrap();
        child.wait_until_stopped();
        assert_eq!(
            tcgetattr(child.slave()).unwrap(),
            child.original_termios,
            "each stopped interval must expose the original PTY termios"
        );
        kill(child.pid(), Signal::SIGCONT).unwrap();
        child.wait_until_ready();
    }

    child.input().write_all(&[0x04]).unwrap();
    child.input().flush().unwrap();
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
        1,
        "suspend/resume and final exit must not replay acknowledged native history"
    );
}

// SIGTSTP-stopped child도 parent assertion unwind에서 Drop owner가 SIGKILL 뒤 exact wait를
// 수행하므로 stopped process나 zombie를 test harness에 남기지 않습니다.
#[test]
fn panic_unwind_reaps_a_stopped_pty_child() {
    let mut pid = None;
    let panic = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let mut child = PtyChild::spawn(
            "pty_tests::suspend_resume::child_inline_repeated_suspend_resume",
            b"\x1b[?25l",
        );
        child.wait_until_ready();
        child.input().write_all(&[0x1a]).unwrap();
        child.input().flush().unwrap();
        child.wait_until_stopped();
        pid = Some(child.pid());
        panic!("injected parent assertion failure while child is stopped");
    }));

    assert!(panic.is_err());
    assert_child_is_gone(pid.unwrap());
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
    let mut tui = yo_tui::TuiSession::new(
        yo_tui::ColorCapability::Unknown,
        yo_tui::MotionPreference::Standard,
    );

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
    let mut tui = yo_tui::TuiSession::new(
        yo_tui::ColorCapability::Unknown,
        yo_tui::MotionPreference::Standard,
    );

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
                    super::super::write_session_output(output).unwrap();
                }
                break;
            },
            _ => panic!("unsupported terminal outcome"),
        }
    }
    coordinator.shutdown().unwrap();
}
