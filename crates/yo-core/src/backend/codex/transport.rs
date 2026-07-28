use std::{
    io::{BufRead, BufReader, Write},
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

use super::CodexBackendConfig;
use crate::{BackendFailure, BackendFailureKind, BackendStopHandle};

pub(super) enum PeerPoll {
    Pending,
    Message(Value),
    Closed,
}

pub(super) trait JsonPeer {
    fn stop_handle(&self) -> BackendStopHandle;
    fn send(&mut self, message: &Value) -> Result<(), BackendFailure>;
    fn receive(&mut self, timeout: Duration) -> Result<PeerPoll, BackendFailure>;
    fn try_receive(&mut self) -> Result<PeerPoll, BackendFailure>;
    fn shutdown(&mut self) -> Result<(), BackendFailure>;
}

enum ReaderMessage {
    Value(Value),
    Failed(String),
    Closed,
}

pub(super) struct StdioPeer {
    process: Arc<ProcessControl>,
    stdin: Option<ChildStdin>,
    receiver: Receiver<ReaderMessage>,
    reader: Option<JoinHandle<()>>,
    stderr_reader: Option<JoinHandle<()>>,
    stderr_tail: Arc<Mutex<String>>,
    shutdown_timeout: Duration,
    shutdown_result: Option<Result<(), BackendFailure>>,
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

impl StdioPeer {
    pub(super) fn spawn(config: &CodexBackendConfig) -> Result<Self, BackendFailure> {
        let mut child = Command::new(config.executable())
            .args(["app-server", "--listen", "stdio://"])
            .current_dir(config.working_directory())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                BackendFailure::new(
                    BackendFailureKind::Unavailable,
                    format!("failed to spawn Codex app-server: {error}"),
                )
            })?;
        let Some(stdin) = child.stdin.take() else {
            terminate_incomplete_spawn(&mut child);
            return Err(BackendFailure::new(
                BackendFailureKind::Initialization,
                "Codex app-server stdin was not piped",
            ));
        };
        let Some(stdout) = child.stdout.take() else {
            terminate_incomplete_spawn(&mut child);
            return Err(BackendFailure::new(
                BackendFailureKind::Initialization,
                "Codex app-server stdout was not piped",
            ));
        };
        let Some(stderr) = child.stderr.take() else {
            terminate_incomplete_spawn(&mut child);
            return Err(BackendFailure::new(
                BackendFailureKind::Initialization,
                "Codex app-server stderr was not piped",
            ));
        };
        let (sender, receiver) = mpsc::sync_channel(256);
        let reader = thread::Builder::new()
            .name("yo-codex-jsonl-reader".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    match line {
                        Ok(line) if line.trim().is_empty() => {},
                        Ok(line) => match serde_json::from_str(&line) {
                            Ok(value) => {
                                if sender.send(ReaderMessage::Value(value)).is_err() {
                                    return;
                                }
                            },
                            Err(error) => {
                                let _ = sender.send(ReaderMessage::Failed(format!(
                                    "invalid JSONL from Codex app-server: {error}"
                                )));
                                return;
                            },
                        },
                        Err(error) => {
                            let _ = sender.send(ReaderMessage::Failed(format!(
                                "failed reading Codex app-server stdout: {error}"
                            )));
                            return;
                        },
                    }
                }
                let _ = sender.send(ReaderMessage::Closed);
            })
            .map_err(|error| {
                let _ = child.kill();
                let _ = child.wait();
                BackendFailure::new(
                    BackendFailureKind::Initialization,
                    format!("failed to start Codex app-server reader: {error}"),
                )
            })?;
        let stderr_tail = Arc::new(Mutex::new(String::new()));
        let captured_stderr = Arc::clone(&stderr_tail);
        let stderr_reader = match thread::Builder::new()
            .name("yo-codex-stderr-reader".into())
            .spawn(move || {
                for line in BufReader::new(stderr).lines() {
                    let Ok(line) = line else {
                        return;
                    };
                    let Ok(mut tail) = captured_stderr.lock() else {
                        return;
                    };
                    tail.push_str(&line);
                    tail.push('\n');
                    const LIMIT: usize = 16 * 1024;
                    if tail.len() > LIMIT {
                        let excess = tail.len() - LIMIT;
                        let boundary = tail
                            .char_indices()
                            .map(|(index, _)| index)
                            .find(|index| *index >= excess)
                            .unwrap_or(excess);
                        tail.drain(..boundary);
                    }
                }
            }) {
            Ok(stderr_reader) => stderr_reader,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                drop(receiver);
                let _ = reader.join();
                return Err(BackendFailure::new(
                    BackendFailureKind::Initialization,
                    format!("failed to start Codex app-server stderr reader: {error}"),
                ));
            },
        };

        Ok(Self {
            process: Arc::new(ProcessControl {
                child: Mutex::new(child),
                stop_requested: AtomicBool::new(false),
            }),
            stdin: Some(stdin),
            receiver,
            reader: Some(reader),
            stderr_reader: Some(stderr_reader),
            stderr_tail,
            shutdown_timeout: config.shutdown_timeout(),
            shutdown_result: None,
        })
    }

    pub(super) fn stop_handle(&self) -> BackendStopHandle {
        let process = Arc::clone(&self.process);
        BackendStopHandle::new(move || process.request_stop())
    }

    fn map_reader(&self, message: ReaderMessage) -> Result<PeerPoll, BackendFailure> {
        match message {
            ReaderMessage::Value(value) => Ok(PeerPoll::Message(value)),
            ReaderMessage::Closed => self.closed_poll(),
            ReaderMessage::Failed(message) => {
                Err(BackendFailure::new(BackendFailureKind::Protocol, message))
            },
        }
    }

    fn closed_poll(&self) -> Result<PeerPoll, BackendFailure> {
        if self.shutdown_result.is_some() {
            return Ok(PeerPoll::Closed);
        }
        let detail = self
            .stderr_tail
            .lock()
            .ok()
            .map(|tail| tail.trim().to_owned())
            .filter(|tail| !tail.is_empty());
        let message = match detail {
            Some(detail) => format!("Codex app-server exited unexpectedly: {detail}"),
            None => "Codex app-server exited unexpectedly".to_owned(),
        };
        Err(BackendFailure::new(
            BackendFailureKind::ProcessExit,
            message,
        ))
    }

    fn stop_child(&mut self) -> Result<(), BackendFailure> {
        self.stdin.take();
        let deadline = Instant::now() + self.shutdown_timeout;
        let process_failure = loop {
            let status = self
                .process
                .child
                .lock()
                .map_err(|_| {
                    BackendFailure::new(
                        BackendFailureKind::Cleanup,
                        "Codex app-server process lock was poisoned",
                    )
                })?
                .try_wait();
            match status {
                Ok(Some(status)) if status.success() => break None,
                Ok(Some(_)) if self.process.stop_requested.load(Ordering::Acquire) => break None,
                Ok(Some(status)) => {
                    break Some(BackendFailure::new(
                        BackendFailureKind::Cleanup,
                        format!("Codex app-server exited with {status}"),
                    ));
                },
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                },
                Ok(None) => {
                    let mut child = self.process.child.lock().map_err(|_| {
                        BackendFailure::new(
                            BackendFailureKind::Cleanup,
                            "Codex app-server process lock was poisoned",
                        )
                    })?;
                    child.kill().map_err(|error| {
                        BackendFailure::new(
                            BackendFailureKind::Cleanup,
                            format!("failed to terminate Codex app-server: {error}"),
                        )
                    })?;
                    child.wait().map_err(|error| {
                        BackendFailure::new(
                            BackendFailureKind::Cleanup,
                            format!("failed to reap Codex app-server: {error}"),
                        )
                    })?;
                    break None;
                },
                Err(error) => {
                    return Err(BackendFailure::new(
                        BackendFailureKind::Cleanup,
                        format!("failed to inspect Codex app-server: {error}"),
                    ));
                },
            }
        };
        let reader_failure = self.join_readers().err();
        match (process_failure, reader_failure) {
            (None, None) => Ok(()),
            (Some(failure), None) | (None, Some(failure)) => Err(failure),
            (Some(process), Some(reader)) => Err(BackendFailure::new(
                BackendFailureKind::Cleanup,
                format!("{process}; additionally, {reader}"),
            )),
        }
    }

    fn join_readers(&mut self) -> Result<(), BackendFailure> {
        while let Ok(message) = self.receiver.recv() {
            if matches!(message, ReaderMessage::Closed | ReaderMessage::Failed(_)) {
                break;
            }
        }
        if let Some(reader) = self.reader.take() {
            reader.join().map_err(|_| {
                BackendFailure::new(
                    BackendFailureKind::Cleanup,
                    "Codex app-server reader panicked",
                )
            })?;
        }
        if let Some(reader) = self.stderr_reader.take() {
            reader.join().map_err(|_| {
                BackendFailure::new(
                    BackendFailureKind::Cleanup,
                    "Codex app-server stderr reader panicked",
                )
            })?;
        }
        Ok(())
    }
}

impl JsonPeer for StdioPeer {
    fn stop_handle(&self) -> BackendStopHandle {
        self.stop_handle()
    }

    fn send(&mut self, message: &Value) -> Result<(), BackendFailure> {
        let stdin = self.stdin.as_mut().ok_or_else(|| {
            BackendFailure::new(
                BackendFailureKind::ProcessExit,
                "Codex app-server stdin is closed",
            )
        })?;
        let mut line = serde_json::to_vec(message).map_err(|error| {
            BackendFailure::new(
                BackendFailureKind::Protocol,
                format!("failed to encode Codex request: {error}"),
            )
        })?;
        line.push(b'\n');
        stdin
            .write_all(&line)
            .and_then(|()| stdin.flush())
            .map_err(|error| {
                BackendFailure::new(
                    BackendFailureKind::ProcessExit,
                    format!("failed writing Codex app-server stdin: {error}"),
                )
            })
    }

    fn receive(&mut self, timeout: Duration) -> Result<PeerPoll, BackendFailure> {
        match self.receiver.recv_timeout(timeout) {
            Ok(message) => self.map_reader(message),
            Err(RecvTimeoutError::Timeout) => Err(BackendFailure::new(
                BackendFailureKind::Unavailable,
                "timed out waiting for Codex app-server",
            )),
            Err(RecvTimeoutError::Disconnected) => self.closed_poll(),
        }
    }

    fn try_receive(&mut self) -> Result<PeerPoll, BackendFailure> {
        match self.receiver.try_recv() {
            Ok(message) => self.map_reader(message),
            Err(TryRecvError::Empty) => Ok(PeerPoll::Pending),
            Err(TryRecvError::Disconnected) => self.closed_poll(),
        }
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        if let Some(result) = &self.shutdown_result {
            return result.clone();
        }
        let result = self.stop_child();
        self.shutdown_result = Some(result.clone());
        result
    }
}

impl Drop for StdioPeer {
    fn drop(&mut self) {
        let reaped = self.process.child.lock().is_ok_and(|mut child| {
            if matches!(child.try_wait(), Ok(Some(_))) {
                true
            } else {
                let _ = child.kill();
                child.wait().is_ok()
            }
        });
        if reaped {
            let _ = self.join_readers();
        }
    }
}

fn terminate_incomplete_spawn(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(all(test, unix))]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };

    use super::{JsonPeer, StdioPeer};
    use crate::backend::codex::CodexBackendConfig;

    static NEXT_SCRIPT: AtomicU64 = AtomicU64::new(1);

    // stdout reader가 bounded queue 용량보다 많은 메시지를 보내며 멈춘 상태에서도 shutdown이
    // queue를 EOF까지 비워 reader를 깨우고 자식 프로세스를 교착 없이 회수하는지 확인한다.
    #[test]
    fn shutdown_drains_a_reader_blocked_on_the_full_queue() {
        let suffix = NEXT_SCRIPT.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "yo-codex-transport-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let script = directory.join("fake-codex");
        fs::write(
            &script,
            "#!/bin/sh\n\
             i=0\n\
             while [ \"$i\" -lt 300 ]; do\n\
               printf '%s\\n' '{\"method\":\"warning\",\"params\":{\"message\":\"queued\"}}'\n\
               i=$((i + 1))\n\
             done\n\
             cat >/dev/null\n",
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        let config = CodexBackendConfig::new(&directory)
            .with_executable(&script)
            .with_shutdown_timeout(Duration::from_secs(1));
        let mut peer = StdioPeer::spawn(&config).unwrap();
        std::thread::sleep(Duration::from_millis(50));

        let result = peer.shutdown();

        let _ = fs::remove_file(&script);
        let _ = fs::remove_dir(&directory);
        result.unwrap();
    }
}
