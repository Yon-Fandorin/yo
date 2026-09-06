use nix::sys::signal::{Signal, kill};

use super::support::{
    CHILD_MARKER, ENTER_ALTERNATE_SCREEN, PtyChild,
    tui::{assert_fullscreen_pair, run_fullscreen},
};
use crate::execution::process::termination::TerminationCoordinator;

// 실제 Linux PTY에서 SIGTERM을 받아도 화면과 termios를 먼저 복구한 뒤 같은 SIGTERM을 재생한다.
#[test]
fn fullscreen_termination_restores_real_pty_before_signal_replay() {
    use std::os::unix::process::ExitStatusExt;

    let mut child = PtyChild::spawn(
        "pty_tests::termination::child_fullscreen_termination",
        ENTER_ALTERNATE_SCREEN,
    );
    child.wait_until_ready();
    kill(child.pid(), Signal::SIGTERM).unwrap();

    let (status, output) = child.finish();
    assert_eq!(status.signal(), Some(Signal::SIGTERM as i32));
    assert_fullscreen_pair(&output);
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
