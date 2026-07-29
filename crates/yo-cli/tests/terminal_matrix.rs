#![cfg(target_os = "linux")]

use std::{
    process::{Command, Output},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

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

    fn run_fullscreen(&self) {
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
            "--fullscreen",
        ]);
        self.wait_until(COMMAND_READY_TIMEOUT, |state| {
            !state.dead
                && state.alternate_screen
                && state.command == "yo"
                && state.cursor_y.checked_add(1) == Some(state.height)
        });
    }

    fn send_empty_ctrl_d(&self) {
        run_tmux(&["send-keys", "-t", &self.name, "C-d"]);
    }

    fn wait_for_clean_exit(&self) -> PaneState {
        self.wait_until(CLEAN_EXIT_TIMEOUT, |state| state.dead)
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

// 실제 Linux tmux 안에서 Fullscreen을 실행하고 빈 입력 Ctrl+D를 보내면 앱이 상태 0으로
// 끝나는지 확인한다. 정상 경로에서는 격리 세션을 명시적으로 제거한 뒤 실제 부재까지
// 검증하며, 중간 실패 시에는 Drop이 best-effort 정리를 시도해 다음 테스트의 오염을 줄인다.
#[test]
#[ignore = "requires local tmux and a compatible installed Codex"]
fn local_tmux_fullscreen_exits_cleanly_from_empty_ctrl_d() {
    let session = TmuxSession::create();
    session.run_fullscreen();
    session.send_empty_ctrl_d();

    let exit = session.wait_for_clean_exit();

    assert_eq!(exit.status, Some(0));
    let session_name = session.name.clone();
    drop(session);
    assert_tmux_session_absent(&session_name);
}
