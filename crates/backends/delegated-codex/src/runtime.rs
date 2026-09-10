mod events;
#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    num::NonZeroU64,
    sync::Arc,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Map, Value, json};
use yo_backend::{BackendAdapter, transport::JsonMessagePeer};
use yo_core::{
    AccountId, ActivityApproval, ActivityId, ActivityKind, ActivityOutcome, ActivityRef,
    ActivityRequestRef, ActivityResponse, ActivityUpdate, AgentCommand, ApprovalDecision,
    BackendBindingEvidence, BackendCapabilities, BackendCommandEvidence, BackendEvent,
    BackendFailure, BackendFailureKind, BackendIdentity, BackendPoll, BackendRequestEvidence,
    BackendResumeTarget, BackendStopHandle, ContinuationStrategy, HostId, ImageInputCapability,
    InputImageHistory, ModelId, ModelInputPart, QuestionChoice, RequestId, SessionId, TurnRef,
    UserInput,
};

use crate::{
    BACKEND_KIND, LEGACY_READ_ONLY_BINDING_SCHEMA, LEGACY_STANDARD_BINDING_SCHEMA,
    MODEL_IDENTITY_SCHEMA, READ_ONLY_BINDING_SCHEMA, READ_ONLY_REVIEW_PROFILE,
    STANDARD_BINDING_SCHEMA,
    binding::{binding_account, decode_optional_account, model_and_provider},
    client::AppServerClient,
    config::{CodexBackendConfig, validate_config},
    protocol::{self, CodexCompatibilityWarning},
    transport::StdioPeer,
};

/// Receives Codex compatibility and server warnings without owning process output.
pub type CodexWarningObserver = Arc<dyn Fn(CodexCompatibilityWarning) + Send + Sync + 'static>;

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
        Self::spawn_with_warning_observer(config, None)
    }

    /// Spawns Codex and forwards compatibility observations to the caller-owned observer.
    pub fn spawn_with_warning_observer(
        config: CodexBackendConfig,
        warning_observer: Option<CodexWarningObserver>,
    ) -> Result<Self, BackendFailure> {
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
        let client = AppServerClient::new(peer, config.request_timeout())
            .with_warning_observer(warning_observer);
        let model_rebind_target = config
            .model_rebind_target()
            .map(|(account, model)| (account.clone(), model.clone()));
        let mut inner =
            Backend::new_uninitialized(client, cwd, config.read_only_review(), model_rebind_target);
        inner.new_session_target = config.new_session_target().cloned();
        Ok(Self { inner })
    }

    /// Verifies the local app-server handshake without creating a backend Session.
    pub fn verify(config: CodexBackendConfig) -> Result<(), BackendFailure> {
        Self::verify_with_warning_observer(config, None)
    }

    /// Verifies the local app-server handshake and forwards compatibility observations.
    pub fn verify_with_warning_observer(
        config: CodexBackendConfig,
        warning_observer: Option<CodexWarningObserver>,
    ) -> Result<(), BackendFailure> {
        let mut backend = Self::spawn_with_warning_observer(config, warning_observer)?;
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

    fn resume_session_rebinding_model(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        self.inner.resume_session_rebinding_model(target)
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
    public_summary: Option<BTreeMap<u64, String>>,
    proposed_plan: Option<String>,
    command: Option<Value>,
}

struct RequestBinding {
    file_approval: Option<(String, ActivityApproval)>,
    wire_id: Value,
    request_activity: ActivityRef,
    kind: RequestKind,
    responded: bool,
}

#[derive(Clone)]
enum RequestKind {
    Approval {
        offered: Vec<Value>,
        explicit: bool,
        command: bool,
    },
    Input(InputQuestions),
}

#[derive(Clone)]
struct InputQuestions {
    questions: Vec<InputQuestion>,
    current: usize,
    answers: Map<String, Value>,
    drafts: HashMap<String, (Option<u32>, String)>,
}

#[derive(Clone)]
struct InputQuestion {
    id: String,
    prompt: String,
    question: String,
    options: Vec<String>,
    choices: Vec<QuestionChoice>,
}

#[derive(Clone, Copy)]
struct WireTurnBinding {
    turn: TurnRef,
    interrupted: bool,
    finished: bool,
}

struct Backend<P> {
    client: AppServerClient<P>,
    initialized: bool,
    backend_version: Option<String>,
    account: Option<AccountId>,
    image_wire_supported: bool,
    image_capability: ImageInputCapability,
    input_image_history: InputImageHistory,
    selected_model: Option<String>,
    cwd: String,
    read_only_review: bool,
    model_rebind_target: Option<(AccountId, ModelId)>,
    new_session_target: Option<(AccountId, ModelId)>,
    session: Option<SessionBinding>,
    turns: HashMap<TurnRef, String>,
    wire_turns: HashMap<String, WireTurnBinding>,
    items: HashMap<String, ItemBinding>,
    file_changes: HashMap<(TurnRef, String), ActivityRef>,
    terminal_commands: HashMap<String, (TurnRef, Option<String>)>,
    plans: HashMap<TurnRef, ActivityRef>,
    turn_diffs: HashMap<TurnRef, ActivityRef>,
    requests: HashMap<ActivityRequestRef, RequestBinding>,
    wire_requests: HashMap<String, ActivityRequestRef>,
    turn_errors: HashMap<String, String>,
    pending_events: VecDeque<BackendEvent>,
    terminal_poll: Option<Result<(), BackendFailure>>,
    next_activity_id: u64,
    next_request_id: u64,
}

impl<P: JsonMessagePeer> Backend<P> {
    fn new_uninitialized(
        client: AppServerClient<P>,
        cwd: String,
        read_only_review: bool,
        model_rebind_target: Option<(AccountId, ModelId)>,
    ) -> Self {
        Self {
            client,
            initialized: false,
            backend_version: None,
            account: None,
            image_wire_supported: false,
            image_capability: ImageInputCapability::Unknown,
            input_image_history: InputImageHistory::TextOnly,
            selected_model: None,
            cwd,
            read_only_review,
            model_rebind_target,
            new_session_target: None,
            session: None,
            turns: HashMap::new(),
            wire_turns: HashMap::new(),
            items: HashMap::new(),
            file_changes: HashMap::new(),
            terminal_commands: HashMap::new(),
            plans: HashMap::new(),
            turn_diffs: HashMap::new(),
            requests: HashMap::new(),
            wire_requests: HashMap::new(),
            turn_errors: HashMap::new(),
            pending_events: VecDeque::new(),
            terminal_poll: None,
            next_activity_id: 1,
            next_request_id: 1,
        }
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::none()
            .with_steer()
            .with_native_model_rebind()
            .with_image_input(self.image_capability)
    }

    fn execute_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        match command {
            AgentCommand::CreateSession { session_id } => self.create_session(session_id),
            AgentCommand::StartTurn { turn, input } => {
                self.validate_direct_input(&input)?;
                let thread_id = self.thread_id(turn.session_id())?.to_owned();
                let projected_input = project_input(&input)?;
                let mut params = json!({
                    "threadId": thread_id,
                    "input": projected_input,
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
                        finished: false,
                    },
                );
                if !input.images().is_empty() {
                    self.input_image_history = InputImageHistory::ContainsImages;
                }
                Ok(BackendCommandEvidence::RequestAccepted(
                    BackendRequestEvidence::new(
                        "codex.app-server/turn-start/v1",
                        json_rpc_identity(call.request_id),
                        accepted_request_identity(call.request_id, &wire_turn),
                    ),
                ))
            },
            AgentCommand::SteerTurn { turn, input } => {
                self.validate_direct_input(&input)?;
                let thread_id = self.thread_id(turn.session_id())?.to_owned();
                let turn_id = self.turn_id(turn)?.to_owned();
                let projected_input = project_input(&input)?;
                let call = self.client.call(
                    "turn/steer",
                    json!({
                        "threadId": thread_id,
                        "expectedTurnId": &turn_id,
                        "input": projected_input,
                    }),
                )?;
                let accepted = protocol::string_at(&call.result, &["turnId"])?;
                if accepted != turn_id {
                    return Err(protocol::protocol_failure(format!(
                        "Codex steer accepted Turn `{accepted}` instead of `{turn_id}`"
                    )));
                }
                if !input.images().is_empty() {
                    self.input_image_history = InputImageHistory::ContainsImages;
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
        if let Some((account, model)) = &self.new_session_target {
            if self.model_rebind_target.is_some() || self.account.as_ref() != Some(account) {
                return Err(BackendFailure::new(
                    BackendFailureKind::Session,
                    "new Session requires the selected authenticated account and no rebind target",
                ));
            }
            params["model"] = json!(model.as_str());
        }
        self.apply_thread_policy(&mut params);
        let result = self.client.call("thread/start", params)?.result;
        let thread_id = protocol::string_at(&result, &["thread", "id"])?.to_owned();
        let backend_session_id = protocol::string_at(&result, &["thread", "sessionId"])?;
        let model = protocol::string_at(&result, &["model"])?;
        if self
            .new_session_target
            .as_ref()
            .is_some_and(|(_, expected)| model != expected.as_str())
        {
            return Err(protocol::protocol_failure(
                "new Session returned a different model",
            ));
        }
        let model_provider = protocol::string_at(&result, &["modelProvider"])?;
        self.selected_model = Some(model.to_owned());
        let backend_version = self.backend_version.clone().ok_or_else(|| {
            protocol::protocol_failure("Codex backend version was not retained after initialize")
        })?;
        let model_value = json!({
            "model": model,
            "provider": model_provider,
        })
        .to_string();
        self.client.bind_notice_thread(&thread_id);
        self.session = Some(SessionBinding {
            yo: session_id,
            codex: thread_id.clone(),
        });
        if self.image_wire_supported {
            self.refresh_model_capability(model);
        } else {
            self.image_capability = ImageInputCapability::Unknown;
        }
        Ok(BackendCommandEvidence::BindingOpened(
            BackendBindingEvidence::new(
                BACKEND_KIND,
                backend_version,
                self.binding_identity(backend_session_id, &thread_id)?,
                BackendIdentity::new(MODEL_IDENTITY_SCHEMA, model_value),
                BackendIdentity::new("codex.app-server/thread-locator/v1", thread_id),
                ContinuationStrategy::BackendManagedState,
            ),
        ))
    }

    fn initialize(&mut self) -> Result<(), BackendFailure> {
        if !self.initialized {
            let initialize = self.client.initialize()?;
            let account_result = self
                .client
                .call("account/read", json!({ "refreshToken": false }))?
                .result;
            self.backend_version = Some(initialize.user_agent);
            self.image_wire_supported = protocol::image_wire_version_supported(
                self.backend_version.as_deref().unwrap_or_default(),
            );
            self.image_capability = ImageInputCapability::Unknown;
            self.account = decode_optional_account(&HostId::codex(), &account_result)?;
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
        if self.new_session_target.is_some() {
            return Err(BackendFailure::new(
                BackendFailureKind::Session,
                "a new-session target cannot be used to resume or fork a Session",
            ));
        }
        self.resume_binding_with_history(
            target.session_id(),
            target.binding(),
            target.input_image_history(),
        )
    }

    #[cfg(test)]
    fn resume_binding(
        &mut self,
        session_id: SessionId,
        binding: &BackendBindingEvidence,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        self.resume_binding_with_history(session_id, binding, InputImageHistory::TextOnly)
    }

    fn resume_binding_with_history(
        &mut self,
        session_id: SessionId,
        binding: &BackendBindingEvidence,
        image_history: InputImageHistory,
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
        let locator = binding.session_locator();
        if locator.schema() != "codex.app-server/thread-locator/v1" {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                format!("unsupported Codex Session locator `{}`", locator.schema()),
            ));
        }
        let thread_id = locator.value();
        self.initialize()?;
        self.validate_execution_binding(binding)?;
        let (expected_model, _) = model_and_provider(binding.model_identity())?;
        if image_history != InputImageHistory::TextOnly {
            self.require_image_capability(expected_model.as_str())?;
        }
        let mut params = json!({ "threadId": thread_id });
        if image_history != InputImageHistory::TextOnly {
            params["model"] = json!(expected_model.as_str());
        }
        self.apply_thread_policy(&mut params);
        let result = self.client.call("thread/resume", params)?.result;
        let resumed_thread = protocol::string_at(&result, &["thread", "id"])?;
        let backend_session_id = protocol::string_at(&result, &["thread", "sessionId"])?;
        let model = protocol::string_at(&result, &["model"])?;
        let model_provider = protocol::string_at(&result, &["modelProvider"])?;
        let binding_identity =
            self.binding_identity_for_resume(binding, backend_session_id, resumed_thread)?;
        let model_identity = BackendIdentity::new(
            MODEL_IDENTITY_SCHEMA,
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
        self.selected_model = Some(model.to_owned());
        if self.image_wire_supported {
            self.refresh_model_capability(model);
        } else {
            self.image_capability = ImageInputCapability::Unknown;
        }
        if image_history != InputImageHistory::TextOnly {
            self.require_image_capability(model)?;
        }
        self.client.bind_notice_thread(resumed_thread);
        self.session = Some(SessionBinding {
            yo: session_id,
            codex: resumed_thread.to_owned(),
        });
        self.input_image_history = image_history;
        Ok(evidence)
    }

    fn resume_session_rebinding_model(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        self.rebind_model_with_history(
            target.session_id(),
            target.binding(),
            target.input_image_history(),
        )
    }

    #[cfg(test)]
    fn rebind_model(
        &mut self,
        session_id: SessionId,
        source: &BackendBindingEvidence,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        self.rebind_model_with_history(session_id, source, InputImageHistory::TextOnly)
    }

    fn rebind_model_with_history(
        &mut self,
        session_id: SessionId,
        source: &BackendBindingEvidence,
        image_history: InputImageHistory,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        if source.backend_kind() != BACKEND_KIND
            || source.continuation_strategy() != ContinuationStrategy::BackendManagedState
        {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                "Codex native model rebind requires a Codex backend-managed source binding",
            ));
        }
        if source.session_locator().schema() != "codex.app-server/thread-locator/v1" {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                "Codex native model rebind requires the supported thread locator",
            ));
        }
        let (requested_account, requested_model) =
            self.model_rebind_target.clone().ok_or_else(|| {
                BackendFailure::new(
                    BackendFailureKind::Unsupported,
                    "Codex native model rebind has no exact account and model target",
                )
            })?;
        self.initialize()?;
        self.validate_execution_binding(source)?;
        let source_account = binding_account(source.binding_identity())?.ok_or_else(|| {
            BackendFailure::new(
                BackendFailureKind::Unsupported,
                "this legacy Codex binding predates verified account identity and cannot be rebound",
            )
        })?;
        if source_account != requested_account || self.account.as_ref() != Some(&requested_account)
        {
            return Err(BackendFailure::new(
                BackendFailureKind::Session,
                "Codex native model rebind account differs from the source Session account",
            ));
        }
        let (source_model, source_provider) = model_and_provider(source.model_identity())?;
        if source_model == requested_model {
            return Err(BackendFailure::new(
                BackendFailureKind::Session,
                "Codex native model rebind target is already active",
            ));
        }
        if image_history != InputImageHistory::TextOnly {
            self.require_image_capability(requested_model.as_str())?;
        }

        let source_thread = source.session_locator().value();
        let mut params = json!({
            "threadId": source_thread,
            "model": requested_model.as_str(),
            "cwd": self.cwd,
        });
        self.apply_thread_policy(&mut params);
        let result = self.client.call("thread/fork", params)?.result;
        let thread_id = protocol::string_at(&result, &["thread", "id"])?;
        let backend_session_id = protocol::string_at(&result, &["thread", "sessionId"])?;
        let model = protocol::string_at(&result, &["model"])?;
        let model_provider = protocol::string_at(&result, &["modelProvider"])?;
        if thread_id == source_thread
            || model != requested_model.as_str()
            || model_provider != source_provider
        {
            return Err(BackendFailure::new(
                BackendFailureKind::Session,
                "Codex thread/fork did not return a distinct thread with the exact requested model and source provider",
            ));
        }
        let evidence = BackendBindingEvidence::new(
            BACKEND_KIND,
            self.backend_version.clone().ok_or_else(|| {
                protocol::protocol_failure(
                    "Codex backend version was not retained after rebind initialize",
                )
            })?,
            self.binding_identity(backend_session_id, thread_id)?,
            BackendIdentity::new(
                MODEL_IDENTITY_SCHEMA,
                json!({ "model": model, "provider": model_provider }).to_string(),
            ),
            BackendIdentity::new("codex.app-server/thread-locator/v1", thread_id),
            ContinuationStrategy::BackendManagedState,
        );
        self.selected_model = Some(model.to_owned());
        if self.image_wire_supported {
            self.refresh_model_capability(model);
        } else {
            self.image_capability = ImageInputCapability::Unknown;
        }
        if image_history != InputImageHistory::TextOnly {
            self.require_image_capability(model)?;
        }
        self.client.bind_notice_thread(thread_id);
        self.session = Some(SessionBinding {
            yo: session_id,
            codex: thread_id.to_owned(),
        });
        self.input_image_history = image_history;
        Ok(evidence)
    }

    fn validate_direct_input(&mut self, input: &UserInput) -> Result<(), BackendFailure> {
        if input.images().is_empty() && self.input_image_history == InputImageHistory::TextOnly {
            return Ok(());
        }
        let Some(model) = self.selected_model.clone() else {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                "Codex image input has no verified selected model",
            ));
        };
        self.require_image_capability(&model)?;
        if let ImageInputCapability::Supported {
            maximum_occurrences,
            maximum_image_bytes,
            maximum_input_bytes,
        } = self.image_capability
            && (!input.images().is_empty())
        {
            let total = input.images().iter().try_fold(0_u64, |total, image| {
                total.checked_add(image.snapshot().png().len() as u64)
            });
            if input.images().len() as u64 > u64::from(maximum_occurrences)
                || input
                    .images()
                    .iter()
                    .any(|image| image.snapshot().png().len() as u64 > maximum_image_bytes)
                || total.is_none_or(|total| total > maximum_input_bytes)
            {
                return Err(BackendFailure::new(
                    BackendFailureKind::InputOverBudget,
                    "Codex image input exceeds the selected model's admitted limits",
                ));
            }
        }
        Ok(())
    }

    fn require_image_capability(&mut self, expected_model: &str) -> Result<(), BackendFailure> {
        if !self.image_wire_supported {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                "Codex image input requires the reviewed image-capable protocol build",
            ));
        }
        if self.selected_model.as_deref() != Some(expected_model)
            || self.image_capability == ImageInputCapability::Unknown
        {
            self.selected_model = Some(expected_model.to_owned());
            self.refresh_model_capability(expected_model);
        }
        match self.image_capability {
            ImageInputCapability::Supported { .. } => Ok(()),
            ImageInputCapability::Unsupported => Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                "the selected Codex model does not advertise image input",
            )),
            ImageInputCapability::Unknown => Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                "Codex image capability could not be established for the selected model",
            )),
        }
    }

    fn refresh_model_capability(&mut self, model: &str) {
        self.image_capability = crate::observation::observe_model_capability(
            &mut self.client,
            model,
            self.image_wire_supported,
        );
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

    fn binding_identity(
        &self,
        session_id: &str,
        thread_id: &str,
    ) -> Result<BackendIdentity, BackendFailure> {
        Ok(match (self.read_only_review, self.account.as_ref()) {
            (true, Some(account)) => BackendIdentity::new(
                READ_ONLY_BINDING_SCHEMA,
                json!({
                    "accountId": account.as_str(),
                    "executionProfile": READ_ONLY_REVIEW_PROFILE,
                    "sessionId": session_id,
                    "threadId": thread_id,
                })
                .to_string(),
            ),
            (true, None) => BackendIdentity::new(
                LEGACY_READ_ONLY_BINDING_SCHEMA,
                json!({
                    "executionProfile": READ_ONLY_REVIEW_PROFILE,
                    "sessionId": session_id,
                    "threadId": thread_id,
                })
                .to_string(),
            ),
            (false, Some(account)) => BackendIdentity::new(
                STANDARD_BINDING_SCHEMA,
                json!({
                    "accountId": account.as_str(),
                    "sessionId": session_id,
                    "threadId": thread_id,
                })
                .to_string(),
            ),
            (false, None) => BackendIdentity::new(
                LEGACY_STANDARD_BINDING_SCHEMA,
                json!({ "sessionId": session_id, "threadId": thread_id }).to_string(),
            ),
        })
    }

    fn binding_identity_for_resume(
        &self,
        source: &BackendBindingEvidence,
        session_id: &str,
        thread_id: &str,
    ) -> Result<BackendIdentity, BackendFailure> {
        Ok(match source.binding_identity().schema() {
            LEGACY_STANDARD_BINDING_SCHEMA => BackendIdentity::new(
                LEGACY_STANDARD_BINDING_SCHEMA,
                json!({ "sessionId": session_id, "threadId": thread_id }).to_string(),
            ),
            LEGACY_READ_ONLY_BINDING_SCHEMA => BackendIdentity::new(
                LEGACY_READ_ONLY_BINDING_SCHEMA,
                json!({
                    "executionProfile": READ_ONLY_REVIEW_PROFILE,
                    "sessionId": session_id,
                    "threadId": thread_id,
                })
                .to_string(),
            ),
            STANDARD_BINDING_SCHEMA | READ_ONLY_BINDING_SCHEMA => {
                self.binding_identity(session_id, thread_id)?
            },
            _ => {
                return Err(BackendFailure::new(
                    BackendFailureKind::Session,
                    "Codex durable execution binding schema is unsupported",
                ));
            },
        })
    }

    fn validate_execution_binding(
        &self,
        binding: &BackendBindingEvidence,
    ) -> Result<(), BackendFailure> {
        let identity = binding.binding_identity();
        let profile_matches = if self.read_only_review {
            matches!(
                identity.schema(),
                READ_ONLY_BINDING_SCHEMA | LEGACY_READ_ONLY_BINDING_SCHEMA
            )
        } else {
            matches!(
                identity.schema(),
                STANDARD_BINDING_SCHEMA | LEGACY_STANDARD_BINDING_SCHEMA
            )
        };
        if !profile_matches {
            return Err(BackendFailure::new(
                BackendFailureKind::Session,
                "Codex durable execution profile differs from the requested resume profile",
            ));
        }
        if let Some(account) = binding_account(identity)?
            && self.account.as_ref() != Some(&account)
        {
            return Err(BackendFailure::new(
                BackendFailureKind::Session,
                "Codex durable binding account differs from the authenticated account",
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
        let binding = self
            .requests
            .get(&request)
            .filter(|binding| !binding.responded)
            .ok_or_else(|| {
                protocol::protocol_failure("response has no unanswered Codex request")
            })?;
        let wire_id = binding.wire_id.clone();
        let mut next = None;
        let response_text;
        let navigating = matches!(response, ActivityResponse::PreviousQuestion { .. });
        let (payload, response_kind) = match (&binding.kind, response) {
            (
                RequestKind::Approval {
                    offered,
                    explicit,
                    command,
                },
                ActivityResponse::Approval(decision),
            ) => {
                let wire_decision = match decision {
                    ApprovalDecision::Approved => json!("accept"),
                    ApprovalDecision::Declined => json!("decline"),
                    ApprovalDecision::Offered(choice) => choice
                        .checked_sub(1)
                        .and_then(|index| offered.get(index as usize))
                        .filter(|value| events::approval_choice(value, *command).is_some())
                        .cloned()
                        .ok_or_else(|| {
                            protocol::protocol_failure(
                                "approval choice is outside the supported outstanding decisions",
                            )
                        })?,
                };
                // Native default menus do not remove protocol-level accept/decline support
                // from legacy clients; only an explicit server list constrains those replies.
                if *explicit && !offered.contains(&wire_decision) {
                    return Err(protocol::protocol_failure(
                        "approval decision was not offered by the outstanding Codex request",
                    ));
                }
                response_text = match decision {
                    ApprovalDecision::Approved => "Decision: approved".to_owned(),
                    ApprovalDecision::Declined => "Decision: declined".to_owned(),
                    ApprovalDecision::Offered(_) => {
                        let choice = events::approval_choice(&wire_decision, *command)
                            .expect("validated offered decision");
                        format!("Decision: {}\n{}", choice.label, choice.description)
                    },
                };
                (
                    Some(json!({"decision": wire_decision})),
                    ActivityKind::ApprovalResponse {
                        request_id: request.request_id(),
                    },
                )
            },
            (
                RequestKind::Input(questions),
                ActivityResponse::PreviousQuestion { choice, draft },
            ) => {
                if questions.current == 0
                    || choice.is_some_and(|choice| {
                        choice == 0
                            || choice as usize
                                > questions.questions[questions.current].options.len()
                    })
                {
                    return Err(protocol::protocol_failure(
                        "previous question is unavailable or draft choice is invalid",
                    ));
                }
                let mut questions = questions.clone();
                questions.drafts.insert(
                    questions.questions[questions.current].id.clone(),
                    (choice, draft.as_str().to_owned()),
                );
                if questions
                    .question_profile(questions.current)
                    .to_snapshot()
                    .is_none()
                    || questions
                        .question_profile(questions.current - 1)
                        .to_snapshot()
                        .is_none()
                {
                    return Err(protocol::protocol_failure(
                        "question draft exceeds the presentation limit",
                    ));
                }
                questions.current -= 1;
                next = Some(questions);
                response_text = String::new();
                (
                    None,
                    ActivityKind::UserInputResponse {
                        request_id: request.request_id(),
                    },
                )
            },
            (
                RequestKind::Input(questions),
                response @ (ActivityResponse::UserInput(_)
                | ActivityResponse::QuestionAnswer { .. }),
            ) => {
                let mut questions = questions.clone();
                let question = &questions.questions[questions.current];
                let draft = match &response {
                    ActivityResponse::UserInput(input) => (None, input.as_str().to_owned()),
                    ActivityResponse::QuestionAnswer { choice, notes } => {
                        (Some(*choice), notes.as_str().to_owned())
                    },
                    _ => unreachable!("input response matched above"),
                };
                let (answers, receipt) = match response {
                    ActivityResponse::UserInput(answer) => {
                        let selected = answer
                            .as_str()
                            .trim()
                            .parse::<usize>()
                            .ok()
                            .and_then(|index| index.checked_sub(1))
                            .and_then(|index| question.options.get(index));
                        let answer = selected.map_or(answer.as_str(), String::as_str);
                        (vec![answer.to_owned()], questions.receipt(answer, None))
                    },
                    ActivityResponse::QuestionAnswer { choice, notes } => {
                        let selected = usize::try_from(choice)
                            .ok()
                            .and_then(|index| index.checked_sub(1))
                            .and_then(|index| question.options.get(index))
                            .ok_or_else(|| {
                                protocol::protocol_failure(
                                    "question choice is outside the outstanding options",
                                )
                            })?;
                        let mut answers = vec![selected.clone()];
                        if !notes.as_str().trim().is_empty() {
                            answers.push(format!("user_note: {}", notes.as_str().trim()));
                        }
                        let receipt = questions.receipt(
                            selected,
                            Some(notes.as_str().trim()).filter(|notes| !notes.is_empty()),
                        );
                        (answers, receipt)
                    },
                    ActivityResponse::Approval(_) | ActivityResponse::PreviousQuestion { .. } => {
                        unreachable!("input response matched above")
                    },
                };
                questions.drafts.insert(question.id.clone(), draft);
                response_text = receipt;
                questions
                    .answers
                    .insert(question.id.clone(), json!({"answers": answers}));
                questions.current += 1;
                let payload = if questions.current == questions.questions.len() {
                    Some(json!({"answers": questions.answers}))
                } else {
                    next = Some(questions);
                    None
                };
                (
                    payload,
                    ActivityKind::UserInputResponse {
                        request_id: request.request_id(),
                    },
                )
            },
            _ => {
                return Err(protocol::protocol_failure(
                    "response kind does not match the Codex request",
                ));
            },
        };
        let interview_progress = match &binding.kind {
            RequestKind::Input(questions) => {
                Some((questions.answers.len(), questions.questions.len()))
            },
            RequestKind::Approval { .. } => None,
        };
        let response_activity = self.next_activity(request.activity().turn())?;
        let next = next
            .map(|questions| {
                let activity = self.next_activity(request.activity().turn())?;
                let request_id = self.next_request()?;
                Ok::<_, BackendFailure>((
                    questions,
                    ActivityRequestRef::new(activity, request_id),
                    events::wire_key(&wire_id)?,
                ))
            })
            .transpose()?;
        if let Some(payload) = payload {
            self.client.respond(wire_id.clone(), payload).map_err(|failure| {
                if let Some((recorded, total)) = interview_progress {
                    BackendFailure::new(
                        failure.kind(),
                        format!(
                            "{}\nInterview incomplete: {recorded}/{total} earlier answers recorded. Final submission was not confirmed.",
                            failure.message()
                        ),
                    )
                } else {
                    failure
                }
            })?;
            self.requests
                .get_mut(&request)
                .expect("validated request")
                .responded = true;
        }
        if !navigating {
            self.pending_events
                .push_back(BackendEvent::ActivityStarted {
                    activity: response_activity,
                    kind: response_kind,
                });
            self.pending_events
                .push_back(BackendEvent::ActivityUpdated {
                    activity: response_activity,
                    update: ActivityUpdate::TextSnapshot(response_text),
                });
            self.pending_events
                .push_back(BackendEvent::ActivityFinished {
                    activity: response_activity,
                    outcome: ActivityOutcome::Completed,
                });
        }
        if let Some((questions, successor, wire_key)) = next {
            let activity = successor.activity();
            let request_id = successor.request_id();
            self.requests.remove(&request);
            self.wire_requests.insert(wire_key, successor);
            self.pending_events
                .push_back(BackendEvent::ActivityFinished {
                    activity: request.activity(),
                    outcome: ActivityOutcome::Completed,
                });
            self.pending_events
                .push_back(BackendEvent::ActivityStarted {
                    activity,
                    kind: ActivityKind::UserInputRequest { request_id },
                });
            self.pending_events
                .push_back(BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(questions.prompt()),
                });
            self.requests.insert(
                successor,
                RequestBinding {
                    file_approval: None,
                    wire_id,
                    request_activity: activity,
                    kind: RequestKind::Input(questions),
                    responded: false,
                },
            );
        }
        Ok(BackendCommandEvidence::None)
    }

    fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        if let Some(event) = self.pending_events.pop_front() {
            return Ok(BackendPoll::Event(event));
        }
        if let Some(terminal) = &self.terminal_poll {
            return terminal.clone().map(|()| BackendPoll::Closed);
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

    fn resume_session_rebinding_model(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        self.resume_session_rebinding_model(target)
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

fn project_input(input: &UserInput) -> Result<Vec<Value>, BackendFailure> {
    if input.images().is_empty() {
        return Ok(vec![json!({
            "type": "text",
            "text": input.model_input(),
        })]);
    }
    let parts = input.model_parts();
    ModelInputPart::validate_user_parts(&parts).map_err(|detail| {
        protocol::protocol_failure(format!("invalid Codex image input: {detail}"))
    })?;
    Ok(parts
        .into_iter()
        .map(|part| match part {
            ModelInputPart::Text { text } => json!({ "type": "text", "text": text }),
            ModelInputPart::Image { snapshot } => json!({
                "type": "image",
                "url": format!("data:image/png;base64,{}", STANDARD.encode(snapshot.png())),
            }),
        })
        .collect())
}
