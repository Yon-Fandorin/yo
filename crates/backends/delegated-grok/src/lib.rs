mod client;
mod config;
mod events;
mod protocol;
mod transport;

#[cfg(test)]
mod tests;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    num::NonZeroU64,
};

use client::AcpClient;
pub use config::GrokBackendConfig;
use serde_json::{Value, json};
use transport::{JsonPeer, StdioPeer};
use yo_backend::BackendAdapter;
use yo_core::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef, ActivityResponse,
    AgentCommand, ApprovalDecision, BackendBindingEvidence, BackendCapabilities,
    BackendCommandEvidence, BackendEvent, BackendFailure, BackendFailureKind, BackendIdentity,
    BackendPoll, BackendRequestEvidence, BackendResumeTarget, BackendStopHandle,
    ContinuationStrategy, RequestId, SessionId, TurnRef,
};

pub const HOST_ID: &str = "grok";
pub const BACKEND_KIND: &str = "grok-build-acp";

/// Local stdio adapter for the Grok Build Agent Client Protocol service.
pub struct GrokBackend {
    inner: Backend<StdioPeer>,
}

impl GrokBackend {
    /// Spawns `grok agent stdio`; initialization and cached-token authentication are deferred.
    pub fn spawn(config: GrokBackendConfig) -> Result<Self, BackendFailure> {
        if !config.working_directory().is_absolute()
            || !config.working_directory().is_dir()
            || config.request_timeout().is_zero()
        {
            return Err(BackendFailure::new(
                BackendFailureKind::Initialization,
                "Grok requires an existing absolute working directory and a non-zero request timeout",
            ));
        }
        let cwd = config
            .working_directory()
            .to_str()
            .ok_or_else(|| {
                BackendFailure::new(
                    BackendFailureKind::Initialization,
                    "Grok working directory is not valid UTF-8",
                )
            })?
            .to_owned();
        let peer = StdioPeer::spawn(&config)?;
        let client = AcpClient::new(peer, config.request_timeout());
        Ok(Self {
            inner: Backend::new_uninitialized(client, cwd),
        })
    }

    /// Verifies ACP compatibility and the cached Grok login without creating a Session.
    pub fn verify(config: GrokBackendConfig) -> Result<(), BackendFailure> {
        let mut backend = Self::spawn(config)?;
        let verification = backend.inner.verify();
        let cleanup = backend.inner.shutdown();
        combine_with_cleanup(verification, cleanup)
    }
}

impl BackendAdapter for GrokBackend {
    type Command = AgentCommand;
    type Event = BackendEvent;
    type ResumeTarget = BackendResumeTarget;

    fn stop_handle(&self) -> BackendStopHandle {
        self.inner.client.stop_handle()
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::none()
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
    grok: String,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum MessageChannel {
    Agent,
    Thought,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct MessageKey {
    channel: MessageChannel,
    message_id: Option<String>,
}

struct MessageBinding {
    activity: ActivityRef,
}

struct ToolBinding {
    activity: ActivityRef,
    finished: bool,
}

#[derive(Clone)]
struct ApprovalBinding {
    wire_id: Value,
    activity: ActivityRef,
    allow_option: String,
    reject_option: String,
}

struct PromptBinding {
    request_id: u64,
    turn: TurnRef,
    interrupt_requested: bool,
}

struct Backend<P> {
    client: AcpClient<P>,
    initialized: bool,
    backend_version: Option<String>,
    load_session: bool,
    cwd: String,
    session: Option<SessionBinding>,
    prompt: Option<PromptBinding>,
    messages: HashMap<MessageKey, MessageBinding>,
    tools: HashMap<String, ToolBinding>,
    seen_tool_ids: HashSet<String>,
    approvals: HashMap<ActivityRequestRef, ApprovalBinding>,
    wire_approvals: HashMap<String, ActivityRequestRef>,
    pending_events: VecDeque<BackendEvent>,
    next_activity_id: u64,
    next_request_id: u64,
}

impl<P: JsonPeer> Backend<P> {
    const MAX_ACTIVE_ACTIVITIES: usize = 1024;
    const MAX_SESSION_TOOL_IDS: usize = 4096;

    fn new_uninitialized(client: AcpClient<P>, cwd: String) -> Self {
        Self {
            client,
            initialized: false,
            backend_version: None,
            load_session: false,
            cwd,
            session: None,
            prompt: None,
            messages: HashMap::new(),
            tools: HashMap::new(),
            seen_tool_ids: HashSet::new(),
            approvals: HashMap::new(),
            wire_approvals: HashMap::new(),
            pending_events: VecDeque::new(),
            next_activity_id: 1,
            next_request_id: 1,
        }
    }

    fn initialize(&mut self) -> Result<(), BackendFailure> {
        if self.initialized {
            return Ok(());
        }
        let result = self
            .client
            .call(
                "initialize",
                json!({
                    "protocolVersion": protocol::PROTOCOL_VERSION,
                    "clientCapabilities": {},
                    "clientInfo": {
                        "name": "yo",
                        "title": "yo",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                }),
            )?
            .result;
        let initialized = protocol::decode_initialize(result)?;
        if !initialized
            .auth_methods
            .iter()
            .any(|method| method == "cached_token")
        {
            return Err(BackendFailure::new(
                BackendFailureKind::Initialization,
                "Grok has no cached login; run `grok login` before using `host:grok`",
            ));
        }
        self.client
            .call(
                "authenticate",
                json!({
                    "methodId": "cached_token",
                    "_meta": { "headless": true },
                }),
            )
            .map_err(|error| {
                BackendFailure::new(
                    error.kind(),
                    format!(
                        "Grok cached login authentication failed; run `grok login` and retry: {}",
                        error.message()
                    ),
                )
            })?;
        self.backend_version = Some(format!(
            "{}/{}",
            initialized.agent_name, initialized.agent_version
        ));
        self.load_session = initialized.load_session;
        self.initialized = true;
        Ok(())
    }

    fn verify(&mut self) -> Result<(), BackendFailure> {
        self.initialize()
    }

    fn execute_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        match command {
            AgentCommand::CreateSession { session_id } => self.create_session(session_id),
            AgentCommand::StartTurn { turn, input } => self.start_turn(turn, input.into_string()),
            AgentCommand::SteerTurn { .. } => Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                "Grok ACP v1 does not support steering an active Turn",
            )),
            AgentCommand::InterruptTurn { turn } => self.interrupt_turn(turn),
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
            .call("session/new", json!({ "cwd": self.cwd, "mcpServers": [] }))?
            .result;
        let grok_session = protocol::string_at(&result, &["sessionId"])?;
        validate_session_id(grok_session)?;
        let evidence = self.binding_evidence(grok_session)?;
        self.session = Some(SessionBinding {
            yo: session_id,
            grok: grok_session.to_owned(),
        });
        Ok(BackendCommandEvidence::BindingOpened(evidence))
    }

    fn resume_session(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        self.resume_binding(target.session_id(), target.binding())
    }

    fn resume_binding(
        &mut self,
        session_id: SessionId,
        binding: &BackendBindingEvidence,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        if binding.backend_kind() != BACKEND_KIND {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                format!(
                    "Grok cannot resume backend kind `{}`",
                    binding.backend_kind()
                ),
            ));
        }
        let locator = binding.session_locator();
        if locator.schema() != "grok.acp/session-locator/v1" {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                format!("unsupported Grok Session locator `{}`", locator.schema()),
            ));
        }
        validate_session_id(locator.value())?;
        self.initialize()?;
        if !self.load_session {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                "this Grok ACP agent does not advertise session/load",
            ));
        }
        self.client.call(
            "session/load",
            json!({
                "sessionId": locator.value(),
                "cwd": self.cwd,
                "mcpServers": [],
            }),
        )?;
        self.client.discard_session_updates(locator.value());
        let evidence = self.binding_evidence(locator.value())?;
        if !binding.same_resume_identity(&evidence) {
            return Err(BackendFailure::new(
                BackendFailureKind::Session,
                "Grok loaded a Session whose durable identity differs from its Continuation Anchor",
            ));
        }
        self.session = Some(SessionBinding {
            yo: session_id,
            grok: locator.value().to_owned(),
        });
        Ok(evidence)
    }

    fn start_turn(
        &mut self,
        turn: TurnRef,
        input: String,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        if self.prompt.is_some() {
            return Err(protocol::protocol_failure(
                "Grok ACP already has an active prompt",
            ));
        }
        let session_id = self.session_id(turn.session_id())?.to_owned();
        let request_id = self.client.begin_prompt(
            json!({
                "sessionId": session_id,
                "prompt": [{ "type": "text", "text": input }],
            }),
            &session_id,
        )?;
        self.prompt = Some(PromptBinding {
            request_id,
            turn,
            interrupt_requested: false,
        });
        Ok(BackendCommandEvidence::RequestAccepted(
            BackendRequestEvidence::new(
                "grok.acp/session-prompt/v1",
                BackendIdentity::new("grok.acp/json-rpc-request/v1", request_id.to_string()),
                BackendIdentity::new(
                    "grok.acp/accepted-prompt/v1",
                    json!({ "jsonRpcId": request_id, "sessionId": session_id }).to_string(),
                ),
            ),
        ))
    }

    fn interrupt_turn(&mut self, turn: TurnRef) -> Result<BackendCommandEvidence, BackendFailure> {
        let session_id = self.session_id(turn.session_id())?.to_owned();
        let prompt = self
            .prompt
            .as_mut()
            .filter(|prompt| prompt.turn == turn)
            .ok_or_else(|| protocol::protocol_failure("Grok active prompt was not found"))?;
        self.client
            .notify("session/cancel", json!({ "sessionId": session_id }))?;
        prompt.interrupt_requested = true;
        let approvals = self.approvals.drain().collect::<Vec<_>>();
        self.wire_approvals.clear();
        for (_, approval) in approvals {
            self.client.respond(
                approval.wire_id,
                json!({ "outcome": { "outcome": "cancelled" } }),
            )?;
            self.pending_events
                .push_back(BackendEvent::ActivityFinished {
                    activity: approval.activity,
                    outcome: ActivityOutcome::Interrupted,
                });
        }
        Ok(BackendCommandEvidence::None)
    }

    fn respond_to_activity(
        &mut self,
        request: ActivityRequestRef,
        response: ActivityResponse,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        let approval = self.approvals.get(&request).cloned().ok_or_else(|| {
            protocol::protocol_failure("approval response has no matching Grok request")
        })?;
        let option_id = match response {
            ActivityResponse::Approval(ApprovalDecision::Approved) => approval.allow_option,
            ActivityResponse::Approval(ApprovalDecision::Declined) => approval.reject_option,
            ActivityResponse::UserInput(_) => {
                return Err(BackendFailure::new(
                    BackendFailureKind::Unsupported,
                    "Grok user-input responses are not enabled in the ACP adapter",
                ));
            },
        };
        self.client.respond(
            approval.wire_id,
            json!({
                "outcome": { "outcome": "selected", "optionId": option_id }
            }),
        )?;
        self.approvals.remove(&request);
        self.wire_approvals.retain(|_, bound| *bound != request);
        self.pending_events
            .push_back(BackendEvent::ActivityFinished {
                activity: approval.activity,
                outcome: ActivityOutcome::Completed,
            });
        let response_activity = self.next_activity(request.activity().turn())?;
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

    fn binding_evidence(
        &self,
        grok_session: &str,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        let backend_version = self.backend_version.clone().ok_or_else(|| {
            protocol::protocol_failure("Grok backend version was not retained after initialize")
        })?;
        Ok(BackendBindingEvidence::new(
            BACKEND_KIND,
            backend_version,
            BackendIdentity::new(
                "grok.acp/session-binding/v1",
                json!({ "sessionId": grok_session }).to_string(),
            ),
            BackendIdentity::new("grok.build/model-selection/v1", "backend-managed"),
            BackendIdentity::new("grok.acp/session-locator/v1", grok_session),
            ContinuationStrategy::BackendManagedState,
        ))
    }

    fn session_id(&self, session_id: SessionId) -> Result<&str, BackendFailure> {
        self.session
            .as_ref()
            .filter(|binding| binding.yo == session_id)
            .map(|binding| binding.grok.as_str())
            .ok_or_else(|| protocol::protocol_failure("Grok ACP Session binding was not found"))
    }

    fn active_turn(&self) -> Result<TurnRef, BackendFailure> {
        self.prompt
            .as_ref()
            .map(|prompt| prompt.turn)
            .ok_or_else(|| protocol::protocol_failure("Grok ACP update has no active Turn"))
    }

    fn ensure_activity_capacity(&self) -> Result<(), BackendFailure> {
        let active = self
            .messages
            .len()
            .checked_add(self.tools.len())
            .and_then(|count| count.checked_add(self.approvals.len()))
            .ok_or_else(|| protocol::protocol_failure("Grok active activity count overflowed"))?;
        if active >= Self::MAX_ACTIVE_ACTIVITIES {
            return Err(protocol::protocol_failure(format!(
                "Grok ACP exceeded the per-Turn active activity limit of {}",
                Self::MAX_ACTIVE_ACTIVITIES
            )));
        }
        Ok(())
    }

    fn next_activity(&mut self, turn: TurnRef) -> Result<ActivityRef, BackendFailure> {
        let id = NonZeroU64::new(self.next_activity_id)
            .map(ActivityId::new)
            .ok_or_else(|| protocol::protocol_failure("Grok Activity id space was exhausted"))?;
        self.next_activity_id = self
            .next_activity_id
            .checked_add(1)
            .ok_or_else(|| protocol::protocol_failure("Grok Activity id space was exhausted"))?;
        Ok(ActivityRef::new(turn, id))
    }

    fn next_request(&mut self) -> Result<RequestId, BackendFailure> {
        let id = NonZeroU64::new(self.next_request_id)
            .map(RequestId::new)
            .ok_or_else(|| protocol::protocol_failure("Grok request id space was exhausted"))?;
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .ok_or_else(|| protocol::protocol_failure("Grok request id space was exhausted"))?;
        Ok(id)
    }
}

impl<P: JsonPeer> BackendAdapter for Backend<P> {
    type Command = AgentCommand;
    type Event = BackendEvent;
    type ResumeTarget = BackendResumeTarget;

    fn stop_handle(&self) -> BackendStopHandle {
        self.client.stop_handle()
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::none()
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

fn validate_session_id(session_id: &str) -> Result<(), BackendFailure> {
    if session_id.is_empty() || session_id.len() > 2048 {
        return Err(protocol::protocol_failure(
            "Grok ACP returned an invalid Session id",
        ));
    }
    Ok(())
}

fn combine_with_cleanup(
    primary: Result<(), BackendFailure>,
    cleanup: Result<(), BackendFailure>,
) -> Result<(), BackendFailure> {
    match (primary, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(cleanup)) => Err(cleanup),
        (Err(primary), Ok(())) => Err(primary),
        (Err(primary), Err(cleanup)) => Err(BackendFailure::new(
            primary.kind(),
            format!("{}; cleanup also failed: {}", primary.message(), cleanup),
        )),
    }
}
