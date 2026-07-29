use std::{
    path::Path,
    process::Stdio,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use nix::{sys::termios::Termios, unistd::Pid};

use super::{PaneState, TmuxGuard, capture, remote_tmux_command, require_tmux};
use crate::{
    ssh::{
        ENTER_ALTERNATE_SCREEN, LEAVE_ALTERNATE_SCREEN, RESTORED_MARKER,
        server::SshServer,
        session::{ChildGuard, assert_ordered_pair, wait_for_exit},
    },
    support::{
        contains, only_child, process_exists, process_is_stopped, read_termios, repository_path,
        shell_quote,
    },
};

impl SshServer {
    fn run_tmux_repeated_suspend_resume(&self, option: &str, alternate_screen: bool) {
        require_tmux();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos();
        let socket = self.fixture_path(&format!(
            "tmux-suspend-{}-{unique}.sock",
            std::process::id()
        ));
        let session = "yo";
        let tmux = TmuxGuard::new(socket.clone());
        let repository = repository_path();
        let yo_path = Path::new(env!("CARGO_BIN_EXE_yo"))
            .canonicalize()
            .expect("canonicalize yo binary");
        let inner_command = format!(
            "env PATH={codex_directory}:/usr/bin:/bin \
             PS1='YO_NESTED_TMUX_READY>' HISTFILE=/dev/null \
             /bin/bash --noprofile --norc -i",
            codex_directory =
                shell_quote(self.codex.parent().expect("Codex executable has a parent")),
        );
        let remote = remote_tmux_command(&repository, &socket, session, &inner_command);
        let mut child = ChildGuard::new(
            self.client(true)
                .arg(remote)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("start SSH PTY containing job-control tmux"),
        );
        let stdout_reader = capture(child.stdout.take().expect("capture SSH tmux stdout"));
        let stderr_reader = capture(child.stderr.take().expect("capture SSH tmux stderr"));

        let shell = tmux.wait_until_shell_ready(session);
        let baseline = read_termios(&shell.tty).expect("read tmux shell termios");
        // Readline may settle its prompt termios just after the prompt becomes visible.
        thread::sleep(Duration::from_secs(1));
        tmux.send_literal(session, &format!("{} {option}", shell_quote(&yo_path)));
        tmux.send_key(session, "Enter");
        let running = tmux.wait_until_ready(session, alternate_screen);
        let yo_pid = only_child(Pid::from_raw(running.shell_pid))
            .expect("read the only yo child inside tmux");

        for _ in 0..2 {
            child.send_input(&[0x1a]);
            tmux.wait_until_suspended(session, yo_pid, &baseline);
            thread::sleep(Duration::from_millis(100));
            child.send_input(b"fg\n");
            tmux.wait_until_ready(session, alternate_screen);
        }

        child.send_input(&[0x04]);
        tmux.wait_until_shell_after_exit(session, yo_pid, &baseline);
        thread::sleep(Duration::from_millis(100));
        child.send_input(b"result=$?; exit \"$result\"\n");
        let pane_exit = tmux.wait_until_dead(session);
        assert_eq!(pane_exit.status, Some(0));
        tmux.kill_session(session);

        let status = wait_for_exit(&mut child);
        drop(child.stdin.take());
        let output = stdout_reader.join().expect("join SSH tmux stdout reader");
        let stderr = stderr_reader.join().expect("join SSH tmux stderr reader");
        assert!(
            status.success(),
            "SSH nested-tmux suspend/resume failed with {status}: stdout={:?}, stderr={:?}",
            String::from_utf8_lossy(&output),
            String::from_utf8_lossy(&stderr),
        );
        assert!(contains(&output, RESTORED_MARKER));
        assert_ordered_pair(&output, ENTER_ALTERNATE_SCREEN, LEAVE_ALTERNATE_SCREEN);
        assert!(!tmux.session_exists(session));
    }
}

impl TmuxGuard {
    fn wait_until_shell_ready(&self, session: &str) -> PaneState {
        self.wait_until(session, "waiting for the nested shell prompt", |state| {
            !state.dead
                && state.command == "bash"
                && self
                    .captured_text(session)
                    .contains("YO_NESTED_TMUX_READY>")
        })
    }

    fn wait_until_suspended(&self, session: &str, yo_pid: Pid, baseline: &Termios) {
        self.wait_until(session, "waiting for suspended yo", |state| {
            !state.dead
                && state.command == "bash"
                && process_is_stopped(yo_pid)
                && read_termios(&state.tty).as_ref() == Some(baseline)
        });
    }

    fn wait_until_shell_after_exit(&self, session: &str, yo_pid: Pid, baseline: &Termios) {
        self.wait_until(session, "waiting for yo to exit", |state| {
            !state.dead
                && state.command == "bash"
                && !process_exists(yo_pid)
                && read_termios(&state.tty).as_ref() == Some(baseline)
        });
    }

    fn send_literal(&self, session: &str, value: &str) {
        let output = self.output(&["send-keys", "-l", "-t", session, value]);
        assert!(output.status.success(), "send literal keys inside SSH tmux");
    }

    fn send_key(&self, session: &str, key: &str) {
        let output = self.output(&["send-keys", "-t", session, key]);
        assert!(output.status.success(), "send key inside SSH tmux");
    }

    fn captured_text(&self, session: &str) -> String {
        let output = self.output(&["capture-pane", "-p", "-S", "-", "-t", session]);
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
}

// 격리된 SSH PTY 안의 tmux Inline에서 실제 키 경로로 두 번 Ctrl+Z와 `fg`를 반복해,
// 내부 pane의 stopped 상태·termios 복구·재진입과 바깥 SSH PTY 복구를 함께 확인한다.
#[test]
#[ignore = "requires local sshd, tmux, and a compatible installed Codex"]
fn ssh_tmux_inline_repeated_suspend_resume_restores_each_generation() {
    SshServer::start().run_tmux_repeated_suspend_resume("--inline", false);
}

// 격리된 SSH PTY 안의 tmux Fullscreen에서 두 번 중지·재개할 때마다 내부 alternate
// screen 세대를 다시 획득하고, 최종적으로 tmux와 바깥 SSH PTY를 모두 복구하는지 확인한다.
#[test]
#[ignore = "requires local sshd, tmux, and a compatible installed Codex"]
fn ssh_tmux_fullscreen_repeated_suspend_resume_restores_each_generation() {
    SshServer::start().run_tmux_repeated_suspend_resume("--fullscreen", true);
}
