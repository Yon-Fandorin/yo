use std::{
    io::Write,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::AtomicBool,
        mpsc::{Receiver, RecvTimeoutError, TryRecvError},
    },
    thread::JoinHandle,
    time::Duration,
};

use serde_json::Value;

use super::{
    config::{StdioJsonlConfig, initialization_failure},
    reader::{self, ReaderMessage},
};
use crate::{BackendFailure, BackendFailureKind, BackendStopHandle};

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
    /// Sends one already encoded JSON value. Implementations backed by a byte stream should
    /// write these bytes directly so callers can enforce one complete-message boundary without
    /// serializing the envelope a second time.
    fn send_encoded(&mut self, encoded: &[u8]) -> Result<(), BackendFailure> {
        let message: Value = serde_json::from_slice(encoded).map_err(|error| {
            BackendFailure::new(
                BackendFailureKind::Protocol,
                format!("failed to validate encoded JSONL message: {error}"),
            )
        })?;
        self.send(&message)
    }
    fn receive(&mut self, timeout: Duration) -> Result<JsonlPoll, BackendFailure>;
    fn try_receive(&mut self) -> Result<JsonlPoll, BackendFailure>;
    fn shutdown(&mut self) -> Result<(), BackendFailure>;
}

pub(super) struct ProcessControl {
    pub(super) child: Mutex<Child>,
    pub(super) stop_requested: AtomicBool,
}

/// Bounded JSONL peer backed by one owned child process.
pub struct StdioJsonlPeer {
    pub(super) process_name: &'static str,
    pub(super) process: Arc<ProcessControl>,
    pub(super) stdin: Option<ChildStdin>,
    pub(super) receiver: Receiver<ReaderMessage>,
    pub(super) reader: Option<JoinHandle<()>>,
    pub(super) stderr_reader: Option<JoinHandle<()>>,
    pub(super) stderr_tail: Arc<Mutex<String>>,
    pub(super) stderr_diagnostic: Option<fn(&str) -> Option<&'static str>>,
    pub(super) shutdown_timeout: Duration,
    pub(super) shutdown_result: Option<Result<(), BackendFailure>>,
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

        let (receiver, reader) = match reader::spawn_stdout_reader(
            stdout,
            config.process_name,
            config.thread_name,
            config.maximum_message_bytes,
        ) {
            Ok(readers) => readers,
            Err(error) => {
                terminate_incomplete_spawn(&mut child);
                return Err(error);
            },
        };

        let (stderr_tail, stderr_reader) =
            match reader::spawn_stderr_reader(stderr, config.process_name, config.thread_name) {
                Ok(readers) => readers,
                Err(error) => {
                    terminate_incomplete_spawn(&mut child);
                    drop(receiver);
                    let _ = reader.join();
                    return Err(error);
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
            stderr_diagnostic: config.stderr_diagnostic,
            shutdown_timeout: config.shutdown_timeout,
            shutdown_result: None,
        })
    }

    pub fn send(&mut self, message: &Value) -> Result<(), BackendFailure> {
        let encoded = serde_json::to_vec(message).map_err(|error| {
            BackendFailure::new(
                BackendFailureKind::Protocol,
                format!("failed to encode {} request: {error}", self.process_name),
            )
        })?;
        self.send_encoded(&encoded)
    }

    pub fn send_encoded(&mut self, encoded: &[u8]) -> Result<(), BackendFailure> {
        let stdin = self.stdin.as_mut().ok_or_else(|| {
            BackendFailure::new(
                BackendFailureKind::ProcessExit,
                format!("{} stdin is closed", self.process_name),
            )
        })?;
        let write_result = stdin
            .write_all(encoded)
            .and_then(|()| stdin.write_all(b"\n"))
            .and_then(|()| stdin.flush());
        write_result.map_err(|error| {
            self.with_failure_diagnostic(BackendFailure::new(
                BackendFailureKind::ProcessExit,
                format!("failed writing {} stdin: {error}", self.process_name),
            ))
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

    fn map_reader(&self, message: ReaderMessage) -> Result<JsonlPoll, BackendFailure> {
        match message {
            ReaderMessage::Value(value) => Ok(JsonlPoll::Message(value)),
            ReaderMessage::Closed => self.closed_poll(),
            ReaderMessage::Failed(message) => {
                Err(BackendFailure::new(BackendFailureKind::Protocol, message))
            },
        }
    }
}

impl JsonMessagePeer for StdioJsonlPeer {
    fn stop_handle(&self) -> BackendStopHandle {
        Self::stop_handle(self)
    }

    fn send(&mut self, message: &Value) -> Result<(), BackendFailure> {
        Self::send(self, message)
    }

    fn send_encoded(&mut self, encoded: &[u8]) -> Result<(), BackendFailure> {
        Self::send_encoded(self, encoded)
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

fn terminate_incomplete_spawn(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}
