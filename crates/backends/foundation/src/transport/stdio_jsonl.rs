use std::{
    ffi::OsString,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use serde_json::Value;

use crate::{BackendFailure, BackendFailureKind, BackendStopHandle};

pub const DEFAULT_MAX_JSONL_MESSAGE_BYTES: usize = 1024 * 1024;
const MESSAGE_QUEUE_CAPACITY: usize = 256;
const STDERR_TAIL_BYTES: usize = 16 * 1024;

/// Process launch and resource bounds for one stdio JSONL peer.
#[derive(Clone, Debug)]
pub struct StdioJsonlConfig {
    executable: PathBuf,
    arguments: Vec<OsString>,
    working_directory: PathBuf,
    process_name: &'static str,
    thread_name: &'static str,
    shutdown_timeout: Duration,
    maximum_message_bytes: usize,
}

impl StdioJsonlConfig {
    pub fn new(
        process_name: &'static str,
        thread_name: &'static str,
        executable: impl Into<PathBuf>,
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        Self {
            executable: executable.into(),
            arguments: Vec::new(),
            working_directory: working_directory.into(),
            process_name,
            thread_name,
            shutdown_timeout: Duration::from_secs(2),
            maximum_message_bytes: DEFAULT_MAX_JSONL_MESSAGE_BYTES,
        }
    }

    #[must_use]
    pub fn with_arguments<I, S>(mut self, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.arguments = arguments.into_iter().map(Into::into).collect();
        self
    }

    #[must_use]
    pub const fn with_shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }

    #[must_use]
    pub const fn with_maximum_message_bytes(mut self, bytes: usize) -> Self {
        self.maximum_message_bytes = bytes;
        self
    }

    fn validate(&self) -> Result<(), BackendFailure> {
        if self.process_name.is_empty()
            || self.thread_name.is_empty()
            || self.shutdown_timeout.is_zero()
            || self.maximum_message_bytes == 0
            || self.maximum_message_bytes > usize::MAX - 2
            || !self.working_directory.is_absolute()
        {
            return Err(initialization_failure(
                self.process_name,
                "invalid stdio JSONL process configuration",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum JsonlPoll {
    Pending,
    Message(Value),
    Closed,
}

/// Synchronous JSON message peer used by protocol-specific delegated adapters.
pub trait JsonMessagePeer {
    fn stop_handle(&self) -> BackendStopHandle;
    fn send(&mut self, message: &Value) -> Result<(), BackendFailure>;
    fn receive(&mut self, timeout: Duration) -> Result<JsonlPoll, BackendFailure>;
    fn try_receive(&mut self) -> Result<JsonlPoll, BackendFailure>;
    fn shutdown(&mut self) -> Result<(), BackendFailure>;
}

enum ReaderMessage {
    Value(Value),
    Failed(String),
    Closed,
}

struct ProcessControl {
    child: Mutex<Child>,
    stop_requested: AtomicBool,
}

impl ProcessControl {
    fn request_stop(&self) {
        self.stop_requested.store(true, Ordering::Release);
        let Ok(mut child) = self.child.lock() else {
            return;
        };
        if matches!(child.try_wait(), Ok(None)) {
            let _ = child.kill();
        }
    }
}

/// Bounded JSONL peer backed by one owned child process.
pub struct StdioJsonlPeer {
    process_name: &'static str,
    process: Arc<ProcessControl>,
    stdin: Option<ChildStdin>,
    receiver: Receiver<ReaderMessage>,
    reader: Option<JoinHandle<()>>,
    stderr_reader: Option<JoinHandle<()>>,
    stderr_tail: Arc<Mutex<String>>,
    shutdown_timeout: Duration,
    shutdown_result: Option<Result<(), BackendFailure>>,
}

impl StdioJsonlPeer {
    pub fn spawn(config: StdioJsonlConfig) -> Result<Self, BackendFailure> {
        config.validate()?;
        let mut child = Command::new(&config.executable)
            .args(&config.arguments)
            .current_dir(&config.working_directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                BackendFailure::new(
                    BackendFailureKind::Unavailable,
                    format!("failed to spawn {}: {error}", config.process_name),
                )
            })?;
        let Some(stdin) = child.stdin.take() else {
            terminate_incomplete_spawn(&mut child);
            return Err(initialization_failure(
                config.process_name,
                "stdin was not piped",
            ));
        };
        let Some(stdout) = child.stdout.take() else {
            terminate_incomplete_spawn(&mut child);
            return Err(initialization_failure(
                config.process_name,
                "stdout was not piped",
            ));
        };
        let Some(stderr) = child.stderr.take() else {
            terminate_incomplete_spawn(&mut child);
            return Err(initialization_failure(
                config.process_name,
                "stderr was not piped",
            ));
        };

        let (sender, receiver) = mpsc::sync_channel(MESSAGE_QUEUE_CAPACITY);
        let stdout_name = config.process_name;
        let maximum_message_bytes = config.maximum_message_bytes;
        let reader = thread::Builder::new()
            .name(format!("yo-{}-jsonl-reader", config.thread_name))
            .spawn(move || {
                let mut stdout = BufReader::new(stdout);
                loop {
                    match read_jsonl_message(&mut stdout, stdout_name, maximum_message_bytes) {
                        Ok(Some(value)) => {
                            if sender.send(ReaderMessage::Value(value)).is_err() {
                                return;
                            }
                        },
                        Ok(None) => {
                            let _ = sender.send(ReaderMessage::Closed);
                            return;
                        },
                        Err(error) => {
                            let _ = sender.send(ReaderMessage::Failed(error));
                            return;
                        },
                    }
                }
            })
            .map_err(|error| {
                terminate_incomplete_spawn(&mut child);
                initialization_failure(
                    config.process_name,
                    format!("failed to start stdout reader: {error}"),
                )
            })?;

        let stderr_tail = Arc::new(Mutex::new(String::new()));
        let captured_stderr = Arc::clone(&stderr_tail);
        let stderr_reader = match thread::Builder::new()
            .name(format!("yo-{}-stderr-reader", config.thread_name))
            .spawn(move || capture_stderr(stderr, captured_stderr))
        {
            Ok(reader) => reader,
            Err(error) => {
                terminate_incomplete_spawn(&mut child);
                drop(receiver);
                let _ = reader.join();
                return Err(initialization_failure(
                    config.process_name,
                    format!("failed to start stderr reader: {error}"),
                ));
            },
        };

        Ok(Self {
            process_name: config.process_name,
            process: Arc::new(ProcessControl {
                child: Mutex::new(child),
                stop_requested: AtomicBool::new(false),
            }),
            stdin: Some(stdin),
            receiver,
            reader: Some(reader),
            stderr_reader: Some(stderr_reader),
            stderr_tail,
            shutdown_timeout: config.shutdown_timeout,
            shutdown_result: None,
        })
    }

    pub fn stop_handle(&self) -> BackendStopHandle {
        let process = Arc::clone(&self.process);
        BackendStopHandle::new(move || process.request_stop())
    }

    pub fn send(&mut self, message: &Value) -> Result<(), BackendFailure> {
        let stdin = self.stdin.as_mut().ok_or_else(|| {
            BackendFailure::new(
                BackendFailureKind::ProcessExit,
                format!("{} stdin is closed", self.process_name),
            )
        })?;
        let mut line = serde_json::to_vec(message).map_err(|error| {
            BackendFailure::new(
                BackendFailureKind::Protocol,
                format!("failed to encode {} request: {error}", self.process_name),
            )
        })?;
        line.push(b'\n');
        stdin
            .write_all(&line)
            .and_then(|()| stdin.flush())
            .map_err(|error| {
                BackendFailure::new(
                    BackendFailureKind::ProcessExit,
                    format!("failed writing {} stdin: {error}", self.process_name),
                )
            })
    }

    pub fn receive(&mut self, timeout: Duration) -> Result<JsonlPoll, BackendFailure> {
        match self.receiver.recv_timeout(timeout) {
            Ok(message) => self.map_reader(message),
            Err(RecvTimeoutError::Timeout) => Err(BackendFailure::new(
                BackendFailureKind::Unavailable,
                format!("timed out waiting for {}", self.process_name),
            )),
            Err(RecvTimeoutError::Disconnected) => self.closed_poll(),
        }
    }

    pub fn try_receive(&mut self) -> Result<JsonlPoll, BackendFailure> {
        match self.receiver.try_recv() {
            Ok(message) => self.map_reader(message),
            Err(TryRecvError::Empty) => Ok(JsonlPoll::Pending),
            Err(TryRecvError::Disconnected) => self.closed_poll(),
        }
    }

    pub fn shutdown(&mut self) -> Result<(), BackendFailure> {
        if let Some(result) = &self.shutdown_result {
            return result.clone();
        }
        self.stdin.take();
        let result = self.shutdown_once();
        self.shutdown_result = Some(result.clone());
        result
    }

    fn map_reader(&self, message: ReaderMessage) -> Result<JsonlPoll, BackendFailure> {
        match message {
            ReaderMessage::Value(value) => Ok(JsonlPoll::Message(value)),
            ReaderMessage::Closed => self.closed_poll(),
            ReaderMessage::Failed(message) => {
                Err(BackendFailure::new(BackendFailureKind::Protocol, message))
            },
        }
    }

    fn closed_poll(&self) -> Result<JsonlPoll, BackendFailure> {
        if self.shutdown_result.is_some() {
            return Ok(JsonlPoll::Closed);
        }
        let detail = self
            .stderr_tail
            .lock()
            .ok()
            .map(|tail| tail.trim().to_owned())
            .filter(|tail| !tail.is_empty());
        let message = detail.map_or_else(
            || format!("{} exited unexpectedly", self.process_name),
            |detail| format!("{} exited unexpectedly: {detail}", self.process_name),
        );
        Err(BackendFailure::new(
            BackendFailureKind::ProcessExit,
            message,
        ))
    }

    fn shutdown_once(&mut self) -> Result<(), BackendFailure> {
        let deadline = Instant::now() + self.shutdown_timeout;
        let process_failure = loop {
            let status = self
                .process
                .child
                .lock()
                .map_err(|_| cleanup_failure(self.process_name, "process lock was poisoned"))?
                .try_wait();
            match status {
                Ok(Some(status)) if status.success() => break None,
                Ok(Some(_)) if self.process.stop_requested.load(Ordering::Acquire) => break None,
                Ok(Some(status)) => {
                    break Some(cleanup_failure(
                        self.process_name,
                        format!("exited with {status}"),
                    ));
                },
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                Ok(None) => {
                    let mut child = self.process.child.lock().map_err(|_| {
                        cleanup_failure(self.process_name, "process lock was poisoned")
                    })?;
                    child.kill().map_err(|error| {
                        cleanup_failure(self.process_name, format!("failed to terminate: {error}"))
                    })?;
                    child.wait().map_err(|error| {
                        cleanup_failure(self.process_name, format!("failed to reap: {error}"))
                    })?;
                    break None;
                },
                Err(error) => {
                    return Err(cleanup_failure(
                        self.process_name,
                        format!("failed to inspect process: {error}"),
                    ));
                },
            }
        };
        let reader_failure = self.drain_and_join().err();
        match (process_failure, reader_failure) {
            (None, None) => Ok(()),
            (Some(failure), None) | (None, Some(failure)) => Err(failure),
            (Some(process), Some(reader)) => Err(cleanup_failure(
                self.process_name,
                format!("{process}; additionally, {reader}"),
            )),
        }
    }

    fn drain_and_join(&mut self) -> Result<(), BackendFailure> {
        if let Some(reader) = self.reader.take() {
            while !reader.is_finished() {
                while self.receiver.try_recv().is_ok() {}
                thread::yield_now();
            }
            while self.receiver.try_recv().is_ok() {}
            reader
                .join()
                .map_err(|_| cleanup_failure(self.process_name, "stdout reader panicked"))?;
        }
        if let Some(reader) = self.stderr_reader.take() {
            reader
                .join()
                .map_err(|_| cleanup_failure(self.process_name, "stderr reader panicked"))?;
        }
        Ok(())
    }
}

impl JsonMessagePeer for StdioJsonlPeer {
    fn stop_handle(&self) -> BackendStopHandle {
        Self::stop_handle(self)
    }

    fn send(&mut self, message: &Value) -> Result<(), BackendFailure> {
        Self::send(self, message)
    }

    fn receive(&mut self, timeout: Duration) -> Result<JsonlPoll, BackendFailure> {
        Self::receive(self, timeout)
    }

    fn try_receive(&mut self) -> Result<JsonlPoll, BackendFailure> {
        Self::try_receive(self)
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        Self::shutdown(self)
    }
}

impl Drop for StdioJsonlPeer {
    fn drop(&mut self) {
        if self.shutdown_result.is_none() {
            self.process.request_stop();
            let _ = self.shutdown();
        }
    }
}

fn capture_stderr(stderr: impl Read, captured: Arc<Mutex<String>>) {
    let mut stderr = BufReader::new(stderr);
    let mut chunk = [0_u8; 4096];
    loop {
        let Ok(read) = stderr.read(&mut chunk) else {
            return;
        };
        if read == 0 {
            return;
        }
        let Ok(mut tail) = captured.lock() else {
            return;
        };
        tail.push_str(&String::from_utf8_lossy(&chunk[..read]));
        if tail.len() > STDERR_TAIL_BYTES {
            let mut start = tail.len() - STDERR_TAIL_BYTES;
            while !tail.is_char_boundary(start) {
                start += 1;
            }
            tail.drain(..start);
        }
    }
}

fn read_jsonl_message(
    reader: &mut impl BufRead,
    process_name: &str,
    maximum_message_bytes: usize,
) -> Result<Option<Value>, String> {
    loop {
        let mut line = Vec::new();
        let read = {
            let read_limit = maximum_message_bytes
                .checked_add(2)
                .expect("validated maximum JSONL message size must fit usize");
            let mut limited = Read::by_ref(reader).take(read_limit as u64);
            limited
                .read_until(b'\n', &mut line)
                .map_err(|error| format!("failed reading {process_name} stdout: {error}"))?
        };
        if read == 0 {
            return Ok(None);
        }
        if line.last() == Some(&b'\n') {
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
        }
        if line.len() > maximum_message_bytes {
            return Err(format!(
                "{process_name} JSONL message exceeds the {maximum_message_bytes}-byte limit"
            ));
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        return serde_json::from_slice(&line)
            .map(Some)
            .map_err(|error| format!("invalid JSONL from {process_name}: {error}"));
    }
}

fn terminate_incomplete_spawn(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn initialization_failure(process_name: &str, message: impl Into<String>) -> BackendFailure {
    BackendFailure::new(
        BackendFailureKind::Initialization,
        format!("{process_name}: {}", message.into()),
    )
}

fn cleanup_failure(process_name: &str, message: impl Into<String>) -> BackendFailure {
    BackendFailure::new(
        BackendFailureKind::Cleanup,
        format!("{process_name}: {}", message.into()),
    )
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    // newline 없는 oversized 출력도 제한보다 한 byte만 읽고 실패해 child process가 reader
    // memory를 무제한으로 늘리지 못하게 합니다.
    #[test]
    fn rejects_an_oversized_message_while_reading() {
        let input = vec![b'x'; DEFAULT_MAX_JSONL_MESSAGE_BYTES + 1];
        let error = read_jsonl_message(
            &mut Cursor::new(input),
            "fixture",
            DEFAULT_MAX_JSONL_MESSAGE_BYTES,
        )
        .unwrap_err();

        assert!(error.contains("exceeds"));
        assert!(error.contains(&DEFAULT_MAX_JSONL_MESSAGE_BYTES.to_string()));
    }

    // LF와 CRLF delimiter는 payload 제한에 포함되지 않으므로 정확히 limit 크기인 JSON은
    // 두 줄바꿈 형식 모두에서 허용되어야 합니다.
    #[test]
    fn accepts_a_json_payload_exactly_at_the_message_limit() {
        let maximum_message_bytes = 32;
        let empty_payload = r#"{"value":""}"#;
        let padding = "x".repeat(maximum_message_bytes - empty_payload.len());
        let payload = format!(r#"{{"value":"{padding}"}}"#);
        assert_eq!(payload.len(), maximum_message_bytes);

        for delimiter in ["\n", "\r\n"] {
            let mut input = Cursor::new(format!("{payload}{delimiter}"));
            let value = read_jsonl_message(&mut input, "fixture", maximum_message_bytes)
                .unwrap()
                .unwrap();

            assert_eq!(value["value"], padding);
        }
    }

    // blank JSONL 행을 건너뛰어도 다음 bounded JSON object가 손실되지 않는지 검증합니다.
    #[test]
    fn skips_blank_lines_without_losing_the_next_message() {
        let mut input = Cursor::new(b" \r\n{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n");

        let value = read_jsonl_message(&mut input, "fixture", DEFAULT_MAX_JSONL_MESSAGE_BYTES)
            .unwrap()
            .unwrap();

        assert_eq!(value["id"], 1);
    }

    // JSONL read limit에 CRLF와 sentinel byte를 더할 수 없는 설정은 spawn 전에 거부되어야
    // release build에서도 size 계산이 wrap되지 않습니다.
    #[test]
    fn rejects_a_maximum_message_size_that_cannot_add_a_sentinel_byte() {
        let config = StdioJsonlConfig::new("fixture", "fixture", "/bin/false", "/")
            .with_maximum_message_bytes(usize::MAX);

        let error = config.validate().unwrap_err();

        assert_eq!(error.kind(), BackendFailureKind::Initialization);
    }

    #[cfg(unix)]
    mod unix {
        use std::{
            fs,
            os::unix::fs::PermissionsExt,
            path::PathBuf,
            sync::atomic::{AtomicU64, Ordering},
        };

        use super::*;

        static NEXT_SCRIPT: AtomicU64 = AtomicU64::new(1);

        fn spawn_fixture(label: &str, body: &str) -> (PathBuf, StdioJsonlPeer) {
            let suffix = NEXT_SCRIPT.fetch_add(1, Ordering::Relaxed);
            let directory = std::env::temp_dir().join(format!(
                "yo-backend-stdio-jsonl-{label}-{}-{suffix}",
                std::process::id()
            ));
            fs::create_dir(&directory).unwrap();
            let script = directory.join("fake-backend");
            fs::write(&script, format!("#!/bin/sh\n{body}")).unwrap();
            fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
            let config = StdioJsonlConfig::new("fixture", "fixture", &script, &directory)
                .with_shutdown_timeout(Duration::from_secs(1));
            (directory, StdioJsonlPeer::spawn(config).unwrap())
        }

        fn remove_fixture(directory: &PathBuf) {
            let _ = fs::remove_file(directory.join("fake-backend"));
            let _ = fs::remove_dir(directory);
        }

        // 실제 pipe-backed child가 stdin request를 받은 뒤 stdout JSONL response를
        // 돌려주는 왕복 경계를 검증해 protocol fixture가 transport를 우회하지 않게 합니다.
        #[test]
        fn exchanges_one_json_message_with_a_child_process() {
            let (directory, mut peer) = spawn_fixture(
                "round-trip",
                "IFS= read -r request\nprintf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}'\n",
            );

            peer.send(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {},
            }))
            .unwrap();
            let message = peer.receive(Duration::from_secs(1)).unwrap();

            peer.shutdown().unwrap();
            remove_fixture(&directory);
            assert_eq!(
                message,
                JsonlPoll::Message(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": {},
                }))
            );
        }

        // bounded queue가 가득 차 stdout reader가 대기해도 shutdown이 queue를 비우고 child를
        // 교착 없이 회수하는지 검증합니다.
        #[test]
        fn shutdown_drains_a_reader_blocked_on_the_full_queue() {
            let (directory, mut peer) = spawn_fixture(
                "full-queue",
                "i=0\nwhile [ \"$i\" -lt 300 ]; do\n  printf '%s\\n' '{\"method\":\"warning\",\"params\":{}}'\n  i=$((i + 1))\ndone\ncat >/dev/null\n",
            );
            thread::sleep(Duration::from_millis(50));

            let result = peer.shutdown();

            remove_fixture(&directory);
            result.unwrap();
        }

        // 명시적 shutdown 전 child가 exit 0으로 끝나도 정상 close로 숨기지 않고 stderr를
        // 포함한 ProcessExit로 분류해 상위 runtime이 Turn 실패를 관찰하게 합니다.
        #[test]
        fn unexpected_clean_exit_is_a_process_failure_with_stderr() {
            let (directory, mut peer) = spawn_fixture(
                "unexpected-clean-exit",
                "printf '%s\\n' 'clean-exit-diagnostic' >&2\n",
            );
            let deadline = Instant::now() + Duration::from_secs(1);
            while !peer
                .stderr_tail
                .lock()
                .unwrap()
                .contains("clean-exit-diagnostic")
                && Instant::now() < deadline
            {
                thread::yield_now();
            }

            let failure = peer.receive(Duration::from_secs(1)).unwrap_err();

            assert_eq!(failure.kind(), BackendFailureKind::ProcessExit);
            assert!(failure.message().contains("clean-exit-diagnostic"));
            peer.shutdown().unwrap();
            remove_fixture(&directory);
        }

        // stop handle은 진행 중 poll을 깨우지만 explicit shutdown 전의 child 종료는 기존
        // 계약대로 ProcessExit이며, 뒤이은 shutdown만 정상 cleanup으로 처리합니다.
        #[test]
        fn stop_handle_exit_is_a_process_failure_until_shutdown() {
            let (directory, mut peer) = spawn_fixture("stop-handle", "cat >/dev/null\n");

            peer.stop_handle().request_stop();
            let failure = peer.receive(Duration::from_secs(1)).unwrap_err();

            assert_eq!(failure.kind(), BackendFailureKind::ProcessExit);
            peer.shutdown().unwrap();
            remove_fixture(&directory);
        }

        // explicit shutdown 결과가 기록된 뒤에는 reader channel이 닫혀도 반복 poll이
        // ProcessExit로 되돌아가지 않고 안정적으로 Closed를 반환합니다.
        #[test]
        fn poll_after_explicit_shutdown_remains_closed() {
            let (directory, mut peer) = spawn_fixture("post-shutdown", "cat >/dev/null\n");

            peer.shutdown().unwrap();

            assert_eq!(peer.try_receive().unwrap(), JsonlPoll::Closed);
            remove_fixture(&directory);
        }
    }
}
