use std::{
    io::Read,
    path::Path,
    process::{Command, Output, Stdio},
    thread,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use super::{
    ENTER_ALTERNATE_SCREEN, LEAVE_ALTERNATE_SCREEN, READY_TIMEOUT, RESTORED_MARKER,
    server::SshServer,
    session::{ChildGuard, assert_ordered_pair, wait_for_exit},
};
use crate::support::{
    contains, has_noncanonical_no_echo_input, repository_path, require_command, shell_quote,
};

#[path = "tmux/suspend.rs"]
mod suspend;

impl SshServer {
    pub(super) fn run_tmux_mode(&self, option: &str, alternate_screen: bool) {
        require_tmux();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos();
        let socket = self.fixture_path(&format!("tmux-{}-{unique}.sock", std::process::id()));
        let session = "yo";
        let tmux = TmuxGuard::new(socket.clone());

        let repository = repository_path();
        let yo = Path::new(env!("CARGO_BIN_EXE_yo"))
            .canonicalize()
            .expect("canonicalize yo binary");
        let inner_command = format!(
            "env PATH={codex_directory}:/usr/bin:/bin {yo} {option}",
            codex_directory =
                shell_quote(self.codex.parent().expect("Codex executable has a parent")),
            yo = shell_quote(&yo),
            option = shell_quote(Path::new(option)),
        );
        let remote = remote_tmux_command(&repository, &socket, session, &inner_command);

        let mut child = ChildGuard::new(
            self.client(true)
                .arg(remote)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("start SSH PTY containing tmux"),
        );
        let stdout_reader = capture(child.stdout.take().expect("capture SSH tmux stdout"));
        let stderr_reader = capture(child.stderr.take().expect("capture SSH tmux stderr"));

        tmux.wait_until_ready(session, alternate_screen);
        child.send_input(&[0x04]);

        let pane_exit = tmux.wait_until_dead(session);
        assert_eq!(
            pane_exit.status,
            Some(0),
            "yo inside tmux must exit successfully"
        );
        tmux.kill_session(session);
        let status = wait_for_exit(&mut child);
        drop(child.stdin.take());
        let output = stdout_reader.join().expect("join SSH tmux stdout reader");
        let stderr = stderr_reader.join().expect("join SSH tmux stderr reader");

        assert!(
            status.success(),
            "SSH tmux command failed with {status}: stdout={:?}, stderr={:?}",
            String::from_utf8_lossy(&output),
            String::from_utf8_lossy(&stderr),
        );
        assert!(contains(&output, RESTORED_MARKER));
        assert_ordered_pair(&output, ENTER_ALTERNATE_SCREEN, LEAVE_ALTERNATE_SCREEN);
        assert!(
            !tmux.session_exists(session),
            "remote tmux session remained after yo exited"
        );
    }
}

struct TmuxGuard {
    socket: std::path::PathBuf,
}

impl TmuxGuard {
    fn new(socket: std::path::PathBuf) -> Self {
        Self { socket }
    }

    fn wait_until_ready(&self, session: &str, alternate_screen: bool) -> PaneState {
        self.wait_until(session, "waiting for the TUI pane", |state| {
            !state.dead
                && state.alternate_screen == alternate_screen
                && state.command == "yo"
                && has_noncanonical_no_echo_input(&state.tty)
        })
    }

    fn wait_until_dead(&self, session: &str) -> PaneState {
        self.wait_until(session, "waiting for the TUI pane to exit", |state| {
            state.dead && state.status.is_some()
        })
    }

    fn wait_until(
        &self,
        session: &str,
        context: &'static str,
        predicate: impl Fn(&PaneState) -> bool,
    ) -> PaneState {
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            if let Some(state) = self.pane_state(session)
                && predicate(&state)
            {
                return state;
            }
            assert!(
                Instant::now() < deadline,
                "tmux inside SSH did not converge within {READY_TIMEOUT:?}: {context}"
            );
            thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn pane_state(&self, session: &str) -> Option<PaneState> {
        let output = self.output(&[
            "list-panes",
            "-t",
            session,
            "-F",
            "#{pane_dead}\t#{pane_dead_status}\t#{alternate_on}\t#{pane_current_command}\t#{pane_tty}\t#{pane_pid}",
        ]);
        if !output.status.success() {
            return None;
        }
        let line = String::from_utf8(output.stdout).expect("tmux pane state is UTF-8");
        let mut fields = line.trim_end().split('\t');
        let state = PaneState {
            dead: fields.next().expect("tmux pane dead field") == "1",
            status: fields
                .next()
                .filter(|value| !value.is_empty())
                .map(|value| value.parse().expect("tmux pane exit status is an integer")),
            alternate_screen: fields.next().expect("tmux alternate field") == "1",
            command: fields.next().expect("tmux command field").to_owned(),
            tty: fields.next().expect("tmux tty field").into(),
            shell_pid: fields
                .next()
                .expect("tmux pane pid field")
                .parse()
                .expect("tmux pane pid is an integer"),
        };
        assert!(
            fields.next().is_none(),
            "unexpected tmux pane state: {line:?}"
        );
        Some(state)
    }

    fn session_exists(&self, session: &str) -> bool {
        self.output(&["has-session", "-t", session])
            .status
            .success()
    }

    fn kill_session(&self, session: &str) {
        let output = self.output(&["kill-session", "-t", session]);
        assert!(
            output.status.success(),
            "kill tmux session after observing pane exit: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn output(&self, arguments: &[&str]) -> Output {
        Command::new("tmux")
            .arg("-S")
            .arg(&self.socket)
            .args(arguments)
            .output()
            .expect("inspect tmux inside SSH")
    }
}

impl Drop for TmuxGuard {
    fn drop(&mut self) {
        let _ = self.output(&["kill-server"]);
    }
}

struct PaneState {
    dead: bool,
    status: Option<i32>,
    alternate_screen: bool,
    command: String,
    tty: std::path::PathBuf,
    shell_pid: i32,
}

fn remote_tmux_command(
    repository: &Path,
    socket: &Path,
    session: &str,
    inner_command: &str,
) -> String {
    format!(
        "cd {repository} && stty rows 24 cols 80 && before=$(stty -g) && \
         tmux -f /dev/null -S {socket} start-server \\; \
         set-option -g remain-on-exit on \\; \
         new-session -x 80 -y 24 -s {session} \
         {inner_command}; \
         result_code=$?; after=$(stty -g); test \"$before\" = \"$after\" || exit 91; \
         printf '\\n{restored}\\n'; exit $result_code",
        repository = shell_quote(repository),
        socket = shell_quote(socket),
        session = shell_quote(Path::new(session)),
        restored = String::from_utf8_lossy(RESTORED_MARKER),
    )
}

fn capture(mut reader: impl Read + Send + 'static) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .expect("read SSH tmux stream to completion");
        bytes
    })
}

fn require_tmux() {
    require_command("tmux", &["-V"]);
}
