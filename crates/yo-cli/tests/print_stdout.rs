#![cfg(unix)]

use std::{
    fs,
    io::Read,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, ExitStatus, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);
const PROCESS_DEADLINE: Duration = Duration::from_secs(10);

// 실제 yo 프로세스의 성공 경로가 app-server의 final answer만 정확히 한 번 쓰고,
// answer에 없던 마지막 LF 하나 외의 stdout byte를 만들지 않는지 확인합니다.
#[test]
fn successful_print_process_writes_only_one_framed_answer() {
    let fixture = Fixture::new();
    let output = fixture.run("success", ["-p", "--model", "host:codex", "prompt"]);

    assert!(output.status.success(), "{}", output.diagnostic());
    assert_eq!(output.stdout, b"process answer\n");
    assert!(output.stderr.is_empty(), "{}", output.diagnostic());
}

// app-server가 Turn을 failed로 닫으면 실제 process가 nonzero와 stderr 진단을 반환하고,
// final answer가 없는 stdout은 단 한 byte도 게시하지 않는지 확인합니다.
#[test]
fn generation_failure_keeps_process_stdout_empty() {
    let fixture = Fixture::new();
    let output = fixture.run(
        "generation-failure",
        ["-p", "--model", "host:codex", "prompt"],
    );

    assert!(
        !output.status.success(),
        "generation failure must be nonzero"
    );
    assert!(output.stdout.is_empty(), "{}", output.diagnostic());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("model stream failed"),
        "{}",
        output.diagnostic()
    );
}

// final answer가 완성된 뒤 app-server가 nonzero로 종료되면 agent cleanup 실패가 buffered
// answer의 eligibility를 닫아 실제 stdout을 비우고 stderr로만 실패를 내는지 확인합니다.
#[test]
fn cleanup_failure_withholds_the_buffered_process_answer() {
    let fixture = Fixture::new();
    let output = fixture.run("cleanup-failure", ["-p", "--model", "host:codex", "prompt"]);

    assert!(!output.status.success(), "cleanup failure must be nonzero");
    assert!(output.stdout.is_empty(), "{}", output.diagnostic());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("agent cleanup"), "{}", output.diagnostic());
    assert!(stderr.contains("exit status: 7"), "{}", output.diagnostic());
}

// PATH에 Codex가 없는 실제 startup 실패도 provider나 Session fallback 없이 nonzero가 되고,
// 진단은 stderr에만 남아 print stdout framing 경계를 열지 않는지 확인합니다.
#[test]
fn startup_failure_keeps_process_stdout_empty() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.fake_codex()).unwrap();
    let output = fixture.run("success", ["-p", "--model", "host:codex", "prompt"]);

    assert!(!output.status.success(), "startup failure must be nonzero");
    assert!(output.stdout.is_empty(), "{}", output.diagnostic());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("starting Codex"), "{}", output.diagnostic());
}

// print flag와 top-level subcommand의 실제 argv 충돌은 backend 시작 전 nonzero로 끝나고,
// literal separator 안내만 stderr에 기록하며 stdout은 비워 두는 계약을 유지합니다.
#[test]
fn print_and_top_level_subcommand_conflict_is_a_process_failure() {
    let fixture = Fixture::new();
    let output = fixture.run("success", ["-p", "session"]);

    assert!(
        !output.status.success(),
        "argument conflict must be nonzero"
    );
    assert!(output.stdout.is_empty(), "{}", output.diagnostic());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("use `--` before a literal prompt"),
        "{}",
        output.diagnostic()
    );
}

struct Fixture {
    root: PathBuf,
    bin: PathBuf,
    workspace: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "yo-print-stdout-e2e-{}-{sequence}",
            std::process::id()
        ));
        let bin = root.join("bin");
        let workspace = root.join("workspace");
        for path in [
            &bin,
            &workspace,
            &root.join("home"),
            &root.join("state"),
            &root.join("sessions"),
        ] {
            fs::create_dir_all(path).unwrap();
        }
        let fixture = Self {
            root,
            bin,
            workspace,
        };
        fixture.write_fake_codex();
        fixture
    }

    fn run<const N: usize>(&self, mode: &str, arguments: [&str; N]) -> ProcessOutput {
        let mut command = Command::new(env!("CARGO_BIN_EXE_yo"));
        command
            .args(arguments)
            .current_dir(&self.workspace)
            .env("PATH", &self.bin)
            .env("HOME", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("YO_CONFIG", self.root.join("config.yaml"))
            .env("YO_SESSION_REPOSITORY", self.root.join("sessions"))
            .env("YO_SESSION_CAPACITY_BYTES", "16777216")
            .env("YO_PRINT_STDOUT_E2E_MODE", mode)
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        run_bounded(command, PROCESS_DEADLINE)
    }

    fn fake_codex(&self) -> PathBuf {
        self.bin.join("codex")
    }

    fn write_fake_codex(&self) {
        let path = self.fake_codex();
        fs::write(
            &path,
            r#"#!/bin/sh
set -eu
mode=${YO_PRINT_STDOUT_E2E_MODE:-success}
while IFS= read -r message; do
  case "$message" in
    *'"method":"initialize"'*)
      printf '%s\n' '{"id":1,"result":{"userAgent":"codex_cli_rs/0.149.1 (e2e)","platformFamily":"unix","platformOs":"linux","codexHome":"/tmp/codex-e2e"}}'
      ;;
    *'"method":"account/read"'*)
      printf '%s\n' '{"id":2,"result":{"account":{"type":"chatgpt","email":"person@example.test","planType":"pro"}}}'
      ;;
    *'"method":"thread/start"'*)
      printf '%s\n' '{"id":3,"result":{"thread":{"id":"thread-e2e","sessionId":"session-e2e"},"model":"gpt-e2e","modelProvider":"openai"}}'
      ;;
    *'"method":"turn/start"'*)
      printf '%s\n' '{"id":4,"result":{"turn":{"id":"turn-e2e"}}}'
      if [ "$mode" = generation-failure ]; then
        printf '%s\n' '{"method":"error","params":{"threadId":"thread-e2e","turnId":"turn-e2e","error":{"message":"model stream failed"}}}'
        printf '%s\n' '{"method":"turn/completed","params":{"threadId":"thread-e2e","turn":{"id":"turn-e2e","status":"failed","error":{"message":"less specific"}}}}'
      else
        printf '%s\n' '{"method":"item/started","params":{"threadId":"thread-e2e","turnId":"turn-e2e","item":{"id":"item-e2e","type":"agentMessage","status":"inProgress"}}}'
        printf '%s\n' '{"method":"item/completed","params":{"threadId":"thread-e2e","turnId":"turn-e2e","item":{"id":"item-e2e","type":"agentMessage","status":"completed","text":"process answer"}}}'
        printf '%s\n' '{"method":"turn/completed","params":{"threadId":"thread-e2e","turn":{"id":"turn-e2e","status":"completed"}}}'
        if [ "$mode" = cleanup-failure ]; then
          exit 7
        fi
      fi
      ;;
  esac
done
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct ProcessOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl ProcessOutput {
    fn diagnostic(&self) -> String {
        format!(
            "status={}; stdout={:?}; stderr={:?}",
            self.status,
            String::from_utf8_lossy(&self.stdout),
            String::from_utf8_lossy(&self.stderr)
        )
    }
}

fn run_bounded(mut command: Command, timeout: Duration) -> ProcessOutput {
    let mut child = command.spawn().unwrap();
    let stdout = capture(child.stdout.take().unwrap());
    let stderr = capture(child.stderr.take().unwrap());
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let stdout = stdout.join().unwrap();
            let stderr = stderr.join().unwrap();
            panic!(
                "yo process exceeded {timeout:?}: stdout={:?}; stderr={:?}",
                String::from_utf8_lossy(&stdout),
                String::from_utf8_lossy(&stderr)
            );
        }
        thread::sleep(Duration::from_millis(10));
    };
    ProcessOutput {
        status,
        stdout: stdout.join().unwrap(),
        stderr: stderr.join().unwrap(),
    }
}

fn capture(mut pipe: impl Read + Send + 'static) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        pipe.read_to_end(&mut bytes).unwrap();
        bytes
    })
}
