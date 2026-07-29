use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
    thread,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use nix::sys::termios::{LocalFlags, tcgetattr};

use super::{
    ENTER_ALTERNATE_SCREEN, READY_TIMEOUT, RESTORED_MARKER,
    server::SshServer,
    session::{ChildGuard, assert_ordered_pair, shell_quote, wait_for_exit},
};

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

        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("crate is under <repository>/crates/yo-cli")
            .canonicalize()
            .expect("canonicalize repository");
        let yo = Path::new(env!("CARGO_BIN_EXE_yo"))
            .canonicalize()
            .expect("canonicalize yo binary");
        let remote = remote_command(&repository, &yo, &self.codex, &socket, session, option);

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
        let input = child.stdin.as_mut().expect("SSH tmux stdin remains open");
        input.write_all(&[0x04]).expect("send empty Ctrl+D");
        input.flush().expect("flush SSH tmux input");

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
        assert_ordered_pair(
            &output,
            ENTER_ALTERNATE_SCREEN,
            super::LEAVE_ALTERNATE_SCREEN,
        );
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

    fn wait_until_ready(&self, session: &str, alternate_screen: bool) {
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            if let Some(state) = self.pane_state(session)
                && !state.dead
                && state.alternate_screen == alternate_screen
                && state.command == "yo"
                && has_noncanonical_no_echo_input(&state.tty)
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "tmux inside SSH did not reach the expected pane state within {READY_TIMEOUT:?}"
            );
            thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn wait_until_dead(&self, session: &str) -> PaneState {
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            if let Some(state) = self.pane_state(session)
                && state.dead
            {
                return state;
            }
            assert!(
                Instant::now() < deadline,
                "yo inside tmux did not exit within {READY_TIMEOUT:?}"
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
            "#{pane_dead}\t#{pane_dead_status}\t#{alternate_on}\t#{pane_current_command}\t#{pane_tty}",
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
}

fn has_noncanonical_no_echo_input(tty_path: &Path) -> bool {
    let Ok(tty) = OpenOptions::new().read(true).write(true).open(tty_path) else {
        return false;
    };
    let Ok(termios) = tcgetattr(&tty) else {
        return false;
    };
    !termios
        .local_flags
        .intersects(LocalFlags::ICANON | LocalFlags::ECHO)
}

fn remote_command(
    repository: &Path,
    yo: &Path,
    codex: &Path,
    socket: &Path,
    session: &str,
    option: &str,
) -> String {
    let codex_directory = codex.parent().expect("Codex executable has a parent");
    format!(
        "cd {repository} && stty rows 24 cols 80 && before=$(stty -g) && \
         tmux -f /dev/null -S {socket} start-server \\; \
         set-option -g remain-on-exit on \\; \
         new-session -x 80 -y 24 -s {session} \
         env PATH={codex_directory}:/usr/bin:/bin {yo} {option}; \
         result_code=$?; after=$(stty -g); test \"$before\" = \"$after\" || exit 91; \
         printf '\\n{restored}\\n'; exit $result_code",
        repository = shell_quote(repository),
        socket = shell_quote(socket),
        session = shell_quote(Path::new(session)),
        codex_directory = shell_quote(codex_directory),
        yo = shell_quote(yo),
        option = shell_quote(Path::new(option)),
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

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|candidate| candidate == needle)
}

fn require_tmux() {
    let output = Command::new("tmux")
        .arg("-V")
        .output()
        .expect("required command `tmux` is unavailable");
    assert!(
        output.status.success(),
        "required command `tmux` failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
