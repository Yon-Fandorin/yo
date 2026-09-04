use std::{
    io::{self, Write},
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use yo_backend::transport::{JsonMessagePeer, JsonRpcMailbox};
use yo_core::{BackendFailure, BackendFailureKind, BackendStopHandle};

use super::{
    protocol::{self, Incoming},
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
        self.peer.send(&protocol::initialized_notification())?;
        Ok(initialize)
    }

    /// Preserves the legacy interactive warning route while callers migrate to the typed result.
    pub(super) fn initialize_with_stderr(
        &mut self,
    ) -> Result<protocol::InitializeResult, BackendFailure> {
        let initialize = self.initialize()?;
        let mut stderr = io::stderr().lock();
        write_compatibility_warning(&initialize, &mut stderr);
        Ok(initialize)
    }

    #[cfg(test)]
    pub(super) fn initialize_with_warning_writer<W: Write>(
        &mut self,
        writer: &mut W,
    ) -> Result<protocol::InitializeResult, BackendFailure> {
        let initialize = self.initialize()?;
        write_compatibility_warning(&initialize, writer);
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
        if let Some(message) = self.mailbox.pop() {
            return Ok(ClientPoll::Message(message));
        }
        match self.peer.try_receive()? {
            PeerPoll::Pending => Ok(ClientPoll::Pending),
            PeerPoll::Closed => Ok(ClientPoll::Closed),
            PeerPoll::Message(value) => {
                let message = protocol::classify(value)?;
                if matches!(
                    message,
                    Incoming::Response { .. } | Incoming::ResponseError { .. }
                ) {
                    return Err(protocol::protocol_failure(
                        "Codex response arrived without an active request",
                    ));
                }
                Ok(ClientPoll::Message(message))
            },
        }
    }

    pub(super) fn shutdown(&mut self) -> Result<(), BackendFailure> {
        self.peer.shutdown()
    }
}

fn write_compatibility_warning<W: Write>(initialize: &protocol::InitializeResult, writer: &mut W) {
    if let Some(warning) = &initialize.compatibility_warning {
        let _ = writeln!(writer, "yo: warning: {warning}");
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
