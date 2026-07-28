mod client;
mod config;
mod events;
mod protocol;
mod transport;

#[cfg(test)]
mod tests;

use std::{
    collections::{HashMap, VecDeque},
    num::NonZeroU64,
};

use client::AppServerClient;
pub use config::CodexBackendConfig;
use serde_json::{Value, json};
use transport::{JsonPeer, StdioPeer};

use crate::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef, ActivityResponse,
    AgentBackend, AgentCommand, ApprovalDecision, BackendCapabilities, BackendEvent,
    BackendFailure, BackendFailureKind, BackendPoll, RequestId, SessionId, TurnRef,
};

/// Local stdio adapter for a compatible `codex app-server` process.
pub struct CodexBackend {
    inner: Backend<StdioPeer>,
}

impl CodexBackend {
    /// Spawns Codex, completes the initialize handshake, and verifies the tested protocol line.
    pub fn spawn(config: CodexBackendConfig) -> Result<Self, BackendFailure> {
        if !config.working_directory().is_absolute()
            || !config.working_directory().is_dir()
            || config.request_timeout().is_zero()
        {
            return Err(BackendFailure::new(
                BackendFailureKind::Initialization,
                "Codex requires an existing absolute working directory and a non-zero request timeout",
            ));
        }
        let cwd = config
            .working_directory()
            .to_str()
            .ok_or_else(|| {
                BackendFailure::new(
                    BackendFailureKind::Initialization,
                    "Codex working directory is not valid UTF-8",
                )
            })?
            .to_owned();
        let peer = StdioPeer::spawn(&config)?;
        let client = AppServerClient::initialize(peer, config.request_timeout())?;
        Ok(Self {
            inner: Backend::new(client, cwd),
        })
    }
}

impl AgentBackend for CodexBackend {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    fn execute_command(&mut self, command: AgentCommand) -> Result<(), BackendFailure> {
        self.inner.execute_command(command)
    }

    fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        self.inner.poll_event()
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        self.inner.shutdown()
    }
}

struct SessionBinding {
    yo: SessionId,
    codex: String,
}

struct ItemBinding {
    activity: ActivityRef,
}

struct ApprovalBinding {
    wire_id: Value,
    request_activity: ActivityRef,
}

struct Backend<P> {
    client: AppServerClient<P>,
    cwd: String,
    session: Option<SessionBinding>,
    turns: HashMap<TurnRef, String>,
    wire_turns: HashMap<String, TurnRef>,
    items: HashMap<String, ItemBinding>,
    approvals: HashMap<ActivityRequestRef, ApprovalBinding>,
    wire_approvals: HashMap<String, ActivityRequestRef>,
    turn_errors: HashMap<String, String>,
    pending_events: VecDeque<BackendEvent>,
    next_activity_id: u64,
    next_request_id: u64,
}

impl<P: JsonPeer> Backend<P> {
    fn new(client: AppServerClient<P>, cwd: String) -> Self {
        Self {
            client,
            cwd,
            session: None,
            turns: HashMap::new(),
            wire_turns: HashMap::new(),
            items: HashMap::new(),
            approvals: HashMap::new(),
            wire_approvals: HashMap::new(),
            turn_errors: HashMap::new(),
            pending_events: VecDeque::new(),
            next_activity_id: 1,
            next_request_id: 1,
        }
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::none().with_steer()
    }

    fn execute_command(&mut self, command: AgentCommand) -> Result<(), BackendFailure> {
        match command {
            AgentCommand::CreateSession { session_id } => self.create_session(session_id),
            AgentCommand::StartTurn { turn, input } => {
                let thread_id = self.thread_id(turn.session_id())?.to_owned();
                let result = self.client.call(
                    "turn/start",
                    json!({
                        "threadId": thread_id,
                        "input": [{ "type": "text", "text": input.into_string() }],
                        "cwd": self.cwd,
                    }),
                )?;
                let wire_turn = protocol::string_at(&result, &["turn", "id"])?.to_owned();
                self.turns.insert(turn, wire_turn.clone());
                self.wire_turns.insert(wire_turn, turn);
                Ok(())
            },
            AgentCommand::SteerTurn { turn, input } => {
                let thread_id = self.thread_id(turn.session_id())?.to_owned();
                let turn_id = self.turn_id(turn)?.to_owned();
                let result = self.client.call(
                    "turn/steer",
                    json!({
                        "threadId": thread_id,
                        "expectedTurnId": &turn_id,
                        "input": [{ "type": "text", "text": input.into_string() }],
                    }),
                )?;
                let accepted = protocol::string_at(&result, &["turnId"])?;
                if accepted != turn_id {
                    return Err(protocol::protocol_failure(format!(
                        "Codex steer accepted Turn `{accepted}` instead of `{turn_id}`"
                    )));
                }
                Ok(())
            },
            AgentCommand::InterruptTurn { turn } => {
                let thread_id = self.thread_id(turn.session_id())?.to_owned();
                let turn_id = self.turn_id(turn)?.to_owned();
                self.client.call(
                    "turn/interrupt",
                    json!({ "threadId": thread_id, "turnId": turn_id }),
                )?;
                Ok(())
            },
            AgentCommand::RespondToActivity { request, response } => {
                self.respond_to_activity(request, response)
            },
        }
    }

    fn create_session(&mut self, session_id: SessionId) -> Result<(), BackendFailure> {
        let result = self.client.call(
            "thread/start",
            json!({
                "cwd": self.cwd,
                "serviceName": "yo",
            }),
        )?;
        let thread_id = protocol::string_at(&result, &["thread", "id"])?.to_owned();
        self.session = Some(SessionBinding {
            yo: session_id,
            codex: thread_id,
        });
        Ok(())
    }

    fn respond_to_activity(
        &mut self,
        request: ActivityRequestRef,
        response: ActivityResponse,
    ) -> Result<(), BackendFailure> {
        let approval = self.approvals.get(&request).ok_or_else(|| {
            protocol::protocol_failure("approval response has no matching Codex request")
        })?;
        let decision = match response {
            ActivityResponse::Approval(ApprovalDecision::Approved) => "accept",
            ActivityResponse::Approval(ApprovalDecision::Declined) => "decline",
            ActivityResponse::UserInput(_) => {
                return Err(BackendFailure::new(
                    BackendFailureKind::Unsupported,
                    "Codex user-input responses are not enabled in the initial adapter",
                ));
            },
        };
        let wire_id = approval.wire_id.clone();
        let response_activity = self.next_activity(request.activity().turn())?;
        self.client
            .respond(wire_id, json!({ "decision": decision }))?;
        self.pending_events
            .push_back(BackendEvent::ActivityStarted {
                activity: response_activity,
                kind: ActivityKind::ApprovalResponse {
                    request_id: request.request_id(),
                },
            });
        self.pending_events
            .push_back(BackendEvent::ActivityFinished {
                activity: response_activity,
                outcome: ActivityOutcome::Completed,
            });
        Ok(())
    }

    fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        if let Some(event) = self.pending_events.pop_front() {
            return Ok(BackendPoll::Event(event));
        }
        self.poll_client_message()
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        self.client.shutdown()
    }

    fn thread_id(&self, session_id: SessionId) -> Result<&str, BackendFailure> {
        self.session
            .as_ref()
            .filter(|binding| binding.yo == session_id)
            .map(|binding| binding.codex.as_str())
            .ok_or_else(|| protocol::protocol_failure("Codex Session binding was not found"))
    }

    fn turn_id(&self, turn: TurnRef) -> Result<&str, BackendFailure> {
        self.turns
            .get(&turn)
            .map(String::as_str)
            .ok_or_else(|| protocol::protocol_failure("Codex Turn binding was not found"))
    }

    fn next_activity(&mut self, turn: TurnRef) -> Result<ActivityRef, BackendFailure> {
        let id = NonZeroU64::new(self.next_activity_id)
            .map(ActivityId::new)
            .ok_or_else(|| protocol::protocol_failure("Codex Activity id space was exhausted"))?;
        self.next_activity_id = self
            .next_activity_id
            .checked_add(1)
            .ok_or_else(|| protocol::protocol_failure("Codex Activity id space was exhausted"))?;
        Ok(ActivityRef::new(turn, id))
    }

    fn next_request(&mut self) -> Result<RequestId, BackendFailure> {
        let id = NonZeroU64::new(self.next_request_id)
            .map(RequestId::new)
            .ok_or_else(|| protocol::protocol_failure("Codex request id space was exhausted"))?;
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .ok_or_else(|| protocol::protocol_failure("Codex request id space was exhausted"))?;
        Ok(id)
    }
}

impl<P: JsonPeer> AgentBackend for Backend<P> {
    fn capabilities(&self) -> BackendCapabilities {
        self.capabilities()
    }

    fn execute_command(&mut self, command: AgentCommand) -> Result<(), BackendFailure> {
        self.execute_command(command)
    }

    fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        self.poll_event()
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        self.shutdown()
    }
}
