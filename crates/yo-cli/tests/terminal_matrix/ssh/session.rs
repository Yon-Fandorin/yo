use std::{
    io::{Read, Write},
    ops::{Deref, DerefMut},
    path::Path,
    process::{Child, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Instant,
};

use super::{
    ENTER_ALTERNATE_SCREEN, EXIT_TIMEOUT, HIDE_CURSOR, LEAVE_ALTERNATE_SCREEN, READY_TIMEOUT,
    RESTORED_MARKER, SHOW_CURSOR, server::SshServer,
};

impl SshServer {
    pub(super) fn run_mode(&self, option: &str, alternate_screen: bool) {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("crate is under <repository>/crates/yo-cli")
            .canonicalize()
            .expect("canonicalize repository");
        let yo = Path::new(env!("CARGO_BIN_EXE_yo"))
            .canonicalize()
            .expect("canonicalize yo binary");
        let remote = remote_command(&repository, &yo, &self.codex, option);
        let mut child = ChildGuard::new(
            self.client(true)
                .arg(remote)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("start SSH PTY client"),
        );

        let output = Arc::new(Mutex::new(Vec::new()));
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let stdout = child.stdout.take().expect("capture SSH stdout");
        let stdout_reader = capture(
            stdout,
            Arc::clone(&output),
            ready_tx,
            if alternate_screen {
                ENTER_ALTERNATE_SCREEN
            } else {
                HIDE_CURSOR
            },
        );
        let stderr = child.stderr.take().expect("capture SSH stderr");
        let stderr_reader = thread::spawn(move || {
            let mut stderr = stderr;
            let mut bytes = Vec::new();
            stderr
                .read_to_end(&mut bytes)
                .expect("read SSH stderr to completion");
            bytes
        });

        ready_rx
            .recv_timeout(READY_TIMEOUT)
            .unwrap_or_else(|error| {
                panic!(
                    "SSH PTY did not render the requested mode: {error}; output={:?}",
                    String::from_utf8_lossy(&output.lock().expect("lock SSH output"))
                )
            });
        let input = child.stdin.as_mut().expect("SSH stdin remains open");
        input.write_all(&[0x04]).expect("send empty Ctrl+D");
        input.flush().expect("flush SSH input");

        let status = wait_for_exit(&mut child);
        drop(child.stdin.take());
        stdout_reader.join().expect("join SSH stdout reader");
        let stderr = stderr_reader.join().expect("join SSH stderr reader");
        let output = output.lock().expect("lock final SSH output");

        assert!(
            status.success(),
            "SSH PTY command failed with {status}: stdout={:?}, stderr={:?}",
            String::from_utf8_lossy(&output),
            String::from_utf8_lossy(&stderr),
        );
        assert!(contains(&output, RESTORED_MARKER));
        if alternate_screen {
            assert_ordered_pair(&output, ENTER_ALTERNATE_SCREEN, LEAVE_ALTERNATE_SCREEN);
        } else {
            let final_hide =
                last_position(&output, HIDE_CURSOR).expect("cursor hiding must be observable");
            let final_show =
                last_position(&output, SHOW_CURSOR).expect("cursor restoration must be observable");
            assert!(
                final_hide < final_show,
                "the final cursor visibility operation must restore a visible cursor"
            );
            assert!(!contains(&output, ENTER_ALTERNATE_SCREEN));
            assert!(!contains(&output, LEAVE_ALTERNATE_SCREEN));
        }
    }
}

pub(super) struct ChildGuard {
    child: Child,
}

impl ChildGuard {
    pub(super) fn new(child: Child) -> Self {
        Self { child }
    }
}

impl Deref for ChildGuard {
    type Target = Child;

    fn deref(&self) -> &Self::Target {
        &self.child
    }
}

impl DerefMut for ChildGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.child
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn capture(
    mut reader: impl Read + Send + 'static,
    output: Arc<Mutex<Vec<u8>>>,
    ready: mpsc::SyncSender<()>,
    marker: &'static [u8],
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut announced = false;
        let mut chunk = [0; 4096];
        loop {
            let length = reader.read(&mut chunk).expect("read SSH PTY output");
            if length == 0 {
                return;
            }
            let mut output = output.lock().expect("lock SSH PTY output");
            output.extend_from_slice(&chunk[..length]);
            if !announced && contains(&output, marker) {
                announced = true;
                let _ = ready.send(());
            }
        }
    })
}

fn remote_command(repository: &Path, yo: &Path, codex: &Path, option: &str) -> String {
    let codex_directory = codex.parent().expect("Codex executable has a parent");
    format!(
        "cd {repository} && stty rows 24 cols 80 && before=$(stty -g) && \
         PATH={codex_directory}:/usr/bin:/bin {yo} {option}; \
         result_code=$?; after=$(stty -g); test \"$before\" = \"$after\" || exit 91; \
         printf '\\n{restored}\\n'; exit $result_code",
        repository = shell_quote(repository),
        codex_directory = shell_quote(codex_directory),
        yo = shell_quote(yo),
        option = shell_quote(Path::new(option)),
        restored = String::from_utf8_lossy(RESTORED_MARKER),
    )
}

pub(super) fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\"'\"'"))
}

pub(super) fn wait_for_exit(child: &mut Child) -> std::process::ExitStatus {
    let deadline = Instant::now() + EXIT_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().expect("inspect SSH child") {
            return status;
        }
        if Instant::now() >= deadline {
            child.kill().expect("kill timed-out SSH child");
            let status = child.wait().expect("wait for timed-out SSH child");
            panic!("SSH PTY command exceeded {EXIT_TIMEOUT:?}: {status}");
        }
        thread::sleep(std::time::Duration::from_millis(10));
    }
}

pub(super) fn assert_ordered_pair(output: &[u8], enter: &[u8], leave: &[u8]) {
    let enter_at = position(output, enter).expect("screen entry must be observable");
    let leave_at = position(output, leave).expect("screen restoration must be observable");
    assert!(enter_at < leave_at, "screen restoration must follow entry");
    assert_eq!(count(output, enter), 1);
    assert_eq!(count(output, leave), 1);
}

fn position(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|candidate| candidate == needle)
}

fn last_position(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|candidate| candidate == needle)
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|candidate| *candidate == needle)
        .count()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    position(haystack, needle).is_some()
}
