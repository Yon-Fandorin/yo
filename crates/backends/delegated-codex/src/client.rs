use std::time::{Duration, Instant};

use serde_json::{Value, json};
use yo_backend::transport::{JsonMessagePeer, JsonRpcMailbox};
use yo_core::{BackendFailure, BackendFailureKind, BackendStopHandle};

use super::{
    CodexWarningObserver,
    protocol::{self, CodexWarning, Incoming},
    transport::PeerPoll,
};

pub(super) enum ClientPoll {
    Pending,
    Message(Incoming),
    Closed,
}

pub(super) struct AppServerClient<P> {
    peer: P,
    request_timeout: Duration,
    mailbox: JsonRpcMailbox<Incoming>,
    warning_observer: Option<CodexWarningObserver>,
    notice_thread: Option<String>,
}

pub(super) struct CallResult {
    pub(super) request_id: u64,
    pub(super) result: Value,
}

impl<P: JsonMessagePeer> AppServerClient<P> {
    pub(super) fn new(peer: P, request_timeout: Duration) -> Self {
        Self {
            peer,
            request_timeout,
            mailbox: JsonRpcMailbox::new("Codex app-server"),
            warning_observer: None,
            notice_thread: None,
        }
    }

    pub(super) fn with_warning_observer(
        mut self,
        warning_observer: Option<CodexWarningObserver>,
    ) -> Self {
        self.warning_observer = warning_observer;
        self
    }

    pub(super) fn bind_notice_thread(&mut self, thread: &str) {
        self.notice_thread = Some(thread.to_owned());
    }

    fn observe_warning(&self, incoming: &Incoming, discard_other_threads: bool) -> bool {
        let Incoming::Notification { method, params } = incoming else {
            return false;
        };
        let Some(warning) = CodexWarning::from_notification(method, params) else {
            return false;
        };
        if let Some(target) = warning.thread_id() {
            let Some(bound) = self.notice_thread.as_deref() else {
                return false;
            };
            if target != bound {
                return discard_other_threads;
            }
        }
        if let Some(observer) = &self.warning_observer {
            observer(warning);
            true
        } else {
            false
        }
    }

    pub(super) fn stop_handle(&self) -> BackendStopHandle {
        self.peer.stop_handle()
    }

    pub(super) fn initialize(&mut self) -> Result<protocol::InitializeResult, BackendFailure> {
        let result = self
            .call(
                "initialize",
                json!({
                    "clientInfo": {
                        "name": "yo",
                        "title": "yo",
                        "version": env!("CARGO_PKG_VERSION"),
                    }
                }),
            )?
            .result;
        let initialize = protocol::decode_initialize(result)?;
        if let (Some(observer), Some(warning)) = (
            self.warning_observer.as_ref(),
            initialize.compatibility_warning.as_ref(),
        ) {
            observer(warning.clone());
        }
        self.peer.send(&protocol::initialized_notification())?;
        Ok(initialize)
    }

    pub(super) fn call(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<CallResult, BackendFailure> {
        let id = self.mailbox.next_request_id()?;
        self.peer.send(&protocol::request(id, method, params))?;
        let deadline = Instant::now() + self.request_timeout;

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(BackendFailure::new(
                    BackendFailureKind::Unavailable,
                    format!("timed out waiting for Codex `{method}`"),
                ));
            }
            let message = match self.peer.receive(remaining)? {
                PeerPoll::Message(value) => protocol::classify(value)?,
                PeerPoll::Closed => {
                    return Err(BackendFailure::new(
                        BackendFailureKind::ProcessExit,
                        format!("Codex app-server closed while awaiting `{method}`"),
                    ));
                },
                PeerPoll::Pending => continue,
            };
            if self.observe_warning(&message, false) {
                continue;
            }
            match message {
                Incoming::Response {
                    id: response_id,
                    result,
                } if response_id == id => {
                    return Ok(CallResult {
                        request_id: id,
                        result,
                    });
                },
                Incoming::ResponseError {
                    id: response_id,
                    code,
                    message,
                } if response_id == id => {
                    return Err(BackendFailure::new(
                        failure_kind_for(method),
                        format!("Codex `{method}` rejected ({code}): {message}"),
                    ));
                },
                Incoming::Response {
                    id: response_id, ..
                }
                | Incoming::ResponseError {
                    id: response_id, ..
                } => {
                    return Err(protocol::protocol_failure(format!(
                        "unexpected Codex response id {response_id}; awaited {id}"
                    )));
                },
                message => self.mailbox.push(message)?,
            }
        }
    }

    pub(super) fn respond(&mut self, id: Value, result: Value) -> Result<(), BackendFailure> {
        self.peer.send(&protocol::server_response(id, result))
    }

    pub(super) fn reject(
        &mut self,
        id: Value,
        code: i64,
        message: &str,
    ) -> Result<(), BackendFailure> {
        self.peer.send(&protocol::server_error(id, code, message))
    }

    pub(super) fn poll(&mut self) -> Result<ClientPoll, BackendFailure> {
        for _ in 0..32 {
            let message = if let Some(message) = self.mailbox.pop() {
                message
            } else {
                match self.peer.try_receive()? {
                    PeerPoll::Pending => return Ok(ClientPoll::Pending),
                    PeerPoll::Closed => return Ok(ClientPoll::Closed),
                    PeerPoll::Message(value) => protocol::classify(value)?,
                }
            };
            if matches!(
                message,
                Incoming::Response { .. } | Incoming::ResponseError { .. }
            ) {
                return Err(protocol::protocol_failure(
                    "Codex response arrived without an active request",
                ));
            }
            if self.observe_warning(&message, true) {
                continue;
            }
            return Ok(ClientPoll::Message(message));
        }
        Ok(ClientPoll::Pending)
    }

    pub(super) fn shutdown(&mut self) -> Result<(), BackendFailure> {
        self.peer.shutdown()
    }
}

fn failure_kind_for(method: &str) -> BackendFailureKind {
    match method {
        "initialize" => BackendFailureKind::Initialization,
        "thread/start" | "thread/resume" => BackendFailureKind::Session,
        "turn/start" | "turn/steer" | "turn/interrupt" => BackendFailureKind::Turn,
        _ => BackendFailureKind::Protocol,
    }
}
