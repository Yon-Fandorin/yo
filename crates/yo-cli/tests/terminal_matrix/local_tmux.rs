use std::{
    process::{Command, Output},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use nix::unistd::Pid;

use crate::support::{
    has_noncanonical_no_echo_input, only_child, process_exists, process_is_stopped, read_termios,
    repository_path, require_command, shell_quote,
};

const COMMAND_READY_TIMEOUT: Duration = Duration::from_secs(5);
const CLEAN_EXIT_TIMEOUT: Duration = Duration::from_secs(5);
const EXIT_MARKER: &str = "YO_TMUX_EXIT";
const SHELL_READY: &str = "YO_TMUX_READY>";

#[path = "suspend.rs"]
mod suspend;

struct TmuxSession {
    name: String,
    socket: std::path::PathBuf,
    session_repository: std::path::PathBuf,
}

struct ShellJob {
    baseline: nix::sys::termios::Termios,
    yo_pid: Pid,
}

impl TmuxSession {
    fn create() -> Self {
        require_command("tmux", &["-V"]);
        require_command("codex", &["--version"]);

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let name = format!("yo-matrix-{}-{unique}", std::process::id());
        let socket = std::env::temp_dir().join(format!("{name}.sock"));
        let session_repository = std::env::temp_dir().join(format!("{name}-sessions"));
        run_tmux(
            &socket,
            &["new-session", "-d", "-s", &name, "-x", "80", "-y", "24"],
        );
        let session = Self {
            name,
            socket,
            session_repository,
        };
        session.run_tmux(&["set-option", "-t", &session.name, "remain-on-exit", "on"]);
        session
    }

    fn run_mode(&self, option: &str, alternate_screen: bool) {
        let repository = repository_path();
        let repository = repository
            .to_str()
            .expect("the repository path must be valid UTF-8");
        self.run_tmux(&[
            "respawn-pane",
            "-k",
            "-t",
            &self.name,
            "-c",
            repository,
            "/usr/bin/env",
            &format!(
                "YO_SESSION_REPOSITORY={}",
                self.session_repository.display()
            ),
            env!("CARGO_BIN_EXE_yo"),
            option,
        ]);
        self.wait_until(COMMAND_READY_TIMEOUT, |state| {
            !state.dead
                && state.alternate_screen == alternate_screen
                && state.command == "yo"
                && self.has_noncanonical_no_echo_input()
        });
    }

    fn run_mode_under_shell(&self, option: &str, alternate_screen: bool) -> ShellJob {
        let repository = repository_path();
        self.run_tmux(&[
            "respawn-pane",
            "-k",
            "-t",
            &self.name,
            "-c",
            repository.to_str().expect("repository path is valid UTF-8"),
            "/usr/bin/env",
            &format!("PS1={SHELL_READY}"),
            "HISTFILE=/dev/null",
            "/bin/bash",
            "--noprofile",
            "--norc",
            "-i",
        ]);
        self.wait_until(COMMAND_READY_TIMEOUT, |state| {
            !state.dead && state.command == "bash" && self.captured_text().contains(SHELL_READY)
        });
        // Readline may settle its prompt termios just after the prompt becomes visible.
        thread::sleep(Duration::from_secs(1));
        let baseline = self.termios().expect("read shell terminal state");
        let yo = shell_quote(std::path::Path::new(env!("CARGO_BIN_EXE_yo")));
        let session_repository = shell_quote(&self.session_repository);
        let command = format!("YO_SESSION_REPOSITORY={session_repository} {yo} {option}");
        self.send_literal(&command);
        self.send_enter();
        self.wait_for_mode(alternate_screen);
        ShellJob {
            baseline,
            yo_pid: self.shell_child().expect("read the only yo shell child"),
        }
    }

    fn wait_for_mode(&self, alternate_screen: bool) {
        self.wait_until(COMMAND_READY_TIMEOUT, |state| {
            !state.dead
                && state.alternate_screen == alternate_screen
                && state.command == "yo"
                && self.has_noncanonical_no_echo_input()
        });
    }

    fn send_ctrl_z(&self) {
        self.run_tmux(&["send-keys", "-t", &self.name, "C-z"]);
    }

    fn send_empty_ctrl_d(&self) {
        self.run_tmux(&["send-keys", "-t", &self.name, "C-d"]);
    }

    fn send_literal(&self, value: &str) {
        self.run_tmux(&["send-keys", "-l", "-t", &self.name, value]);
    }

    fn send_enter(&self) {
        self.run_tmux(&["send-keys", "-t", &self.name, "Enter"]);
    }

    fn wait_for_suspension_then_foreground(&self, job: &ShellJob) {
        self.wait_until(COMMAND_READY_TIMEOUT, |state| {
            !state.dead
                && state.command == "bash"
                && process_is_stopped(job.yo_pid)
                && self.termios().as_ref() == Some(&job.baseline)
        });
        thread::sleep(Duration::from_millis(100));
        self.send_literal("fg");
        self.send_enter();
    }

    fn wait_for_yo_exit_then_exit_shell(&self, job: &ShellJob) {
        self.wait_until(COMMAND_READY_TIMEOUT, |state| {
            !state.dead
                && state.command == "bash"
                && !process_exists(job.yo_pid)
                && self.termios().as_ref() == Some(&job.baseline)
        });
        thread::sleep(Duration::from_millis(100));
        self.send_literal(&format!(
            "result=$?; printf '\\n{EXIT_MARKER}:%s\\n' \"$result\"; exit \"$result\""
        ));
        self.send_enter();
    }

    fn wait_for_clean_exit(&self) -> PaneState {
        self.wait_until(CLEAN_EXIT_TIMEOUT, |state| {
            state.dead && state.status.is_some()
        })
    }

    fn has_noncanonical_no_echo_input(&self) -> bool {
        self.tty_path()
            .as_deref()
            .is_some_and(has_noncanonical_no_echo_input)
    }

    fn termios(&self) -> Option<nix::sys::termios::Termios> {
        read_termios(&self.tty_path()?)
    }

    fn tty_path(&self) -> Option<std::path::PathBuf> {
        let output = self.tmux_output(&["display-message", "-p", "-t", &self.name, "#{pane_tty}"]);
        let path = String::from_utf8(output.stdout).ok()?;
        Some(path.trim_end().into())
    }

    fn shell_child(&self) -> Option<Pid> {
        let output = self.tmux_output(&["display-message", "-p", "-t", &self.name, "#{pane_pid}"]);
        let shell = String::from_utf8(output.stdout)
            .ok()?
            .trim()
            .parse::<i32>()
            .ok()?;
        only_child(Pid::from_raw(shell))
    }

    fn captured_text(&self) -> String {
        let output = self.tmux_output(&["capture-pane", "-p", "-S", "-", "-t", &self.name]);
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn wait_until(&self, timeout: Duration, predicate: impl Fn(&PaneState) -> bool) -> PaneState {
        let deadline = Instant::now() + timeout;
        loop {
            let state = self.pane_state();
            if predicate(&state) {
                return state;
            }
            if Instant::now() >= deadline {
                if state.command == "bash" {
                    self.send_literal(
                        "printf '\\nYO_TMUX_DEBUG_STATUS:%s\\n' \"$?\"; jobs -l; history 5",
                    );
                    self.send_enter();
                    thread::sleep(Duration::from_millis(50));
                }
                panic!(
                    "tmux pane did not reach the expected state within {timeout:?}: {state:?}; \
                     termios={:?}; pane={:?}",
                    self.termios(),
                    self.captured_text()
                );
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn pane_state(&self) -> PaneState {
        let output = self.tmux_output(&[
            "list-panes",
            "-t",
            &self.name,
            "-F",
            "#{pane_dead}|#{pane_dead_status}|#{alternate_on}|#{pane_current_command}",
        ]);
        let line = String::from_utf8(output.stdout).unwrap();
        PaneState::parse(&line)
    }

    fn run_tmux(&self, arguments: &[&str]) {
        let _ = self.tmux_output(arguments);
    }

    fn tmux_output(&self, arguments: &[&str]) -> Output {
        tmux_output(&self.socket, arguments)
    }
}

impl Drop for TmuxSession {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .arg("-S")
            .arg(&self.socket)
            .args(["kill-session", "-t", &self.name])
            .output();
        let _ = Command::new("tmux")
            .arg("-S")
            .arg(&self.socket)
            .arg("kill-server")
            .output();
        let server_is_absent = Command::new("tmux")
            .arg("-S")
            .arg(&self.socket)
            .arg("list-sessions")
            .output()
            .is_ok_and(|output| !output.status.success());
        if server_is_absent {
            let _ = std::fs::remove_file(&self.socket);
        }
        let _ = std::fs::remove_dir_all(&self.session_repository);
    }
}

#[derive(Debug)]
struct PaneState {
    dead: bool,
    status: Option<i32>,
    alternate_screen: bool,
    command: String,
}

impl PaneState {
    fn parse(line: &str) -> Self {
        // macOS tmux normalizes control-character format separators to `_`.
        // A printable delimiter keeps empty status and command fields distinct
        // on both supported hosts.
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            return Self {
                dead: false,
                status: None,
                alternate_screen: false,
                command: String::new(),
            };
        }
        let mut fields = line.split('|');
        let dead = fields.next().expect("pane_dead field") == "1";
        let status = fields
            .next()
            .expect("pane_dead_status field")
            .parse::<i32>()
            .ok();
        let alternate_screen = fields.next().expect("alternate_on field") == "1";
        let command = fields
            .next()
            .expect("pane_current_command field")
            .to_owned();
        assert!(
            fields.next().is_none(),
            "unexpected tmux pane state: {line:?}"
        );
        Self {
            dead,
            status,
            alternate_screen,
            command,
        }
    }
}

fn run_tmux(socket: &std::path::Path, arguments: &[&str]) {
    let _ = tmux_output(socket, arguments);
}

fn tmux_output(socket: &std::path::Path, arguments: &[&str]) -> Output {
    let output = Command::new("tmux")
        .args(["-f", "/dev/null"])
        .arg("-S")
        .arg(socket)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "tmux {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn assert_tmux_server_absent(socket: &std::path::Path, name: &str) {
    let output = Command::new("tmux")
        .arg("-S")
        .arg(socket)
        .args(["has-session", "-t", name])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "isolated tmux session `{name}` remained after cleanup"
    );
    assert!(
        !socket.exists(),
        "isolated tmux server socket remained after cleanup: {socket:?}"
    );
}

fn assert_empty_ctrl_d_exits_cleanly(option: &str, alternate_screen: bool) {
    let session = TmuxSession::create();
    session.run_mode(option, alternate_screen);
    session.send_empty_ctrl_d();

    let exit = session.wait_for_clean_exit();

    assert_eq!(exit.status, Some(0));
    let session_name = session.name.clone();
    let socket = session.socket.clone();
    let session_repository = session.session_repository.clone();
    drop(session);
    assert_tmux_server_absent(&socket, &session_name);
    assert!(
        !session_repository.exists(),
        "isolated Session repository remained after cleanup: {session_repository:?}"
    );
}

// macOS tmux에서 살아 있는 pane의 빈 dead-status와 command 필드도 printable 구분자로
// 보존해, 정상적인 빈 값을 필드 누락으로 오인하지 않고 준비 전 상태로 해석한다.
#[test]
fn pane_state_preserves_empty_trailing_tmux_fields() {
    let state = PaneState::parse("0||0|\n");

    assert!(!state.dead);
    assert_eq!(state.status, None);
    assert!(!state.alternate_screen);
    assert!(state.command.is_empty());
}

// macOS tmux가 respawn 직후 pane 행을 아직 반환하지 않아도 준비 전 상태로 해석해
// 대기 루프가 다음 관찰을 시도하고, 일시적인 빈 출력 때문에 panic하지 않는다.
#[test]
fn pane_state_treats_an_empty_tmux_observation_as_not_ready() {
    let state = PaneState::parse("\n");

    assert!(!state.dead);
    assert_eq!(state.status, None);
    assert!(!state.alternate_screen);
    assert!(state.command.is_empty());
}

// 실제 Unix tmux의 main screen에서 Inline이 noncanonical·no-echo 입력 상태에 들어간 뒤
// 빈 입력 Ctrl+D를 보내면 상태 0으로 끝나고, 격리된 tmux 세션까지 제거하는지 확인한다.
#[test]
#[ignore = "requires local tmux and a compatible installed Codex"]
fn local_tmux_inline_exits_cleanly_from_empty_ctrl_d() {
    assert_empty_ctrl_d_exits_cleanly("--inline", false);
}

// 실제 Unix tmux에서 noncanonical·no-echo 입력과 alternate screen을 획득한
// Fullscreen에 빈 입력 Ctrl+D를 보내면 상태 0으로 끝나고, 격리 세션까지 제거하는지
// 확인한다. 중간 실패 시에는 Drop이 best-effort 정리를 시도해 다음 테스트의 오염을 줄인다.
#[test]
#[ignore = "requires local tmux and a compatible installed Codex"]
fn local_tmux_fullscreen_exits_cleanly_from_empty_ctrl_d() {
    assert_empty_ctrl_d_exits_cleanly("--fullscreen", true);
}
