use std::{
    fs,
    net::{TcpListener, TcpStream},
    ops::Deref,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use super::READY_TIMEOUT;
use crate::support::require_command;

pub(super) struct SshServer {
    root: FixtureRoot,
    pub(super) port: u16,
    pub(super) identity: PathBuf,
    pub(super) codex: PathBuf,
    pub(super) child: Child,
}

impl SshServer {
    pub(super) fn start() -> Self {
        let sshd = command_path("sshd");
        require_command("ssh", &["-V"]);
        let _ = command_path("ssh-keygen");
        require_command("codex", &["--version"]);
        let codex = command_path("codex");

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos();
        let root = FixtureRoot::create(
            std::env::temp_dir().join(format!("yo-ssh-matrix-{}-{unique}", std::process::id())),
        );
        let host_key = root.join("host-key");
        let identity = root.join("client-key");
        generate_key(&host_key);
        generate_key(&identity);
        let authorized_keys = root.join("authorized_keys");
        fs::copy(identity.with_extension("pub"), &authorized_keys)
            .expect("install temporary authorized key");

        let config = root.join("sshd_config");
        let (port, child) = (0..5)
            .find_map(|_| {
                let (reservation, port) = available_port();
                fs::write(
                    &config,
                    format!(
                        "\
Port {port}
ListenAddress 127.0.0.1
HostKey {}
PidFile {}
AuthorizedKeysFile {}
PasswordAuthentication no
KbdInteractiveAuthentication no
UsePAM no
StrictModes no
PermitRootLogin no
LogLevel ERROR
",
                        host_key.display(),
                        root.join("sshd.pid").display(),
                        authorized_keys.display(),
                    ),
                )
                .expect("write isolated sshd configuration");
                drop(reservation);

                let child = Command::new(&sshd)
                    .args(["-D", "-e", "-f"])
                    .arg(&config)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::piped())
                    .spawn()
                    .expect("start isolated sshd");
                match wait_until_ready(child, port) {
                    Ok(child) => Some((port, child)),
                    Err(failure) if failure.contains("Address already in use") => None,
                    Err(failure) => panic!("isolated sshd failed to start: {failure}"),
                }
            })
            .expect("isolated sshd could not acquire a local port after five attempts");

        Self {
            root,
            port,
            identity,
            codex,
            child,
        }
    }

    pub(super) fn client(&self, allocate_pty: bool) -> Command {
        let destination = format!(
            "{}@127.0.0.1",
            std::env::var("USER").expect("USER identifies the local SSH account")
        );
        let mut command = Command::new("ssh");
        command
            .args(["-F", "/dev/null"])
            .arg(if allocate_pty { "-tt" } else { "-T" })
            .args([
                "-o",
                "BatchMode=yes",
                "-o",
                "IdentitiesOnly=yes",
                "-o",
                "StrictHostKeyChecking=no",
                "-o",
                "UserKnownHostsFile=/dev/null",
                "-o",
                "LogLevel=ERROR",
                "-p",
                &self.port.to_string(),
                "-i",
            ])
            .arg(&self.identity)
            .arg(destination);
        command
    }

    pub(super) fn fixture_path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for SshServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct FixtureRoot(PathBuf);

impl FixtureRoot {
    fn create(path: PathBuf) -> Self {
        fs::create_dir(&path).expect("create SSH fixture directory");
        Self(path)
    }
}

impl Deref for FixtureRoot {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for FixtureRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn generate_key(path: &Path) {
    let output = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(path)
        .output()
        .expect("run ssh-keygen");
    assert!(
        output.status.success(),
        "ssh-keygen failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn available_port() -> (TcpListener, u16) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("reserve local SSH port");
    let port = listener.local_addr().expect("read local SSH port").port();
    (listener, port)
}

fn wait_until_ready(mut child: Child, port: u16) -> Result<Child, String> {
    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Ok(child);
        }
        if let Some(status) = child.try_wait().expect("inspect sshd") {
            let mut stderr = String::new();
            std::io::Read::read_to_string(
                child.stderr.as_mut().expect("capture sshd stderr"),
                &mut stderr,
            )
            .expect("read sshd stderr");
            return Err(format!("{status}: {stderr}"));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("isolated sshd did not listen within {READY_TIMEOUT:?}");
        }
        thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn command_path(command: &str) -> PathBuf {
    let output = Command::new("sh")
        .args(["-c", "command -v \"$1\"", "sh", command])
        .output()
        .expect("resolve command path");
    assert!(output.status.success(), "cannot resolve `{command}`");
    fs::canonicalize(
        String::from_utf8(output.stdout)
            .expect("command path is UTF-8")
            .trim(),
    )
    .expect("canonicalize command path")
}
