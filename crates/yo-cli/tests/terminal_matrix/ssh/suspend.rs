use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use nix::{sys::termios::Termios, unistd::Pid};

use super::{
    ENTER_ALTERNATE_SCREEN, EXIT_TIMEOUT, HIDE_CURSOR, LEAVE_ALTERNATE_SCREEN, READY_TIMEOUT,
    SHOW_CURSOR,
    server::SshServer,
    session::{ChildGuard, wait_for_exit},
};
use crate::support::{
    count, has_noncanonical_no_echo_input, last_position, only_child, position, process_exists,
    process_is_stopped, read_termios, repository_path, shell_quote,
};

const SHELL_MARKER: &[u8] = b"YO_SSH_SHELL:";

impl SshServer {
    pub(super) fn run_repeated_suspend_resume(&self, option: &str, alternate_screen: bool) {
        let repository = repository_path();
        let yo_path = Path::new(env!("CARGO_BIN_EXE_yo"))
            .canonicalize()
            .expect("canonicalize yo binary");
        let remote = interactive_shell_command(&repository, &self.codex);
        let mut child = ChildGuard::new(
            self.client(true)
                .arg(remote)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("start interactive SSH PTY"),
        );
        let output = CapturedOutput::new(child.stdout.take().expect("capture SSH stdout"));
        let stderr = CapturedOutput::new(child.stderr.take().expect("capture SSH stderr"));

        let shell = wait_for_shell(&output);
        let tty_path = PathBuf::from(format!("/proc/{}/fd/0", shell.as_raw()));
        let baseline = wait_for_termios(&tty_path);
        thread::sleep(Duration::from_millis(100));
        child.send_input(
            format!("before=$(stty -g); {} {}\n", shell_quote(&yo_path), option).as_bytes(),
        );
        wait_for_generation(&output, &tty_path, alternate_screen, 0);
        let yo_pid = wait_for_shell_child(shell);

        for _ in 0..2 {
            child.send_input(&[0x1a]);
            wait_for_suspension(yo_pid, &tty_path, &baseline);
            let before = output.marker_count(generation_marker(alternate_screen));
            thread::sleep(Duration::from_millis(100));
            child.send_input(b"fg\n");
            wait_for_generation(&output, &tty_path, alternate_screen, before);
        }

        child.send_input(&[0x04]);
        wait_for_process_exit(yo_pid);
        wait_for_exact_termios(&tty_path, &baseline);
        thread::sleep(Duration::from_millis(100));
        child.send_input(
            b"result=$?; after=$(stty -g); test \"$before\" = \"$after\" || exit 91; \
              exit \"$result\"\n",
        );

        let status = wait_for_exit(&mut child);
        drop(child.stdin.take());
        let output = output.finish();
        let stderr = stderr.finish();

        assert!(
            status.success(),
            "SSH suspend/resume command failed with {status}: stdout={:?}, stderr={:?}",
            String::from_utf8_lossy(&output),
            String::from_utf8_lossy(&stderr),
        );
        if alternate_screen {
            // Initial launch and each of the two `fg` operations start a new screen generation.
            assert_eq!(count(&output, ENTER_ALTERNATE_SCREEN), 3);
            assert_eq!(count(&output, LEAVE_ALTERNATE_SCREEN), 3);
        } else {
            assert_eq!(count(&output, ENTER_ALTERNATE_SCREEN), 0);
            assert_eq!(count(&output, LEAVE_ALTERNATE_SCREEN), 0);
            assert!(
                last_position(&output, HIDE_CURSOR).expect("cursor hiding is observable")
                    < last_position(&output, SHOW_CURSOR)
                        .expect("cursor restoration is observable"),
                "the final Inline cursor operation must restore visibility"
            );
        }
    }
}

struct CapturedOutput {
    bytes: Arc<Mutex<Vec<u8>>>,
    reader: thread::JoinHandle<()>,
}

impl CapturedOutput {
    fn new(mut reader: impl Read + Send + 'static) -> Self {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&bytes);
        let reader = thread::spawn(move || {
            let mut chunk = [0; 4096];
            loop {
                let length = reader.read(&mut chunk).expect("read SSH PTY stream");
                if length == 0 {
                    return;
                }
                captured
                    .lock()
                    .expect("lock SSH PTY output")
                    .extend_from_slice(&chunk[..length]);
            }
        });
        Self { bytes, reader }
    }

    fn snapshot(&self) -> Vec<u8> {
        self.bytes.lock().expect("lock SSH PTY output").clone()
    }

    fn marker_count(&self, marker: &[u8]) -> usize {
        count(&self.snapshot(), marker)
    }

    fn finish(self) -> Vec<u8> {
        self.reader.join().expect("join SSH PTY reader");
        Arc::try_unwrap(self.bytes)
            .expect("SSH PTY output has no remaining owners")
            .into_inner()
            .expect("unlock SSH PTY output")
    }
}

fn interactive_shell_command(repository: &Path, codex: &Path) -> String {
    let codex_directory = codex.parent().expect("Codex executable has a parent");
    format!(
        "cd {repository} && stty rows 24 cols 80 && \
         exec env PATH={codex_directory}:/usr/bin:/bin \
         PS1='YO_SSH_SHELL:$$>' HISTFILE=/dev/null /bin/bash --noprofile --norc -i",
        repository = shell_quote(repository),
        codex_directory = shell_quote(codex_directory),
    )
}

fn wait_for_shell(output: &CapturedOutput) -> Pid {
    wait_until(READY_TIMEOUT, "waiting for the remote shell prompt", || {
        parse_shell_pid(&output.snapshot())
    })
}

fn parse_shell_pid(output: &[u8]) -> Option<Pid> {
    let start = position(output, SHELL_MARKER)? + SHELL_MARKER.len();
    let digits = output[start..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .copied()
        .collect::<Vec<_>>();
    let pid = std::str::from_utf8(&digits).ok()?.parse::<i32>().ok()?;
    Some(Pid::from_raw(pid))
}

fn wait_for_shell_child(shell: Pid) -> Pid {
    wait_until(READY_TIMEOUT, "waiting for the only yo shell child", || {
        only_child(shell)
    })
}

fn wait_for_generation(
    output: &CapturedOutput,
    tty_path: &Path,
    alternate_screen: bool,
    previous_markers: usize,
) {
    let marker = generation_marker(alternate_screen);
    wait_until(READY_TIMEOUT, "waiting for a fresh TUI generation", || {
        (has_noncanonical_no_echo_input(tty_path) && output.marker_count(marker) > previous_markers)
            .then_some(())
    });
}

fn generation_marker(alternate_screen: bool) -> &'static [u8] {
    if alternate_screen {
        ENTER_ALTERNATE_SCREEN
    } else {
        HIDE_CURSOR
    }
}

fn wait_for_suspension(pid: Pid, tty_path: &Path, baseline: &Termios) {
    wait_until(
        READY_TIMEOUT,
        "waiting for yo to stop and restore termios",
        || {
            (process_is_stopped(pid) && read_termios(tty_path).as_ref() == Some(baseline))
                .then_some(())
        },
    );
}

fn wait_for_process_exit(pid: Pid) {
    wait_until(EXIT_TIMEOUT, "waiting for yo to exit", || {
        (!process_exists(pid)).then_some(())
    });
}

fn wait_for_termios(path: &Path) -> Termios {
    wait_until(READY_TIMEOUT, "reading the remote shell termios", || {
        read_termios(path)
    })
}

fn wait_for_exact_termios(path: &Path, expected: &Termios) {
    wait_until(
        READY_TIMEOUT,
        "waiting for exact termios restoration",
        || (read_termios(path).as_ref() == Some(expected)).then_some(()),
    );
}

fn wait_until<T>(
    timeout: Duration,
    context: &'static str,
    mut observe: impl FnMut() -> Option<T>,
) -> T {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = observe() {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "SSH job-control state did not converge within {timeout:?}: {context}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

// 격리된 localhost SSH PTY의 Inline을 두 번 Ctrl+Z로 중지할 때마다 원격 셸의
// 실제 termios로 돌아오고, `fg` 뒤에는 새 terminal generation으로 재진입하는지 확인한다.
#[test]
#[ignore = "requires local sshd and a compatible installed Codex"]
fn ssh_inline_repeated_suspend_resume_restores_each_generation() {
    SshServer::start().run_repeated_suspend_resume("--inline", false);
}

// 격리된 localhost SSH PTY의 Fullscreen을 두 번 Ctrl+Z로 중지할 때마다 화면과
// termios를 반납하고, `fg` 뒤에는 alternate screen을 다시 획득하는지 확인한다.
#[test]
#[ignore = "requires local sshd and a compatible installed Codex"]
fn ssh_fullscreen_repeated_suspend_resume_restores_each_generation() {
    SshServer::start().run_repeated_suspend_resume("--fullscreen", true);
}
