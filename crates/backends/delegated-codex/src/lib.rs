mod client;
mod config;
mod events;
mod protocol;
mod skill_catalog;
mod transport;

#[cfg(test)]
mod tests;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    num::NonZeroU64,
};

use client::AppServerClient;
pub use config::CodexBackendConfig;
use serde_json::{Value, json};
pub use skill_catalog::CodexSkillReferenceProvider;
use transport::StdioPeer;
use yo_backend::{BackendAdapter, transport::JsonMessagePeer};
use yo_core::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef, ActivityResponse,
    AgentCommand, ApprovalDecision, BackendBindingEvidence, BackendCapabilities,
    BackendCommandEvidence, BackendEvent, BackendFailure, BackendFailureKind, BackendIdentity,
    BackendPoll, BackendRequestEvidence, BackendResumeTarget, BackendStopHandle,
    ContinuationStrategy, RequestId, SessionId, TurnRef,
};

pub const HOST_ID: &str = "codex";
pub const BACKEND_KIND: &str = "codex-app-server";
const READ_ONLY_REVIEW_PROFILE: &str = "yo.delegated-review-execution/v1alpha1";
const STANDARD_BINDING_SCHEMA: &str = "codex.app-server/thread-binding/v1";
const READ_ONLY_BINDING_SCHEMA: &str = "codex.app-server/thread-binding/v1alpha1";

/// Local stdio adapter for a compatible `codex app-server` process.
pub struct CodexBackend {
    inner: Backend<StdioPeer>,
}

impl CodexBackend {
    /// Spawns Codex and prepares the cancellable transport.
    ///
    /// The initialize handshake is deferred to `CreateSession` so the runtime owner can cancel it
    /// through [`yo_core::AgentBackend::stop_handle`].
    pub fn spawn(config: CodexBackendConfig) -> Result<Self, BackendFailure> {
        validate_config(&config)?;
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
            inner: Backend::new_uninitialized(client, cwd, config.read_only_review()),
        })
    }

    /// Verifies the local app-server handshake without creating a backend Session.
    pub fn verify(config: CodexBackendConfig) -> Result<(), BackendFailure> {
        let mut backend = Self::spawn(config)?;
        let verification = backend.inner.verify();
        let cleanup = backend.inner.shutdown();
        match (verification, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Ok(()), Err(cleanup)) => Err(cleanup),
            (Err(verification), Ok(())) => Err(verification),
            (Err(verification), Err(cleanup)) => Err(BackendFailure::new(
                verification.kind(),
                format!(
                    "{}; cleanup also failed: {}",
                    verification.message(),
                    cleanup
                ),
            )),
        }
    }
}

/// Reads the current Codex account capacity without creating an Agent Session.
pub fn read_account_capacity(
    config: CodexBackendConfig,
) -> Result<yo_core::AccountCapacitySnapshot, BackendFailure> {
    validate_config(&config)?;
    let peer = StdioPeer::spawn(&config)?;
    let mut client = AppServerClient::new(peer, config.request_timeout());
    let observation = observe_account_capacity(&mut client);
    let cleanup = client.shutdown();
    match (observation, cleanup) {
        (Ok(snapshot), Ok(())) => Ok(snapshot),
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(observation), Ok(())) => Err(observation),
        (Err(observation), Err(cleanup)) => Err(BackendFailure::new(
            observation.kind(),
            format!(
                "{}; cleanup also failed: {}",
                observation.message(),
                cleanup
            ),
        )),
    }
}

/// Reads the authenticated Codex account and complete visible app-server model inventory.
pub fn read_model_catalog(
    config: CodexBackendConfig,
) -> Result<yo_core::HostModelCatalog, BackendFailure> {
    validate_config(&config)?;
    let peer = StdioPeer::spawn(&config)?;
    let mut client = AppServerClient::new(peer, config.request_timeout());
    let observation = observe_model_catalog(&mut client);
    let cleanup = client.shutdown();
    match (observation, cleanup) {
        (Ok(catalog), Ok(())) => Ok(catalog),
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(observation), Ok(())) => Err(observation),
        (Err(observation), Err(cleanup)) => Err(BackendFailure::new(
            observation.kind(),
            format!("{}; cleanup also failed: {cleanup}", observation.message()),
        )),
    }
}

fn validate_config(config: &CodexBackendConfig) -> Result<(), BackendFailure> {
    if !config.working_directory().is_absolute()
        || !config.working_directory().is_dir()
        || config.request_timeout().is_zero()
    {
        return Err(BackendFailure::new(
            BackendFailureKind::Initialization,
            "Codex requires an existing absolute working directory and a non-zero request timeout",
        ));
    }
    Ok(())
}

fn observe_account_capacity<P: JsonMessagePeer>(
    client: &mut AppServerClient<P>,
) -> Result<yo_core::AccountCapacitySnapshot, BackendFailure> {
    client.initialize()?;
    let result = client.call("account/rateLimits/read", Value::Null)?.result;
    protocol::decode_account_capacity(result)
}

fn observe_model_catalog<P: JsonMessagePeer>(
    client: &mut AppServerClient<P>,
) -> Result<yo_core::HostModelCatalog, BackendFailure> {
    const PAGE_LIMIT: u64 = 100;
    const MAX_MODELS: usize = 4096;

    client.initialize()?;
    let account_result = client
        .call("account/read", json!({ "refreshToken": false }))?
        .result;
    let (account_label, evidence) = protocol::decode_account_identity(&account_result)?;
    let evidence_refs = evidence
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let host = yo_core::HostId::codex();
    let account = yo_core::derive_host_account_id(&host, &evidence_refs)
        .map_err(|error| protocol::protocol_failure(error.to_string()))?;

    let mut cursor = None::<String>;
    let mut seen_cursors = HashSet::new();
    let mut seen_models = HashSet::new();
    let mut models = Vec::new();
    let mut ids = Vec::new();
    let mut current = None;
    loop {
        let mut params = json!({ "limit": PAGE_LIMIT, "includeHidden": false });
        if let Some(cursor) = cursor.as_ref() {
            params["cursor"] = Value::String(cursor.clone());
        }
        let page = protocol::decode_model_list(client.call("model/list", params)?.result)?;
        for (id, label, is_default) in page.models {
            if models.len() == MAX_MODELS || !seen_models.insert(id.clone()) {
                return Err(protocol::protocol_failure(
                    "Codex model/list exceeded the model bound or repeated a model id",
                ));
            }
            let id = yo_core::ModelId::new(id)
                .map_err(|error| protocol::protocol_failure(error.to_string()))?;
            if is_default && current.replace(id.clone()).is_some() {
                return Err(protocol::protocol_failure(
                    "Codex model/list advertised more than one default model",
                ));
            }
            ids.push(id.clone());
            models.push(
                yo_core::HostCatalogModel::selectable(id, label)
                    .map_err(|error| protocol::protocol_failure(error.to_string()))?,
            );
        }
        let Some(next) = page.next_cursor else {
            break;
        };
        if !seen_cursors.insert(next.clone()) {
            return Err(protocol::protocol_failure(
                "Codex model/list repeated a pagination cursor",
            ));
        }
        cursor = Some(next);
    }
    let revision = yo_core::derive_host_catalog_revision(&host, &account, current.as_ref(), &ids);
    yo_core::HostModelCatalog::new(
        host,
        "Codex",
        account,
        account_label,
        revision,
        current,
        models,
    )
    .map_err(|error| protocol::protocol_failure(error.to_string()))
}

impl BackendAdapter for CodexBackend {
    type Command = AgentCommand;
    type Event = BackendEvent;
    type ResumeTarget = BackendResumeTarget;

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
    read_only_review: bool,
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

impl<P: JsonMessagePeer> Backend<P> {
    fn new_uninitialized(client: AppServerClient<P>, cwd: String, read_only_review: bool) -> Self {
        Self {
            client,
            initialized: false,
            backend_version: None,
            cwd,
            read_only_review,
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
                let mut params = json!({
                    "threadId": thread_id,
                    "input": [{ "type": "text", "text": input.into_string() }],
                    "cwd": self.cwd,
                });
                self.apply_turn_policy(&mut params);
                let call = self.client.call("turn/start", params)?;
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
            AgentCommand::CompactContext { .. } => Err(BackendFailure::new(
                BackendFailureKind::CommandRejected,
                "Codex delegated Sessions do not use Yo-managed context compaction",
            )),
        }
    }

    fn create_session(
        &mut self,
        session_id: SessionId,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        self.initialize()?;
        let mut params = json!({
            "cwd": self.cwd,
            "serviceName": "yo",
        });
        self.apply_thread_policy(&mut params);
        let result = self.client.call("thread/start", params)?.result;
        let thread_id = protocol::string_at(&result, &["thread", "id"])?.to_owned();
        let backend_session_id = protocol::string_at(&result, &["thread", "sessionId"])?;
        let model = protocol::string_at(&result, &["model"])?;
        let model_provider = protocol::string_at(&result, &["modelProvider"])?;
        let backend_version = self.backend_version.clone().ok_or_else(|| {
            protocol::protocol_failure("Codex backend version was not retained after initialize")
        })?;
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
                BACKEND_KIND,
                backend_version,
                self.binding_identity(backend_session_id, &thread_id),
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

    fn verify(&mut self) -> Result<(), BackendFailure> {
        self.initialize()
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
                    "Codex cannot resume backend kind `{}`",
                    binding.backend_kind()
                ),
            ));
        }
        self.validate_execution_binding(binding)?;
        let locator = binding.session_locator();
        if locator.schema() != "codex.app-server/thread-locator/v1" {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                format!("unsupported Codex Session locator `{}`", locator.schema()),
            ));
        }
        let thread_id = locator.value();
        self.initialize()?;
        let mut params = json!({ "threadId": thread_id });
        self.apply_thread_policy(&mut params);
        let result = self.client.call("thread/resume", params)?.result;
        let resumed_thread = protocol::string_at(&result, &["thread", "id"])?;
        let backend_session_id = protocol::string_at(&result, &["thread", "sessionId"])?;
        let model = protocol::string_at(&result, &["model"])?;
        let model_provider = protocol::string_at(&result, &["modelProvider"])?;
        let binding_identity = self.binding_identity(backend_session_id, resumed_thread);
        let model_identity = BackendIdentity::new(
            "codex.app-server/model-and-provider/v1",
            json!({ "model": model, "provider": model_provider }).to_string(),
        );
        let evidence = BackendBindingEvidence::new(
            BACKEND_KIND,
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
        if !binding.same_resume_identity(&evidence) {
            return Err(BackendFailure::new(
                BackendFailureKind::Session,
                "Codex resumed a binding whose thread, Session, model, or provider identity differs from the durable Continuation Anchor",
            ));
        }
        self.session = Some(SessionBinding {
            yo: session_id,
            codex: resumed_thread.to_owned(),
        });
        Ok(evidence)
    }

    fn apply_thread_policy(&self, params: &mut Value) {
        if self.read_only_review {
            params["approvalPolicy"] = json!("never");
            params["sandbox"] = json!("read-only");
        }
    }

    fn apply_turn_policy(&self, params: &mut Value) {
        if self.read_only_review {
            params["approvalPolicy"] = json!("never");
            params["sandboxPolicy"] = json!({
                "type": "readOnly",
                "networkAccess": false,
            });
        }
    }

    fn binding_identity(&self, session_id: &str, thread_id: &str) -> BackendIdentity {
        if self.read_only_review {
            BackendIdentity::new(
                READ_ONLY_BINDING_SCHEMA,
                json!({
                    "executionProfile": READ_ONLY_REVIEW_PROFILE,
                    "sessionId": session_id,
                    "threadId": thread_id,
                })
                .to_string(),
            )
        } else {
            BackendIdentity::new(
                STANDARD_BINDING_SCHEMA,
                json!({ "sessionId": session_id, "threadId": thread_id }).to_string(),
            )
        }
    }

    fn validate_execution_binding(
        &self,
        binding: &BackendBindingEvidence,
    ) -> Result<(), BackendFailure> {
        let identity = binding.binding_identity();
        let expected_schema = if self.read_only_review {
            READ_ONLY_BINDING_SCHEMA
        } else {
            STANDARD_BINDING_SCHEMA
        };
        if identity.schema() != expected_schema {
            return Err(BackendFailure::new(
                BackendFailureKind::Session,
                "Codex durable execution profile differs from the requested resume profile",
            ));
        }
        if self.read_only_review {
            let value: Value = serde_json::from_str(identity.value()).map_err(|_| {
                protocol::protocol_failure("Codex read-only review binding is malformed")
            })?;
            if value.get("executionProfile").and_then(Value::as_str)
                != Some(READ_ONLY_REVIEW_PROFILE)
            {
                return Err(protocol::protocol_failure(
                    "Codex read-only review binding has a different execution profile",
                ));
            }
        }
        Ok(())
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

impl<P: JsonMessagePeer> BackendAdapter for Backend<P> {
    type Command = AgentCommand;
    type Event = BackendEvent;
    type ResumeTarget = BackendResumeTarget;

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
