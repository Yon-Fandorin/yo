use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use super::{
    protocol::{self, Incoming},
    transport::{JsonPeer, PeerPoll},
};
use crate::{BackendFailure, BackendFailureKind, BackendStopHandle};

pub(super) enum ClientPoll {
    Pending,
    Message(Incoming),
    Closed,
}

pub(super) struct AppServerClient<P> {
    peer: P,
    request_timeout: Duration,
    next_request_id: u64,
    pending: VecDeque<Incoming>,
}

impl<P: JsonPeer> AppServerClient<P> {
    pub(super) fn new(peer: P, request_timeout: Duration) -> Self {
        Self {
            peer,
            request_timeout,
            next_request_id: 1,
            pending: VecDeque::new(),
        }
    }

    pub(super) fn stop_handle(&self) -> BackendStopHandle {
        self.peer.stop_handle()
    }

    pub(super) fn initialize(&mut self) -> Result<(), BackendFailure> {
        let result = self.call(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "yo",
                    "title": "yo",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }),
        )?;
        protocol::decode_initialize(result)?;
        self.peer.send(&protocol::initialized_notification())
    }

    pub(super) fn call(&mut self, method: &str, params: Value) -> Result<Value, BackendFailure> {
        let id = self.next_request_id;
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .ok_or_else(|| protocol::protocol_failure("Codex request id space was exhausted"))?;
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
                } if response_id == id => return Ok(result),
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
                message => {
                    const MAX_PENDING_MESSAGES: usize = 1024;
                    if self.pending.len() == MAX_PENDING_MESSAGES {
                        return Err(BackendFailure::new(
                            BackendFailureKind::Unavailable,
                            "Codex event backlog filled while awaiting a response",
                        ));
                    }
                    self.pending.push_back(message);
                },
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
        if let Some(message) = self.pending.pop_front() {
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

fn failure_kind_for(method: &str) -> BackendFailureKind {
    match method {
        "initialize" => BackendFailureKind::Initialization,
        "thread/start" => BackendFailureKind::Session,
        "turn/start" | "turn/steer" | "turn/interrupt" => BackendFailureKind::Turn,
        _ => BackendFailureKind::Protocol,
    }
}
