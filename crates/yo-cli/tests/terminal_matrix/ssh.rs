use std::time::Duration;

use server::SshServer;

#[path = "ssh/server.rs"]
mod server;
#[path = "ssh/session.rs"]
mod session;
#[path = "ssh/suspend.rs"]
mod suspend;
#[path = "ssh/tmux.rs"]
mod tmux;

const READY_TIMEOUT: Duration = Duration::from_secs(10);
const EXIT_TIMEOUT: Duration = Duration::from_secs(5);
const HIDE_CURSOR: &[u8] = b"\x1b[?25l";
const SHOW_CURSOR: &[u8] = b"\x1b[?25h";
const ENTER_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049h";
const LEAVE_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049l";
const RESTORED_MARKER: &[u8] = b"YO_SSH_TERMIOS_RESTORED";

// 격리된 localhost SSH PTY의 main screen에서 Inline의 화면 출력이 시작된 뒤 빈 Ctrl+D로
// 정상 종료하고, 마지막 cursor 명령과 원격 PTY termios가 복구 상태인지 확인한다.
#[test]
#[ignore = "requires local sshd and a compatible installed Codex"]
fn ssh_inline_exits_cleanly_and_restores_remote_pty() {
    SshServer::start().run_mode("--inline", false);
}

// 격리된 localhost SSH PTY에서 Fullscreen이 alternate screen을 획득한 뒤 빈 Ctrl+D로
// 정상 종료하고, 소유한 alternate screen과 원격 PTY termios를 모두 복구하는지 확인한다.
#[test]
#[ignore = "requires local sshd and a compatible installed Codex"]
fn ssh_fullscreen_exits_cleanly_and_restores_remote_pty() {
    SshServer::start().run_mode("--fullscreen", true);
}

// 격리된 SSH PTY 안에서 tmux와 Inline을 함께 실행해, tmux pane이 main screen과
// raw 입력 상태에 도달한 뒤 빈 Ctrl+D로 전체 계층이 정상 종료·정리되는지 확인한다.
#[test]
#[ignore = "requires local sshd, tmux, and a compatible installed Codex"]
fn ssh_tmux_inline_exits_cleanly_and_restores_remote_pty() {
    SshServer::start().run_tmux_mode("--inline", false);
}

// 격리된 SSH PTY 안에서 tmux와 Fullscreen을 함께 실행해, 내부 pane이 alternate
// screen과 raw 입력 상태에 도달한 뒤 빈 Ctrl+D로 전체 계층이 정상 종료·정리되는지 확인한다.
#[test]
#[ignore = "requires local sshd, tmux, and a compatible installed Codex"]
fn ssh_tmux_fullscreen_exits_cleanly_and_restores_remote_pty() {
    SshServer::start().run_tmux_mode("--fullscreen", true);
}
