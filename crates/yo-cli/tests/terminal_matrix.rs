#![cfg(target_os = "linux")]

use std::{
    fs::OpenOptions,
    process::{Command, Output},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use nix::sys::termios::{LocalFlags, tcgetattr};

const COMMAND_READY_TIMEOUT: Duration = Duration::from_secs(5);
const CLEAN_EXIT_TIMEOUT: Duration = Duration::from_secs(5);

struct TmuxSession {
    name: String,
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
        run_tmux(&["new-session", "-d", "-s", &name, "-x", "80", "-y", "24"]);
        let session = Self { name };
        run_tmux(&["set-option", "-t", &session.name, "remain-on-exit", "on"]);
        session
    }

    fn run_mode(&self, option: &str, alternate_screen: bool) {
        let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .unwrap()
            .canonicalize()
            .unwrap();
        let repository = repository
            .to_str()
            .expect("the repository path must be valid UTF-8");
        run_tmux(&[
            "respawn-pane",
            "-k",
            "-t",
            &self.name,
            "-c",
            repository,
            env!("CARGO_BIN_EXE_yo"),
            option,
        ]);
        self.wait_until(COMMAND_READY_TIMEOUT, |state| {
            !state.dead
                && state.alternate_screen == alternate_screen
                && state.command == "yo"
                && state.cursor_y.checked_add(1) == Some(state.height)
                && self.has_noncanonical_no_echo_input()
        });
    }

    fn send_empty_ctrl_d(&self) {
        run_tmux(&["send-keys", "-t", &self.name, "C-d"]);
    }

    fn wait_for_clean_exit(&self) -> PaneState {
        self.wait_until(CLEAN_EXIT_TIMEOUT, |state| state.dead)
    }

    fn has_noncanonical_no_echo_input(&self) -> bool {
        let tty_path = tmux_output(&["display-message", "-p", "-t", &self.name, "#{pane_tty}"]);
        let Ok(tty_path) = String::from_utf8(tty_path.stdout) else {
            return false;
        };
        let Ok(tty) = OpenOptions::new()
            .read(true)
            .write(true)
            .open(tty_path.trim_end())
        else {
            return false;
        };
        let Ok(termios) = tcgetattr(&tty) else {
            return false;
        };
        !termios
            .local_flags
            .intersects(LocalFlags::ICANON | LocalFlags::ECHO)
    }

    fn wait_until(&self, timeout: Duration, predicate: impl Fn(&PaneState) -> bool) -> PaneState {
        let deadline = Instant::now() + timeout;
        loop {
            let state = self.pane_state();
            if predicate(&state) {
                return state;
            }
            assert!(
                Instant::now() < deadline,
                "tmux pane did not reach the expected state within {timeout:?}: {state:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn pane_state(&self) -> PaneState {
        let output = tmux_output(&[
            "list-panes",
            "-t",
            &self.name,
            "-F",
            "#{pane_dead}\t#{pane_dead_status}\t#{alternate_on}\t#{pane_current_command}\t#{cursor_y}\t#{pane_height}",
        ]);
        let line = String::from_utf8(output.stdout).unwrap();
        let mut fields = line.trim_end().split('\t');
        let dead = fields.next().unwrap() == "1";
        let status = fields
            .next()
            .filter(|value| !value.is_empty())
            .map(|value| value.parse::<i32>().unwrap());
        let alternate_screen = fields.next().unwrap() == "1";
        let command = fields.next().unwrap_or_default().to_owned();
        let cursor_y = fields.next().unwrap().parse::<u16>().unwrap();
        let height = fields.next().unwrap().parse::<u16>().unwrap();
        assert!(
            fields.next().is_none(),
            "unexpected tmux pane state: {line:?}"
        );
        PaneState {
            dead,
            status,
            alternate_screen,
            command,
            cursor_y,
            height,
        }
    }
}

impl Drop for TmuxSession {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["kill-session", "-t", &self.name])
            .output();
    }
}

#[derive(Debug)]
struct PaneState {
    dead: bool,
    status: Option<i32>,
    alternate_screen: bool,
    command: String,
    cursor_y: u16,
    height: u16,
}

fn require_command(command: &str, arguments: &[&str]) {
    let output = Command::new(command)
        .args(arguments)
        .output()
        .unwrap_or_else(|error| {
            panic!("required environment command `{command}` is unavailable: {error}")
        });
    assert!(
        output.status.success(),
        "required environment command `{command}` failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_tmux(arguments: &[&str]) {
    let _ = tmux_output(arguments);
}

fn tmux_output(arguments: &[&str]) -> Output {
    let output = Command::new("tmux").args(arguments).output().unwrap();
    assert!(
        output.status.success(),
        "tmux {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn assert_tmux_session_absent(name: &str) {
    let output = Command::new("tmux")
        .args(["has-session", "-t", name])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "isolated tmux session `{name}` remained after cleanup"
    );
}

fn assert_empty_ctrl_d_exits_cleanly(option: &str, alternate_screen: bool) {
    let session = TmuxSession::create();
    session.run_mode(option, alternate_screen);
    session.send_empty_ctrl_d();

    let exit = session.wait_for_clean_exit();

    assert_eq!(exit.status, Some(0));
    let session_name = session.name.clone();
    drop(session);
    assert_tmux_session_absent(&session_name);
}

// 실제 Linux tmux의 main screen에서 Inline이 noncanonical·no-echo 입력 상태에 들어간 뒤
// 빈 입력 Ctrl+D를 보내면 상태 0으로 끝나고, 격리된 tmux 세션까지 제거하는지 확인한다.
#[test]
#[ignore = "requires local tmux and a compatible installed Codex"]
fn local_tmux_inline_exits_cleanly_from_empty_ctrl_d() {
    assert_empty_ctrl_d_exits_cleanly("--inline", false);
}

// 실제 Linux tmux에서 noncanonical·no-echo 입력과 alternate screen을 획득한
// Fullscreen에 빈 입력 Ctrl+D를 보내면 상태 0으로 끝나고, 격리 세션까지 제거하는지
// 확인한다. 중간 실패 시에는 Drop이 best-effort 정리를 시도해 다음 테스트의 오염을 줄인다.
#[test]
#[ignore = "requires local tmux and a compatible installed Codex"]
fn local_tmux_fullscreen_exits_cleanly_from_empty_ctrl_d() {
    assert_empty_ctrl_d_exits_cleanly("--fullscreen", true);
}
