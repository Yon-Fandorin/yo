mod client;
mod config;
mod events;
mod protocol;
mod skill_catalog;
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
pub use skill_catalog::CodexSkillReferenceProvider;
use transport::{JsonPeer, StdioPeer};

use crate::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef, ActivityResponse,
    AgentBackend, AgentCommand, ApprovalDecision, BackendBindingEvidence, BackendCapabilities,
    BackendCommandEvidence, BackendEvent, BackendFailure, BackendFailureKind, BackendIdentity,
    BackendPoll, BackendRequestEvidence, BackendResumeTarget, BackendStopHandle,
    ContinuationStrategy, RequestId, SessionId, TurnRef,
};

/// Local stdio adapter for a compatible `codex app-server` process.
pub struct CodexBackend {
    inner: Backend<StdioPeer>,
}

impl CodexBackend {
    /// Spawns Codex and prepares the cancellable transport.
    ///
    /// The initialize handshake is deferred to `CreateSession` so the runtime owner can cancel it
    /// through [`AgentBackend::stop_handle`].
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
        let client = AppServerClient::new(peer, config.request_timeout());
        Ok(Self {
            inner: Backend::new_uninitialized(client, cwd),
        })
    }
}

impl AgentBackend for CodexBackend {
    fn stop_handle(&self) -> BackendStopHandle {
        self.inner.client.stop_handle()
    }

    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    fn resume_session(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        self.inner.resume_session(target)
    }

    fn execute_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
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

#[derive(Clone, Copy)]
struct WireTurnBinding {
    turn: TurnRef,
    interrupted: bool,
}

struct Backend<P> {
    client: AppServerClient<P>,
    initialized: bool,
    backend_version: Option<String>,
    cwd: String,
    session: Option<SessionBinding>,
    turns: HashMap<TurnRef, String>,
    wire_turns: HashMap<String, WireTurnBinding>,
    items: HashMap<String, ItemBinding>,
    approvals: HashMap<ActivityRequestRef, ApprovalBinding>,
    wire_approvals: HashMap<String, ActivityRequestRef>,
    turn_errors: HashMap<String, String>,
    pending_events: VecDeque<BackendEvent>,
    next_activity_id: u64,
    next_request_id: u64,
}

impl<P: JsonPeer> Backend<P> {
    fn new_uninitialized(client: AppServerClient<P>, cwd: String) -> Self {
        Self {
            client,
            initialized: false,
            backend_version: None,
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

    fn execute_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        match command {
            AgentCommand::CreateSession { session_id } => self.create_session(session_id),
            AgentCommand::StartTurn { turn, input } => {
                let thread_id = self.thread_id(turn.session_id())?.to_owned();
                let call = self.client.call(
                    "turn/start",
                    json!({
                        "threadId": thread_id,
                        "input": [{ "type": "text", "text": input.into_string() }],
                        "cwd": self.cwd,
                    }),
                )?;
                let wire_turn = protocol::string_at(&call.result, &["turn", "id"])?.to_owned();
                self.turns.insert(turn, wire_turn.clone());
                self.wire_turns.insert(
                    wire_turn.clone(),
                    WireTurnBinding {
                        turn,
                        interrupted: false,
                    },
                );
                Ok(BackendCommandEvidence::RequestAccepted(
                    BackendRequestEvidence::new(
                        "codex.app-server/turn-start/v1",
                        json_rpc_identity(call.request_id),
                        accepted_request_identity(call.request_id, &wire_turn),
                    ),
                ))
            },
            AgentCommand::SteerTurn { turn, input } => {
                let thread_id = self.thread_id(turn.session_id())?.to_owned();
                let turn_id = self.turn_id(turn)?.to_owned();
                let call = self.client.call(
                    "turn/steer",
                    json!({
                        "threadId": thread_id,
                        "expectedTurnId": &turn_id,
                        "input": [{ "type": "text", "text": input.into_string() }],
                    }),
                )?;
                let accepted = protocol::string_at(&call.result, &["turnId"])?;
                if accepted != turn_id {
                    return Err(protocol::protocol_failure(format!(
                        "Codex steer accepted Turn `{accepted}` instead of `{turn_id}`"
                    )));
                }
                Ok(BackendCommandEvidence::RequestAccepted(
                    BackendRequestEvidence::new(
                        "codex.app-server/turn-steer/v1",
                        json_rpc_identity(call.request_id),
                        accepted_request_identity(call.request_id, accepted),
                    ),
                ))
            },
            AgentCommand::InterruptTurn { turn } => {
                let thread_id = self.thread_id(turn.session_id())?.to_owned();
                let turn_id = self.turn_id(turn)?.to_owned();
                self.client.call(
                    "turn/interrupt",
                    json!({ "threadId": thread_id, "turnId": turn_id }),
                )?;
                Ok(BackendCommandEvidence::None)
            },
            AgentCommand::RespondToActivity { request, response } => {
                self.respond_to_activity(request, response)
            },
        }
    }

    fn create_session(
        &mut self,
        session_id: SessionId,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        self.initialize()?;
        let result = self
            .client
            .call(
                "thread/start",
                json!({
                    "cwd": self.cwd,
                    "serviceName": "yo",
                }),
            )?
            .result;
        let thread_id = protocol::string_at(&result, &["thread", "id"])?.to_owned();
        let backend_session_id = protocol::string_at(&result, &["thread", "sessionId"])?;
        let model = protocol::string_at(&result, &["model"])?;
        let model_provider = protocol::string_at(&result, &["modelProvider"])?;
        let backend_version = self.backend_version.clone().ok_or_else(|| {
            protocol::protocol_failure("Codex backend version was not retained after initialize")
        })?;
        let binding_value = json!({
            "sessionId": backend_session_id,
            "threadId": thread_id,
        })
        .to_string();
        let model_value = json!({
            "model": model,
            "provider": model_provider,
        })
        .to_string();
        self.session = Some(SessionBinding {
            yo: session_id,
            codex: thread_id.clone(),
        });
        Ok(BackendCommandEvidence::BindingOpened(
            BackendBindingEvidence::new(
                "codex-app-server",
                backend_version,
                BackendIdentity::new("codex.app-server/thread-binding/v1", binding_value),
                BackendIdentity::new("codex.app-server/model-and-provider/v1", model_value),
                BackendIdentity::new("codex.app-server/thread-locator/v1", thread_id),
                ContinuationStrategy::BackendManagedState,
            ),
        ))
    }

    fn initialize(&mut self) -> Result<(), BackendFailure> {
        if !self.initialized {
            let initialize = self.client.initialize()?;
            self.backend_version = Some(initialize.user_agent);
            self.initialized = true;
        }
        Ok(())
    }

    fn resume_session(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        if target.binding().backend_kind() != "codex-app-server" {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                format!(
                    "Codex cannot resume backend kind `{}`",
                    target.binding().backend_kind()
                ),
            ));
        }
        let locator = target.binding().session_locator();
        if locator.schema() != "codex.app-server/thread-locator/v1" {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                format!("unsupported Codex Session locator `{}`", locator.schema()),
            ));
        }
        let thread_id = locator.value();
        self.initialize()?;
        let result = self
            .client
            .call("thread/resume", json!({ "threadId": thread_id }))?
            .result;
        let resumed_thread = protocol::string_at(&result, &["thread", "id"])?;
        let backend_session_id = protocol::string_at(&result, &["thread", "sessionId"])?;
        let model = protocol::string_at(&result, &["model"])?;
        let model_provider = protocol::string_at(&result, &["modelProvider"])?;
        let binding_identity = BackendIdentity::new(
            "codex.app-server/thread-binding/v1",
            json!({
                "sessionId": backend_session_id,
                "threadId": resumed_thread,
            })
            .to_string(),
        );
        let model_identity = BackendIdentity::new(
            "codex.app-server/model-and-provider/v1",
            json!({ "model": model, "provider": model_provider }).to_string(),
        );
        let evidence = BackendBindingEvidence::new(
            "codex-app-server",
            self.backend_version.clone().ok_or_else(|| {
                protocol::protocol_failure(
                    "Codex backend version was not retained after resume initialize",
                )
            })?,
            binding_identity,
            model_identity,
            BackendIdentity::new("codex.app-server/thread-locator/v1", resumed_thread),
            ContinuationStrategy::BackendManagedState,
        );
        if !target.binding().same_resume_identity(&evidence) {
            return Err(BackendFailure::new(
                BackendFailureKind::Session,
                "Codex resumed a binding whose thread, Session, model, or provider identity differs from the durable Continuation Anchor",
            ));
        }
        self.session = Some(SessionBinding {
            yo: target.session_id(),
            codex: resumed_thread.to_owned(),
        });
        Ok(evidence)
    }

    fn respond_to_activity(
        &mut self,
        request: ActivityRequestRef,
        response: ActivityResponse,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
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
        Ok(BackendCommandEvidence::None)
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
    fn stop_handle(&self) -> BackendStopHandle {
        self.client.stop_handle()
    }

    fn capabilities(&self) -> BackendCapabilities {
        self.capabilities()
    }

    fn resume_session(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        self.resume_session(target)
    }

    fn execute_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        self.execute_command(command)
    }

    fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        self.poll_event()
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        self.shutdown()
    }
}

fn json_rpc_identity(request_id: u64) -> BackendIdentity {
    BackendIdentity::new(
        "codex.app-server/json-rpc-request/v1",
        request_id.to_string(),
    )
}

fn accepted_request_identity(request_id: u64, turn_id: &str) -> BackendIdentity {
    BackendIdentity::new(
        "codex.app-server/accepted-request/v1",
        json!({ "jsonRpcId": request_id, "turnId": turn_id }).to_string(),
    )
}
