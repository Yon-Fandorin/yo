use std::time::{Duration, Instant};

use serde_json::Value;
use yo_backend::transport::JsonRpcMailbox;
use yo_core::{BackendFailure, BackendFailureKind, BackendStopHandle};

use super::{
    protocol::{self, Incoming},
    transport::{JsonPeer, PeerPoll},
};

pub(super) enum ClientPoll {
    Pending,
    Message(Incoming),
    Closed,
}

pub(super) struct AcpClient<P> {
    peer: P,
    request_timeout: Duration,
    mailbox: JsonRpcMailbox<Incoming>,
}

pub(super) struct CallResult {
    pub(super) result: Value,
}

impl<P: JsonPeer> AcpClient<P> {
    pub(super) fn new(peer: P, request_timeout: Duration) -> Self {
        Self {
            peer,
            request_timeout,
            mailbox: JsonRpcMailbox::new("Grok ACP"),
        }
    }

    pub(super) fn stop_handle(&self) -> BackendStopHandle {
        self.peer.stop_handle()
    }

    pub(super) fn call(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<CallResult, BackendFailure> {
        let id = self.begin_request(method, params)?;
        let deadline = Instant::now() + self.request_timeout;
        loop {
            let message = self.receive_until(deadline, method)?;
            match message {
                Incoming::Response {
                    id: response_id,
                    result,
                } if response_id == id => {
                    return Ok(CallResult { result });
                },
                Incoming::ResponseError {
                    id: response_id,
                    code,
                    message,
                } if response_id == id => {
                    return Err(BackendFailure::new(
                        failure_kind_for(method),
                        format!("Grok ACP `{method}` rejected ({code}): {message}"),
                    ));
                },
                Incoming::Response {
                    id: response_id, ..
                }
                | Incoming::ResponseError {
                    id: response_id, ..
                } => {
                    return Err(protocol::protocol_failure(format!(
                        "unexpected Grok ACP response id {response_id}; awaited {id}"
                    )));
                },
                message => self.queue(message)?,
            }
        }
    }

    pub(super) fn begin_prompt(
        &mut self,
        params: Value,
        session_id: &str,
    ) -> Result<u64, BackendFailure> {
        let id = self.begin_request("session/prompt", params)?;
        let deadline = Instant::now() + self.request_timeout;
        loop {
            let message = self.receive_until(deadline, "session/prompt acceptance")?;
            match &message {
                Incoming::Response {
                    id: response_id, ..
                } if *response_id == id => {
                    self.queue(message)?;
                    return Ok(id);
                },
                Incoming::ResponseError {
                    id: response_id,
                    code,
                    message,
                } if *response_id == id => {
                    return Err(BackendFailure::new(
                        BackendFailureKind::Turn,
                        format!("Grok ACP `session/prompt` rejected ({code}): {message}"),
                    ));
                },
                Incoming::Response {
                    id: response_id, ..
                }
                | Incoming::ResponseError {
                    id: response_id, ..
                } => {
                    return Err(protocol::protocol_failure(format!(
                        "unexpected Grok ACP response id {response_id} while awaiting prompt {id}"
                    )));
                },
                Incoming::Notification { method, params }
                    if method == "session/update" && session_matches(params, session_id) =>
                {
                    self.queue(message)?;
                    return Ok(id);
                },
                Incoming::ServerRequest { method, params, .. }
                    if method == "session/request_permission"
                        && session_matches(params, session_id) =>
                {
                    self.queue(message)?;
                    return Ok(id);
                },
                _ => self.queue(message)?,
            }
        }
    }

    pub(super) fn notify(&mut self, method: &str, params: Value) -> Result<(), BackendFailure> {
        self.peer.send(&protocol::notification(method, params))
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

    pub(super) fn discard_session_updates(&mut self, session_id: &str) {
        self.mailbox.retain(|message| {
            !matches!(
                message,
                Incoming::Notification { method, params }
                    if method == "session/update" && session_matches(params, session_id)
            )
        });
    }

    pub(super) fn poll(
        &mut self,
        active_prompt: Option<u64>,
    ) -> Result<ClientPoll, BackendFailure> {
        let message = if let Some(message) = self.mailbox.pop() {
            message
        } else {
            match self.peer.try_receive()? {
                PeerPoll::Pending => return Ok(ClientPoll::Pending),
                PeerPoll::Closed => return Ok(ClientPoll::Closed),
                PeerPoll::Message(value) => protocol::classify(value)?,
            }
        };
        match &message {
            Incoming::Response { id, .. } if Some(*id) == active_prompt => {},
            Incoming::ResponseError { id, code, message } if Some(*id) == active_prompt => {
                return Err(BackendFailure::new(
                    BackendFailureKind::Turn,
                    format!("Grok ACP prompt failed ({code}): {message}"),
                ));
            },
            Incoming::Response { id, .. } | Incoming::ResponseError { id, .. } => {
                return Err(protocol::protocol_failure(format!(
                    "Grok ACP response {id} arrived without its active request"
                )));
            },
            _ => {},
        }
        Ok(ClientPoll::Message(message))
    }

    pub(super) fn shutdown(&mut self) -> Result<(), BackendFailure> {
        self.peer.shutdown()
    }

    fn begin_request(&mut self, method: &str, params: Value) -> Result<u64, BackendFailure> {
        let id = self.mailbox.next_request_id()?;
        self.peer.send(&protocol::request(id, method, params))?;
        Ok(id)
    }

    fn receive_until(
        &mut self,
        deadline: Instant,
        operation: &str,
    ) -> Result<Incoming, BackendFailure> {
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(BackendFailure::new(
                    BackendFailureKind::Unavailable,
                    format!("timed out waiting for Grok ACP {operation}"),
                ));
            }
            match self.peer.receive(remaining)? {
                PeerPoll::Message(value) => return protocol::classify(value),
                PeerPoll::Closed => {
                    return Err(BackendFailure::new(
                        BackendFailureKind::ProcessExit,
                        format!("Grok ACP closed while awaiting {operation}"),
                    ));
                },
                PeerPoll::Pending => {},
            }
        }
    }

    fn queue(&mut self, message: Incoming) -> Result<(), BackendFailure> {
        self.mailbox.push(message)
    }
}

fn session_matches(params: &Value, session_id: &str) -> bool {
    params.get("sessionId").and_then(Value::as_str) == Some(session_id)
}

fn failure_kind_for(method: &str) -> BackendFailureKind {
    match method {
        "initialize" | "authenticate" => BackendFailureKind::Initialization,
        "session/new" | "session/load" => BackendFailureKind::Session,
        "session/prompt" => BackendFailureKind::Turn,
        _ => BackendFailureKind::Protocol,
    }
}
