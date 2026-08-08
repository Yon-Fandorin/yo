//! Yo-managed model/tool loop over the OpenAI Responses connector.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    num::NonZeroU64,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use serde_json::json;

use super::{
    AgentBackend, BackendBindingEvidence, BackendCapabilities, BackendCommandEvidence,
    BackendEvent, BackendFailure, BackendFailureKind, BackendIdentity, BackendOutcomeEvidence,
    BackendPoll, BackendRequestEvidence, BackendResumeTarget, BackendStopHandle,
};
use crate::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef, ActivityResponse,
    AgentCommand, ApiCredential, ApprovalDecision, ContinuationStrategy, EffectiveModelBinding,
    Failure, FrozenToolRegistry, ModelCatalogEntry, ModelContextProfile, ModelReplay,
    ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole, ModelTokenCounter,
    OpenAiResponsesConnector, ReasoningChannel, ReasoningEffort, ReplayExecutor, RequestId,
    ResponseTerminal, ResponsesCancellation, ResponsesConnectorLimits, ResponsesEvent,
    ResponsesInputItem, ResponsesInputRole, ResponsesPoll, ResponsesRequest, ResponsesStream,
    SessionId, ToolApprovalBinding, ToolApprovalRequirement, ToolExecution, ToolExecutionHost,
    ToolExecutionOutcome, ToolExecutionPoll, ToolExecutionRequest, ToolSemanticAdmission,
    ToolValidationFailure, TurnOutcome, TurnRef, ValidatedToolCall,
};

const BACKEND_KIND: &str = "yo-managed-model";
const BACKEND_VERSION: &str = "1";
const TOOL_TRUNCATION_MARKER: &str = "\n[yo: tool output truncated]";

#[derive(Clone, Debug)]
pub struct NativeModelBackendConfig {
    pub system_prompt: String,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub maximum_model_rounds: usize,
    pub maximum_tool_argument_bytes: usize,
    pub maximum_tool_output_bytes: usize,
}

impl Default for NativeModelBackendConfig {
    fn default() -> Self {
        Self {
            system_prompt: "You are Yo, a careful software-engineering agent.".to_owned(),
            reasoning_effort: Some(ReasoningEffort::Medium),
            maximum_model_rounds: 32,
            maximum_tool_argument_bytes: 4 * 1024 * 1024,
            maximum_tool_output_bytes: 4 * 1024 * 1024,
        }
    }
}

/// Host-owned services used by the provider-neutral model loop.
pub struct NativeModelBackendServices {
    semantic_admission: Option<Box<dyn ToolSemanticAdmission>>,
    tool_host: Box<dyn ToolExecutionHost>,
    token_counter: Box<dyn ModelTokenCounter>,
}

impl NativeModelBackendServices {
    pub fn new(
        semantic_admission: Option<Box<dyn ToolSemanticAdmission>>,
        tool_host: Box<dyn ToolExecutionHost>,
        token_counter: Box<dyn ModelTokenCounter>,
    ) -> Self {
        Self {
            semantic_admission,
            tool_host,
            token_counter,
        }
    }
}

trait ResponseConnector: Send {
    fn request_url(&self) -> &str;
    fn start(
        &self,
        request: ResponsesRequest,
        cancellation: ResponsesCancellation,
    ) -> Result<Box<dyn ResponseStream>, crate::ConnectorError>;
}

trait ResponseStream: Send {
    fn poll(&mut self) -> Result<ResponsesPoll, crate::ConnectorError>;
    fn cancel(&self);
    fn shutdown(&mut self) -> Result<(), crate::ConnectorError>;
}

impl ResponseConnector for OpenAiResponsesConnector {
    fn request_url(&self) -> &str {
        self.request_url()
    }

    fn start(
        &self,
        request: ResponsesRequest,
        cancellation: ResponsesCancellation,
    ) -> Result<Box<dyn ResponseStream>, crate::ConnectorError> {
        OpenAiResponsesConnector::start(self, request, cancellation)
            .map(|stream| Box::new(stream) as Box<dyn ResponseStream>)
    }
}

impl ResponseStream for ResponsesStream {
    fn poll(&mut self) -> Result<ResponsesPoll, crate::ConnectorError> {
        self.poll()
    }

    fn cancel(&self) {
        self.cancel();
    }

    fn shutdown(&mut self) -> Result<(), crate::ConnectorError> {
        self.shutdown()
    }
}

#[derive(Default)]
struct SharedStop {
    requested: AtomicBool,
    response: Mutex<Option<ResponsesCancellation>>,
}

struct PendingCall {
    call: ValidatedToolCall,
    approval: Option<ToolApprovalBinding>,
}

struct ActiveTool {
    call: ValidatedToolCall,
    activity: ActivityRef,
    execution: Box<dyn ToolExecution>,
}

struct CallActivity {
    activity: ActivityRef,
    output_index: usize,
    call_id: String,
    name: String,
}

struct TurnState {
    turn: TurnRef,
    round: usize,
    delta: Vec<ModelReplayItem>,
    stream: Option<Box<dyn ResponseStream>>,
    response_id: Option<String>,
    assistant_activities: HashMap<(usize, usize), ActivityRef>,
    reasoning_activities: HashMap<(usize, usize), ActivityRef>,
    call_activities: HashMap<String, CallActivity>,
    seen_call_ids: HashSet<String>,
    round_message_items: BTreeSet<usize>,
    round_messages: BTreeMap<(usize, usize), String>,
    round_replay: BTreeMap<usize, ModelReplayItem>,
    pending_calls: BTreeMap<usize, PendingCall>,
    active_tool: Option<ActiveTool>,
    ready_tool: Option<ValidatedToolCall>,
    dispatch_tool: Option<(ValidatedToolCall, ActivityRef)>,
    awaiting_approval: Option<(ActivityRequestRef, PendingCall)>,
    start_next_round: bool,
}

pub struct NativeModelBackend {
    connector: Box<dyn ResponseConnector>,
    binding: EffectiveModelBinding,
    registry: FrozenToolRegistry,
    semantic_admission: Option<Box<dyn ToolSemanticAdmission>>,
    tool_host: Box<dyn ToolExecutionHost>,
    config: NativeModelBackendConfig,
    model_context: ModelContextProfile,
    token_counter: Box<dyn ModelTokenCounter>,
    contract: ModelReplayContract,
    session: Option<SessionId>,
    replay: ModelReplay,
    turn: Option<TurnState>,
    events: VecDeque<BackendEvent>,
    open_activities: HashSet<ActivityRef>,
    next_activity_id: u64,
    next_request_id: u64,
    shared_stop: Arc<SharedStop>,
    closed: bool,
    context_exhausted: bool,
    shutdown_result: Option<Result<(), BackendFailure>>,
}

impl NativeModelBackend {
    pub fn new(
        catalog_entry: &ModelCatalogEntry,
        credential: ApiCredential,
        connector_limits: ResponsesConnectorLimits,
        registry: FrozenToolRegistry,
        services: NativeModelBackendServices,
        config: NativeModelBackendConfig,
    ) -> Result<Self, BackendFailure> {
        let binding = catalog_entry.binding().clone();
        let connector = OpenAiResponsesConnector::new(&binding, credential, connector_limits)
            .map_err(map_connector_initialization)?;
        Self::with_connector(
            Box::new(connector),
            binding,
            registry,
            services,
            catalog_entry.context().clone(),
            config,
        )
    }

    fn with_connector(
        connector: Box<dyn ResponseConnector>,
        binding: EffectiveModelBinding,
        registry: FrozenToolRegistry,
        services: NativeModelBackendServices,
        model_context: ModelContextProfile,
        config: NativeModelBackendConfig,
    ) -> Result<Self, BackendFailure> {
        if config.system_prompt.is_empty()
            || config.maximum_model_rounds == 0
            || config.maximum_tool_argument_bytes == 0
            || config.maximum_tool_output_bytes < TOOL_TRUNCATION_MARKER.len()
        {
            return Err(failure(
                BackendFailureKind::Initialization,
                "native model-loop configuration contains an empty or invalid bound",
            ));
        }
        if !registry.is_empty() && services.semantic_admission.is_none() {
            return Err(failure(
                BackendFailureKind::Initialization,
                "native local tools require an installed semantic-admission gate",
            ));
        }
        let contract =
            ModelReplayContract::new(config.system_prompt.clone(), registry.replay_tools());
        if !contract.is_valid() {
            return Err(failure(
                BackendFailureKind::Initialization,
                "native model replay contract is invalid or exceeds its bounds",
            ));
        }
        Ok(Self {
            connector,
            binding,
            registry,
            semantic_admission: services.semantic_admission,
            tool_host: services.tool_host,
            config,
            model_context,
            token_counter: services.token_counter,
            contract,
            session: None,
            replay: ModelReplay::default(),
            turn: None,
            events: VecDeque::new(),
            open_activities: HashSet::new(),
            next_activity_id: 1,
            next_request_id: 1,
            shared_stop: Arc::new(SharedStop::default()),
            closed: false,
            context_exhausted: false,
            shutdown_result: None,
        })
    }

    fn binding_evidence(&self, session_id: SessionId) -> BackendBindingEvidence {
        let binding = json!({
            "provider": self.binding.provider_id().as_str(),
            "account": self.binding.account_id().as_str(),
            "model": self.binding.model_id().as_str(),
            "connector": self.binding.connector_id().as_str(),
            "api_protocol": self.binding.api_protocol().as_str(),
            "base_url": self.binding.endpoint().as_str(),
        })
        .to_string();
        BackendBindingEvidence::new(
            BACKEND_KIND,
            BACKEND_VERSION,
            BackendIdentity::new("yo.model-binding/v1", binding),
            BackendIdentity::new("yo.model-id/v1", self.binding.model_id().as_str()),
            BackendIdentity::new("yo.session-id/v1", session_id.to_string()),
            ContinuationStrategy::ExactReplay {
                executor: ReplayExecutor::LocalClient,
            },
        )
    }

    fn start_turn(
        &mut self,
        turn: TurnRef,
        input: String,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        if self.context_exhausted {
            return Err(failure(
                BackendFailureKind::ContextExhausted,
                "context_exhausted: this binding cannot admit another model request",
            ));
        }
        if self.session != Some(turn.session_id()) || self.turn.is_some() {
            return Err(failure(
                BackendFailureKind::Session,
                "native backend requires its bound idle Session before starting a Turn",
            ));
        }
        let delta = vec![ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: input,
        }];
        let mut state = TurnState {
            turn,
            round: 0,
            delta,
            stream: None,
            response_id: None,
            assistant_activities: HashMap::new(),
            reasoning_activities: HashMap::new(),
            call_activities: HashMap::new(),
            seen_call_ids: self
                .replay
                .items()
                .iter()
                .filter_map(|item| match item {
                    ModelReplayItem::FunctionCall { call_id, .. } => Some(call_id.clone()),
                    _ => None,
                })
                .collect(),
            round_message_items: BTreeSet::new(),
            round_messages: BTreeMap::new(),
            round_replay: BTreeMap::new(),
            pending_calls: BTreeMap::new(),
            active_tool: None,
            ready_tool: None,
            dispatch_tool: None,
            awaiting_approval: None,
            start_next_round: false,
        };
        let request_started = match self.start_model_round(&mut state) {
            Ok(()) => {
                self.turn = Some(state);
                true
            },
            Err(error) if error.kind() == BackendFailureKind::ContextExhausted => {
                self.context_exhausted = true;
                self.exhaust_turn(&mut state);
                false
            },
            Err(error) => return Err(error),
        };
        if !request_started {
            return Ok(BackendCommandEvidence::None);
        }
        Ok(BackendCommandEvidence::RequestAccepted(
            BackendRequestEvidence::new(
                "openai.responses/yo-managed-turn/v1",
                BackendIdentity::new("responses.endpoint/v1", self.connector.request_url()),
                BackendIdentity::new(
                    "yo.turn/v1",
                    format!("{}:{}", turn.session_id(), turn.turn_id().get()),
                ),
            ),
        ))
    }

    fn start_model_round(&mut self, state: &mut TurnState) -> Result<(), BackendFailure> {
        if state.round >= self.config.maximum_model_rounds {
            return Err(failure(
                BackendFailureKind::Turn,
                "native model loop exceeded its model-round limit",
            ));
        }
        let mut items = Vec::new();
        items.push(ResponsesInputItem::Message {
            role: ResponsesInputRole::System,
            content: self.contract.system_prompt().to_owned(),
        });
        items.extend(self.replay.items().iter().map(replay_input));
        items.extend(state.delta.iter().map(replay_input));
        let request = ResponsesRequest::new(
            items,
            self.registry
                .function_tools()
                .map_err(|error| failure(BackendFailureKind::Initialization, error.to_string()))?,
            self.config.reasoning_effort,
        )
        .map_err(map_connector_turn)?;
        let input_tokens = self
            .token_counter
            .count_input_tokens(
                self.model_context.tokenizer_profile(),
                &request.tokenization_payload(self.binding.model_id().as_str()),
            )
            .map_err(|_| {
                failure(
                    BackendFailureKind::Turn,
                    "model token counting failed before remote request dispatch",
                )
            })?;
        let admitted_input = self
            .model_context
            .input_token_limit()
            .saturating_sub(self.model_context.output_token_reserve());
        if input_tokens > admitted_input {
            return Err(failure(
                BackendFailureKind::ContextExhausted,
                format!(
                    "context_exhausted: {input_tokens} input tokens exceed the admitted {admitted_input} after output reserve"
                ),
            ));
        }
        let cancellation = ResponsesCancellation::new();
        *self
            .shared_stop
            .response
            .lock()
            .map_err(|_| failure(BackendFailureKind::Cleanup, "native stop state is poisoned"))? =
            Some(cancellation.clone());
        let stream = match self.connector.start(request, cancellation) {
            Ok(stream) => stream,
            Err(error) => {
                *self.shared_stop.response.lock().map_err(|_| {
                    failure(BackendFailureKind::Cleanup, "native stop state is poisoned")
                })? = None;
                return Err(map_connector_turn(error));
            },
        };
        state.stream = Some(stream);
        state.round += 1;
        state.response_id = None;
        state.assistant_activities.clear();
        state.reasoning_activities.clear();
        state.call_activities.clear();
        state.round_message_items.clear();
        state.round_messages.clear();
        state.round_replay.clear();
        Ok(())
    }

    fn next_activity(&mut self, turn: TurnRef) -> Result<ActivityRef, BackendFailure> {
        let id = NonZeroU64::new(self.next_activity_id).ok_or_else(|| {
            failure(
                BackendFailureKind::Turn,
                "native Activity identity exhausted",
            )
        })?;
        self.next_activity_id = self.next_activity_id.checked_add(1).ok_or_else(|| {
            failure(
                BackendFailureKind::Turn,
                "native Activity identity exhausted",
            )
        })?;
        Ok(ActivityRef::new(turn, ActivityId::new(id)))
    }

    fn next_request(&mut self) -> Result<RequestId, BackendFailure> {
        let id = NonZeroU64::new(self.next_request_id).ok_or_else(|| {
            failure(
                BackendFailureKind::Turn,
                "native approval identity exhausted",
            )
        })?;
        self.next_request_id = self.next_request_id.checked_add(1).ok_or_else(|| {
            failure(
                BackendFailureKind::Turn,
                "native approval identity exhausted",
            )
        })?;
        Ok(RequestId::new(id))
    }

    fn queue_activity_text(
        &mut self,
        activity: ActivityRef,
        kind: ActivityKind,
        text: String,
        finish: Option<ActivityOutcome>,
    ) {
        self.events
            .push_back(BackendEvent::ActivityStarted { activity, kind });
        if !text.is_empty() {
            self.events.push_back(BackendEvent::ActivityUpdated {
                activity,
                update: crate::ActivityUpdate::TextSnapshot(text),
            });
        }
        if let Some(outcome) = finish {
            self.events
                .push_back(BackendEvent::ActivityFinished { activity, outcome });
        }
    }

    fn handle_response_event(&mut self, event: ResponsesEvent) -> Result<(), BackendFailure> {
        let mut state = self.turn.take().ok_or_else(|| {
            failure(
                BackendFailureKind::Protocol,
                "response event has no active Turn",
            )
        })?;
        if let Err(error) = self.apply_response_event(&mut state, event) {
            if error.kind() == BackendFailureKind::ContextExhausted {
                self.context_exhausted = true;
                self.exhaust_turn(&mut state);
            } else {
                self.fail_turn(&mut state, error.to_string());
            }
        }
        if self.turn.is_none()
            && !self.events.iter().any(|event| {
                matches!(
                    event,
                    BackendEvent::TurnFinished { .. } | BackendEvent::ResumableTurnFinished { .. }
                )
            })
        {
            self.turn = Some(state);
        }
        Ok(())
    }

    fn apply_response_event(
        &mut self,
        state: &mut TurnState,
        event: ResponsesEvent,
    ) -> Result<(), BackendFailure> {
        match event {
            ResponsesEvent::ResponseCreated { response_id } => {
                state.response_id = Some(response_id);
            },
            ResponsesEvent::TextDelta {
                output_index,
                content_index,
                delta,
                ..
            }
            | ResponsesEvent::RefusalDelta {
                output_index,
                content_index,
                delta,
                ..
            } => {
                let key = (output_index, content_index);
                let activity = if let Some(activity) = state.assistant_activities.get(&key) {
                    *activity
                } else {
                    let activity = self.next_activity(state.turn)?;
                    state.assistant_activities.insert(key, activity);
                    self.events.push_back(BackendEvent::ActivityStarted {
                        activity,
                        kind: ActivityKind::ModelWork,
                    });
                    activity
                };
                if state.round_replay.contains_key(&output_index) {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "Responses output index changed semantic item kind",
                    ));
                }
                state
                    .round_messages
                    .entry(key)
                    .or_default()
                    .push_str(&delta);
                self.events.push_back(BackendEvent::ActivityUpdated {
                    activity,
                    update: crate::ActivityUpdate::TextDelta(delta),
                });
            },
            ResponsesEvent::MessageDone {
                output_index,
                item_id: _,
            } => {
                if state.round_replay.contains_key(&output_index)
                    || !state.round_message_items.insert(output_index)
                {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "Responses output index completed more than one semantic item",
                    ));
                }
            },
            ResponsesEvent::ReasoningDelta {
                output_index,
                part_index,
                channel,
                delta,
                ..
            } => {
                if channel == ReasoningChannel::Summary {
                    let key = (output_index, part_index);
                    let activity = if let Some(activity) = state.reasoning_activities.get(&key) {
                        *activity
                    } else {
                        let activity = self.next_activity(state.turn)?;
                        state.reasoning_activities.insert(key, activity);
                        self.events.push_back(BackendEvent::ActivityStarted {
                            activity,
                            kind: ActivityKind::ModelWork,
                        });
                        activity
                    };
                    self.events.push_back(BackendEvent::ActivityUpdated {
                        activity,
                        update: crate::ActivityUpdate::TextDelta(delta),
                    });
                }
            },
            ResponsesEvent::FunctionCallStarted {
                output_index,
                item_id,
                call_id,
                name,
            } => {
                if state.call_activities.contains_key(&item_id)
                    || !state.seen_call_ids.insert(call_id.clone())
                {
                    let message = "duplicate function item or call identity";
                    let activity = self.next_activity(state.turn)?;
                    self.queue_activity_text(
                        activity,
                        ActivityKind::ToolCall,
                        format!("{name} {call_id}"),
                        Some(ActivityOutcome::Failed(tool_validation_failure(
                            ToolValidationFailure::DuplicateIdentity,
                            message,
                        ))),
                    );
                    self.fail_turn(state, message.to_owned());
                    return Ok(());
                }
                let activity = self.next_activity(state.turn)?;
                state.call_activities.insert(
                    item_id,
                    CallActivity {
                        activity,
                        output_index,
                        call_id,
                        name: name.clone(),
                    },
                );
                self.queue_activity_text(activity, ActivityKind::ToolCall, name, None);
            },
            ResponsesEvent::FunctionArgumentsDelta { .. } => {},
            ResponsesEvent::FunctionCallDone {
                output_index,
                item_id,
                call_id,
                name,
                arguments,
            } => {
                let started = state.call_activities.remove(&item_id).ok_or_else(|| {
                    failure(
                        BackendFailureKind::Protocol,
                        "completed function call was not started",
                    )
                })?;
                if started.output_index != output_index
                    || started.call_id != call_id
                    || started.name != name
                {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "completed function call does not match its start identity",
                    ));
                }
                let activity = started.activity;
                match self.registry.validate_call(
                    call_id.clone(),
                    &name,
                    &arguments,
                    self.config.maximum_tool_argument_bytes,
                ) {
                    Ok(call) => {
                        let admitted_arguments = match self
                            .semantic_admission
                            .as_ref()
                            .expect("a non-empty registry requires semantic admission")
                            .admit_arguments(call.definition(), &arguments)
                        {
                            Ok(admitted)
                                if admitted.len() <= self.config.maximum_tool_argument_bytes
                                    && serde_json::from_str::<serde_json::Value>(&admitted)
                                        .is_ok() =>
                            {
                                admitted
                            },
                            Ok(_) => {
                                let message = "semantic admission returned invalid or oversized argument JSON";
                                self.fail_tool_admission(activity, call_id, name, message);
                                self.fail_turn(state, message.to_owned());
                                return Ok(());
                            },
                            Err(_) => {
                                let message = "tool argument semantic admission was rejected";
                                self.fail_tool_admission(activity, call_id, name, message);
                                self.fail_turn(state, message.to_owned());
                                return Ok(());
                            },
                        };
                        self.events.push_back(BackendEvent::ActivityUpdated {
                            activity,
                            update: crate::ActivityUpdate::TextSnapshot(
                                json!({
                                    "call_id": call_id,
                                    "name": name,
                                    "arguments": admitted_arguments,
                                })
                                .to_string(),
                            ),
                        });
                        if !self.tool_host.is_available(call.definition().id()) {
                            let message = "tool is unavailable on the selected execution host";
                            self.events.push_back(BackendEvent::ActivityFinished {
                                activity,
                                outcome: ActivityOutcome::Failed(tool_validation_failure(
                                    ToolValidationFailure::Unavailable,
                                    message,
                                )),
                            });
                            self.fail_turn(state, message.to_owned());
                            return Ok(());
                        }
                        self.events.push_back(BackendEvent::ActivityFinished {
                            activity,
                            outcome: ActivityOutcome::Completed,
                        });
                        if state
                            .round_replay
                            .insert(
                                output_index,
                                ModelReplayItem::FunctionCall {
                                    call_id,
                                    name,
                                    arguments: admitted_arguments,
                                },
                            )
                            .is_some()
                        {
                            return Err(failure(
                                BackendFailureKind::Protocol,
                                "Responses output index was completed more than once",
                            ));
                        }
                        if state
                            .pending_calls
                            .insert(
                                output_index,
                                PendingCall {
                                    call,
                                    approval: None,
                                },
                            )
                            .is_some()
                        {
                            return Err(failure(
                                BackendFailureKind::Protocol,
                                "Responses output index declared more than one function call",
                            ));
                        }
                    },
                    Err(error) => {
                        let kind = error.kind();
                        let message = durable_tool_validation_message(kind);
                        self.events.push_back(BackendEvent::ActivityUpdated {
                            activity,
                            update: crate::ActivityUpdate::TextSnapshot(
                                json!({
                                    "call_id": call_id,
                                    "name": name,
                                    "validation_failure": {
                                        "code": kind.code(),
                                        "message": message,
                                    },
                                })
                                .to_string(),
                            ),
                        });
                        self.events.push_back(BackendEvent::ActivityFinished {
                            activity,
                            outcome: ActivityOutcome::Failed(tool_validation_failure(
                                kind, message,
                            )),
                        });
                        self.fail_turn(state, message.to_owned());
                    },
                }
            },
            ResponsesEvent::Terminal {
                response_id,
                status,
                usage,
            } => {
                if !state.call_activities.is_empty() {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "Responses terminal arrived with an incomplete function call",
                    ));
                }
                if state.response_id.as_deref() != Some(response_id.as_str()) {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "Responses terminal identity does not match response.created",
                    ));
                }
                if let Some(mut stream) = state.stream.take() {
                    stream.shutdown().map_err(map_connector_cleanup)?;
                }
                *self.shared_stop.response.lock().map_err(|_| {
                    failure(BackendFailureKind::Cleanup, "native stop state is poisoned")
                })? = None;
                let attribution = self.next_activity(state.turn)?;
                self.queue_activity_text(
                    attribution,
                    ActivityKind::ModelWork,
                    json!({
                        "response_id": response_id,
                        "round": state.round,
                        "provider": self.binding.provider_id().as_str(),
                        "account": self.binding.account_id().as_str(),
                        "model": self.binding.model_id().as_str(),
                        "connector": self.binding.connector_id().as_str(),
                        "api_protocol": self.binding.api_protocol().as_str(),
                        "base_url": self.binding.endpoint().as_str(),
                        "usage": {
                            "input_tokens": usage.input_tokens,
                            "output_tokens": usage.output_tokens,
                            "total_tokens": usage.total_tokens,
                            "reasoning_tokens": usage.reasoning_tokens,
                        },
                    })
                    .to_string(),
                    Some(ActivityOutcome::Completed),
                );
                for activity in state
                    .assistant_activities
                    .values()
                    .chain(state.reasoning_activities.values())
                {
                    self.events.push_back(BackendEvent::ActivityFinished {
                        activity: *activity,
                        outcome: terminal_activity_outcome(&status),
                    });
                }
                let mut messages = BTreeMap::<usize, String>::new();
                for ((output_index, _), content) in std::mem::take(&mut state.round_messages) {
                    messages.entry(output_index).or_default().push_str(&content);
                }
                for output_index in std::mem::take(&mut state.round_message_items) {
                    let content = messages.remove(&output_index).unwrap_or_default();
                    if state
                        .round_replay
                        .insert(
                            output_index,
                            ModelReplayItem::Message {
                                role: ModelReplayRole::Assistant,
                                content,
                            },
                        )
                        .is_some()
                    {
                        return Err(failure(
                            BackendFailureKind::Protocol,
                            "Responses output index changed semantic item kind",
                        ));
                    }
                }
                if !messages.is_empty() {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "Responses message text completed without its message output item",
                    ));
                }
                state
                    .delta
                    .extend(std::mem::take(&mut state.round_replay).into_values());
                match status {
                    ResponseTerminal::Completed if state.pending_calls.is_empty() => {
                        if state.delta.last().is_some_and(|item| {
                            matches!(
                                item,
                                ModelReplayItem::Message {
                                    role: ModelReplayRole::Assistant,
                                    ..
                                }
                            )
                        }) {
                            self.complete_turn(state)?;
                        } else {
                            self.fail_turn(
                                state,
                                "completed model response did not contain a final assistant message"
                                    .to_owned(),
                            );
                        }
                    },
                    ResponseTerminal::Completed => self.advance_tool_queue(state)?,
                    ResponseTerminal::Incomplete { reason } => self.fail_turn(
                        state,
                        format!(
                            "model response was incomplete: {}",
                            reason.unwrap_or_else(|| "unknown reason".to_owned())
                        ),
                    ),
                    ResponseTerminal::Failed { code } => self.fail_turn(
                        state,
                        format!(
                            "model response failed: {}",
                            code.unwrap_or_else(|| "unknown code".to_owned())
                        ),
                    ),
                }
            },
        }
        Ok(())
    }

    fn advance_tool_queue(&mut self, state: &mut TurnState) -> Result<(), BackendFailure> {
        let Some((_, mut pending)) = state.pending_calls.pop_first() else {
            state.start_next_round = true;
            return Ok(());
        };
        if pending.call.definition().approval() == ToolApprovalRequirement::Required {
            let activity = self.next_activity(state.turn)?;
            let request = ActivityRequestRef::new(activity, self.next_request()?);
            let binding =
                ToolApprovalBinding::new(state.turn, &pending.call, self.tool_host.identity());
            let approval_text = json!({
                "call_id": pending.call.call_id(),
                "tool_id": pending.call.definition().id().as_str(),
                "argument_digest": binding.argument_digest_hex(),
                "effect": format!("{:?}", binding.effect()),
                "execution_host": binding.execution_host(),
            })
            .to_string();
            pending.approval = Some(binding);
            self.queue_activity_text(
                activity,
                ActivityKind::ApprovalRequest {
                    request_id: request.request_id(),
                },
                approval_text,
                None,
            );
            state.awaiting_approval = Some((request, pending));
            return Ok(());
        }
        state.ready_tool = Some(pending.call);
        Ok(())
    }

    fn fail_tool_admission(
        &mut self,
        activity: ActivityRef,
        call_id: String,
        name: String,
        message: &str,
    ) {
        self.events.push_back(BackendEvent::ActivityUpdated {
            activity,
            update: crate::ActivityUpdate::TextSnapshot(
                json!({
                    "call_id": call_id,
                    "name": name,
                    "validation_failure": {
                        "code": ToolValidationFailure::SemanticAdmission.code(),
                        "message": message,
                    },
                })
                .to_string(),
            ),
        });
        self.events.push_back(BackendEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Failed(tool_validation_failure(
                ToolValidationFailure::SemanticAdmission,
                message,
            )),
        });
    }

    fn start_tool_execution(
        &mut self,
        state: &mut TurnState,
        call: ValidatedToolCall,
        activity: ActivityRef,
    ) -> Result<(), BackendFailure> {
        let request = ToolExecutionRequest {
            turn: state.turn,
            call: call.clone(),
            maximum_output_bytes: self.config.maximum_tool_output_bytes,
        };
        match self.tool_host.start(request) {
            Ok(execution) => {
                state.active_tool = Some(ActiveTool {
                    call,
                    activity,
                    execution,
                });
            },
            Err(_) => self.finish_tool(
                state,
                call,
                activity,
                ToolExecutionOutcome::Failed,
                "tool execution failed".to_owned(),
            )?,
        }
        Ok(())
    }

    fn poll_tool(&mut self) -> Result<(), BackendFailure> {
        let mut state = self.turn.take().ok_or_else(|| {
            failure(
                BackendFailureKind::Protocol,
                "tool execution has no active Turn",
            )
        })?;
        if let Err(error) = self.poll_tool_state(&mut state) {
            self.fail_turn(&mut state, error.to_string());
        } else if self.turn.is_none()
            && !self.events.iter().any(|event| {
                matches!(
                    event,
                    BackendEvent::TurnFinished { .. } | BackendEvent::ResumableTurnFinished { .. }
                )
            })
        {
            self.turn = Some(state);
        }
        Ok(())
    }

    fn poll_tool_state(&mut self, state: &mut TurnState) -> Result<(), BackendFailure> {
        let mut active = state.active_tool.take().ok_or_else(|| {
            failure(
                BackendFailureKind::Protocol,
                "tool poll has no active execution",
            )
        })?;
        let poll = match active.execution.poll() {
            Ok(poll) => poll,
            Err(error) => {
                state.active_tool = Some(active);
                return Err(map_tool_turn(error));
            },
        };
        match poll {
            ToolExecutionPoll::Pending => state.active_tool = Some(active),
            ToolExecutionPoll::Ready => {
                let Some(result) = active.execution.take_result() else {
                    state.active_tool = Some(active);
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "ready tool execution has no result",
                    ));
                };
                if let Err(error) = active.execution.shutdown() {
                    state.active_tool = Some(active);
                    return Err(map_tool_cleanup(error));
                }
                self.finish_tool(
                    state,
                    active.call,
                    active.activity,
                    result.outcome(),
                    bounded_output(
                        result.output(),
                        self.config.maximum_tool_output_bytes,
                        result.truncated(),
                    ),
                )?;
            },
        }
        Ok(())
    }

    fn finish_tool_without_execution(
        &mut self,
        state: &mut TurnState,
        call: ValidatedToolCall,
        outcome: ToolExecutionOutcome,
        output: String,
    ) -> Result<(), BackendFailure> {
        let activity = self.next_activity(state.turn)?;
        self.events.push_back(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ToolResult,
        });
        self.finish_tool(state, call, activity, outcome, output)
    }

    fn finish_tool(
        &mut self,
        state: &mut TurnState,
        call: ValidatedToolCall,
        activity: ActivityRef,
        outcome: ToolExecutionOutcome,
        output: String,
    ) -> Result<(), BackendFailure> {
        let output = match self
            .semantic_admission
            .as_ref()
            .expect("an executed local tool requires semantic admission")
            .admit_output(call.definition(), &output)
        {
            Ok(admitted) if admitted.len() <= self.config.maximum_tool_output_bytes => admitted,
            Ok(_) => {
                let message = "semantic admission returned oversized tool output";
                self.fail_tool_admission(
                    activity,
                    call.call_id().to_owned(),
                    call.definition().wire_name().to_owned(),
                    message,
                );
                return Err(failure(BackendFailureKind::Protocol, message));
            },
            Err(_) => {
                let message = "tool output semantic admission was rejected";
                self.fail_tool_admission(
                    activity,
                    call.call_id().to_owned(),
                    call.definition().wire_name().to_owned(),
                    message,
                );
                return Err(failure(BackendFailureKind::Protocol, message));
            },
        };
        let activity_outcome = match outcome {
            ToolExecutionOutcome::Completed => ActivityOutcome::Completed,
            ToolExecutionOutcome::Failed => {
                ActivityOutcome::Failed(Failure::new("tool execution failed"))
            },
            ToolExecutionOutcome::Interrupted => ActivityOutcome::Interrupted,
        };
        self.events.push_back(BackendEvent::ActivityUpdated {
            activity,
            update: crate::ActivityUpdate::TextSnapshot(
                json!({ "call_id": call.call_id(), "output": output }).to_string(),
            ),
        });
        self.events.push_back(BackendEvent::ActivityFinished {
            activity,
            outcome: activity_outcome,
        });
        state.delta.push(ModelReplayItem::FunctionCallOutput {
            call_id: call.call_id().to_owned(),
            output,
        });
        if state.pending_calls.is_empty() {
            state.start_next_round = true;
        } else {
            self.advance_tool_queue(state)?;
        }
        Ok(())
    }

    fn complete_turn(&mut self, state: &mut TurnState) -> Result<(), BackendFailure> {
        let delta = ModelReplayDelta::new(
            self.replay
                .contract()
                .is_none()
                .then(|| self.contract.clone()),
            std::mem::take(&mut state.delta),
        );
        self.replay.apply(&delta).map_err(|message| {
            let kind = if is_replay_capacity_error(message) {
                BackendFailureKind::ContextExhausted
            } else {
                BackendFailureKind::Turn
            };
            failure(kind, message)
        })?;
        self.events.push_back(BackendEvent::ResumableTurnFinished {
            turn: state.turn,
            evidence: BackendOutcomeEvidence::with_identity(BackendIdentity::new(
                "responses.response-id/v1",
                state
                    .response_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_owned()),
            ))
            .with_replay(delta),
        });
        self.turn = None;
        Ok(())
    }

    fn cleanup_turn_resources(&mut self, state: &mut TurnState) -> Vec<String> {
        let mut diagnostics = Vec::new();
        if let Some(mut stream) = state.stream.take() {
            stream.cancel();
            if stream.shutdown().is_err() {
                diagnostics.push("response cleanup failed".to_owned());
            }
        }
        match self.shared_stop.response.lock() {
            Ok(mut response) => *response = None,
            Err(_) => diagnostics.push("native stop state is poisoned".to_owned()),
        }
        if let Some(mut active) = state.active_tool.take() {
            active.execution.cancel();
            if active.execution.shutdown().is_err() {
                diagnostics.push("tool execution cleanup failed".to_owned());
            }
        }
        diagnostics
    }

    fn projected_open_activities(&self) -> BTreeSet<ActivityRef> {
        let mut activities = self
            .open_activities
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        for event in &self.events {
            match event {
                BackendEvent::ActivityStarted { activity, .. } => {
                    activities.insert(*activity);
                },
                BackendEvent::ActivityFinished { activity, .. } => {
                    activities.remove(activity);
                },
                BackendEvent::TurnFinished { .. } | BackendEvent::ResumableTurnFinished { .. } => {
                    activities.clear()
                },
                BackendEvent::ActivityUpdated { .. } => {},
            }
        }
        activities
    }

    fn fail_turn(&mut self, state: &mut TurnState, mut message: String) {
        let active_tool = state
            .active_tool
            .as_ref()
            .map(|active| (active.activity, active.call.call_id().to_owned()));
        let diagnostics = self.cleanup_turn_resources(state);
        if !diagnostics.is_empty() {
            message.push_str("; ");
            message.push_str(&diagnostics.join("; "));
        }
        for activity in self.projected_open_activities() {
            if let Some((active_activity, call_id)) = active_tool.as_ref()
                && *active_activity == activity
            {
                self.events.push_back(BackendEvent::ActivityUpdated {
                    activity,
                    update: crate::ActivityUpdate::TextSnapshot(
                        json!({
                            "call_id": call_id,
                            "error": "execution failed or was cancelled; effect may be uncertain",
                        })
                        .to_string(),
                    ),
                });
            }
            self.events.push_back(BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Failed(Failure::new(message.clone())),
            });
        }
        self.events.push_back(BackendEvent::TurnFinished {
            turn: state.turn,
            outcome: TurnOutcome::Failed(Failure::new(message)),
        });
        self.turn = None;
    }

    fn exhaust_turn(&mut self, state: &mut TurnState) {
        let diagnostics = self.cleanup_turn_resources(state);
        if diagnostics.is_empty() {
            for activity in self.projected_open_activities() {
                self.events.push_back(BackendEvent::ActivityFinished {
                    activity,
                    outcome: ActivityOutcome::Completed,
                });
            }
            self.events.push_back(BackendEvent::TurnFinished {
                turn: state.turn,
                outcome: TurnOutcome::Completed,
            });
            self.turn = None;
        } else {
            self.fail_turn(state, diagnostics.join("; "));
        }
    }

    fn interrupt(&mut self, turn: TurnRef) -> Result<BackendCommandEvidence, BackendFailure> {
        let Some(mut state) = self.turn.take() else {
            return Err(failure(
                BackendFailureKind::Turn,
                "no active Turn to interrupt",
            ));
        };
        if state.turn != turn {
            self.turn = Some(state);
            return Err(failure(
                BackendFailureKind::Turn,
                "interrupt names a different Turn",
            ));
        }
        let interrupted_call = state
            .active_tool
            .as_ref()
            .map(|active| (active.activity, active.call.call_id().to_owned()));
        let cleanup_diagnostics = self.cleanup_turn_resources(&mut state);
        self.events.clear();
        let open_activities = std::mem::take(&mut self.open_activities)
            .into_iter()
            .collect::<BTreeSet<_>>();
        for activity in open_activities {
            if let Some((interrupted_activity, call_id)) = interrupted_call.as_ref()
                && *interrupted_activity == activity
            {
                self.events.push_back(BackendEvent::ActivityUpdated {
                    activity,
                    update: crate::ActivityUpdate::TextSnapshot(
                        json!({
                            "call_id": call_id,
                            "error": "execution interrupted; effect may be uncertain",
                        })
                        .to_string(),
                    ),
                });
            }
            self.events.push_back(BackendEvent::ActivityFinished {
                activity,
                outcome: if cleanup_diagnostics.is_empty() {
                    ActivityOutcome::Interrupted
                } else {
                    ActivityOutcome::Failed(Failure::new(cleanup_diagnostics.join("; ")))
                },
            });
        }
        self.events.push_back(BackendEvent::TurnFinished {
            turn,
            outcome: if cleanup_diagnostics.is_empty() {
                TurnOutcome::Interrupted
            } else {
                TurnOutcome::Failed(Failure::new(cleanup_diagnostics.join("; ")))
            },
        });
        Ok(BackendCommandEvidence::None)
    }

    fn respond_to_approval(
        &mut self,
        request: ActivityRequestRef,
        decision: ApprovalDecision,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        let mut state = self.turn.take().ok_or_else(|| {
            failure(
                BackendFailureKind::Turn,
                "approval response has no active Turn",
            )
        })?;
        let Some((expected, pending)) = state.awaiting_approval.take() else {
            self.turn = Some(state);
            return Err(failure(
                BackendFailureKind::Turn,
                "no approval is awaiting a response",
            ));
        };
        if expected != request
            || !pending.approval.as_ref().is_some_and(|binding| {
                binding.matches(state.turn, &pending.call, self.tool_host.identity())
            })
        {
            state.awaiting_approval = Some((expected, pending));
            self.turn = Some(state);
            return Err(failure(
                BackendFailureKind::Turn,
                "approval response does not match the exact tool execution binding",
            ));
        }
        self.events.push_back(BackendEvent::ActivityFinished {
            activity: request.activity(),
            outcome: ActivityOutcome::Completed,
        });
        let response_activity = self.next_activity(state.turn)?;
        self.queue_activity_text(
            response_activity,
            ActivityKind::ApprovalResponse {
                request_id: request.request_id(),
            },
            format!("{decision:?}"),
            Some(ActivityOutcome::Completed),
        );
        match decision {
            ApprovalDecision::Approved => state.ready_tool = Some(pending.call),
            ApprovalDecision::Declined => self.finish_tool_without_execution(
                &mut state,
                pending.call,
                ToolExecutionOutcome::Failed,
                r#"{"error":"tool approval declined"}"#.to_owned(),
            )?,
        }
        self.turn = Some(state);
        Ok(BackendCommandEvidence::None)
    }

    fn pop_event(&mut self) -> Option<BackendEvent> {
        let event = self.events.pop_front()?;
        match &event {
            BackendEvent::ActivityStarted { activity, .. } => {
                self.open_activities.insert(*activity);
            },
            BackendEvent::ActivityFinished { activity, .. } => {
                self.open_activities.remove(activity);
            },
            BackendEvent::TurnFinished { .. } | BackendEvent::ResumableTurnFinished { .. } => {
                self.open_activities.clear();
            },
            BackendEvent::ActivityUpdated { .. } => {},
        }
        Some(event)
    }
}

fn is_replay_capacity_error(message: &str) -> bool {
    matches!(
        message,
        "model replay delta is invalid or exceeds its bounds"
            | "model replay item limit exceeded"
            | "model replay prefix byte limit exceeded"
    )
}

impl AgentBackend for NativeModelBackend {
    fn stop_handle(&self) -> BackendStopHandle {
        let shared = Arc::clone(&self.shared_stop);
        BackendStopHandle::new(move || {
            shared.requested.store(true, Ordering::Release);
            if let Ok(guard) = shared.response.lock()
                && let Some(cancellation) = guard.as_ref()
            {
                cancellation.cancel();
            }
        })
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::none()
    }

    fn resume_session(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<BackendBindingEvidence, BackendFailure> {
        if self.closed || self.session.is_some() || self.turn.is_some() {
            return Err(failure(
                BackendFailureKind::Session,
                "native backend is not available for resume",
            ));
        }
        let expected = self.binding_evidence(target.session_id());
        if !expected.same_resume_identity(target.binding())
            || target.model_replay().contract() != Some(&self.contract)
        {
            return Err(failure(
                BackendFailureKind::Session,
                "durable native model binding or replay contract does not match current configuration",
            ));
        }
        self.session = Some(target.session_id());
        self.replay = target.model_replay().clone();
        Ok(expected)
    }

    fn execute_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        if self.closed {
            return Err(failure(
                BackendFailureKind::Session,
                "native backend is closed",
            ));
        }
        match command {
            AgentCommand::CreateSession { session_id } => {
                if self.session.is_some() {
                    return Err(failure(
                        BackendFailureKind::Session,
                        "native backend already has a Session",
                    ));
                }
                self.session = Some(session_id);
                Ok(BackendCommandEvidence::BindingOpened(
                    self.binding_evidence(session_id),
                ))
            },
            AgentCommand::StartTurn { turn, input } => self.start_turn(turn, input.into_string()),
            AgentCommand::SteerTurn { .. } => Err(failure(
                BackendFailureKind::Unsupported,
                "native model loop does not support steering",
            )),
            AgentCommand::InterruptTurn { turn } => self.interrupt(turn),
            AgentCommand::RespondToActivity {
                request,
                response: ActivityResponse::Approval(decision),
            } => self.respond_to_approval(request, decision),
            AgentCommand::RespondToActivity { .. } => Err(failure(
                BackendFailureKind::Unsupported,
                "native model loop only accepts approval responses",
            )),
        }
    }

    fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        if let Some(event) = self.pop_event() {
            return Ok(BackendPoll::Event(event));
        }
        if self.closed {
            return Ok(BackendPoll::Closed);
        }
        if self.shared_stop.requested.swap(false, Ordering::AcqRel)
            && let Some(turn) = self.turn.as_ref().map(|state| state.turn)
        {
            self.interrupt(turn)?;
            return Ok(BackendPoll::Event(
                self.pop_event().expect("interrupt queues an event"),
            ));
        }
        if self
            .turn
            .as_ref()
            .is_some_and(|state| state.active_tool.is_some())
        {
            self.poll_tool()?;
        } else if self
            .turn
            .as_ref()
            .is_some_and(|state| state.dispatch_tool.is_some())
        {
            let mut state = self.turn.take().expect("active Turn was checked");
            let (call, activity) = state
                .dispatch_tool
                .take()
                .expect("dispatch-ready tool was checked");
            if let Err(error) = self.start_tool_execution(&mut state, call, activity) {
                self.fail_turn(&mut state, error.to_string());
            } else {
                self.turn = Some(state);
            }
        } else if self
            .turn
            .as_ref()
            .is_some_and(|state| state.ready_tool.is_some())
        {
            let mut state = self.turn.take().expect("active Turn was checked");
            let call = state.ready_tool.take().expect("ready tool was checked");
            match self.next_activity(state.turn) {
                Ok(activity) => {
                    self.queue_activity_text(
                        activity,
                        ActivityKind::ToolResult,
                        json!({
                            "call_id": call.call_id(),
                            "tool_id": call.definition().id().as_str(),
                            "execution_host": self.tool_host.identity(),
                            "attempt": 1,
                        })
                        .to_string(),
                        None,
                    );
                    state.dispatch_tool = Some((call, activity));
                    self.turn = Some(state);
                },
                Err(error) => self.fail_turn(&mut state, error.to_string()),
            }
        } else if self
            .turn
            .as_ref()
            .is_some_and(|state| state.start_next_round)
        {
            let mut state = self.turn.take().expect("active Turn was checked");
            state.start_next_round = false;
            if let Err(error) = self.start_model_round(&mut state) {
                if error.kind() == BackendFailureKind::ContextExhausted {
                    self.context_exhausted = true;
                    self.exhaust_turn(&mut state);
                } else {
                    self.fail_turn(&mut state, error.to_string());
                }
            } else {
                self.turn = Some(state);
            }
        } else if self
            .turn
            .as_ref()
            .is_some_and(|state| state.stream.is_some())
        {
            let poll = {
                let state = self.turn.as_mut().expect("active Turn was checked");
                state
                    .stream
                    .as_mut()
                    .expect("response stream was checked")
                    .poll()
            };
            match poll {
                Err(error) => {
                    let mut state = self.turn.take().expect("active Turn was checked");
                    self.fail_turn(&mut state, map_connector_turn(error).to_string());
                },
                Ok(ResponsesPoll::Event(event)) => self.handle_response_event(event)?,
                Ok(ResponsesPoll::Closed) => {
                    let mut state = self.turn.take().expect("active Turn was checked");
                    self.fail_turn(
                        &mut state,
                        "Responses stream closed without a terminal event".to_owned(),
                    );
                },
                Ok(ResponsesPoll::Pending) => {},
            }
        }
        Ok(self
            .pop_event()
            .map_or(BackendPoll::Pending, BackendPoll::Event))
    }

    fn shutdown(&mut self) -> Result<(), BackendFailure> {
        if let Some(result) = &self.shutdown_result {
            return result.clone();
        }
        let mut result = Ok(());
        if let Some(mut state) = self.turn.take() {
            if let Some(mut stream) = state.stream.take() {
                stream.cancel();
                if let Err(error) = stream.shutdown() {
                    result = Err(map_connector_cleanup(error));
                }
            }
            if let Some(mut active) = state.active_tool.take() {
                active.execution.cancel();
                if let Err(error) = active.execution.shutdown()
                    && result.is_ok()
                {
                    result = Err(map_tool_cleanup(error));
                }
            }
        }
        if let Err(error) = self.tool_host.shutdown()
            && result.is_ok()
        {
            result = Err(map_tool_cleanup(error));
        }
        self.events.clear();
        self.open_activities.clear();
        self.closed = true;
        self.shutdown_result = Some(result.clone());
        result
    }
}

fn replay_input(item: &ModelReplayItem) -> ResponsesInputItem {
    match item {
        ModelReplayItem::Message { role, content } => ResponsesInputItem::Message {
            role: match role {
                ModelReplayRole::System => ResponsesInputRole::System,
                ModelReplayRole::Developer => ResponsesInputRole::Developer,
                ModelReplayRole::User => ResponsesInputRole::User,
                ModelReplayRole::Assistant => ResponsesInputRole::Assistant,
            },
            content: content.clone(),
        },
        ModelReplayItem::FunctionCall {
            call_id,
            name,
            arguments,
        } => ResponsesInputItem::FunctionCall {
            call_id: call_id.clone(),
            name: name.clone(),
            arguments: arguments.clone(),
        },
        ModelReplayItem::FunctionCallOutput { call_id, output } => {
            ResponsesInputItem::FunctionCallOutput {
                call_id: call_id.clone(),
                output: output.clone(),
            }
        },
    }
}

fn terminal_activity_outcome(status: &ResponseTerminal) -> ActivityOutcome {
    match status {
        ResponseTerminal::Completed => ActivityOutcome::Completed,
        ResponseTerminal::Incomplete { .. } | ResponseTerminal::Failed { .. } => {
            ActivityOutcome::Failed(Failure::new("model response did not complete"))
        },
    }
}

fn bounded_output(output: &str, limit: usize, already_truncated: bool) -> String {
    if output.len() <= limit && !already_truncated {
        return output.to_owned();
    }
    if limit <= TOOL_TRUNCATION_MARKER.len() {
        let mut end = limit;
        while end > 0 && !TOOL_TRUNCATION_MARKER.is_char_boundary(end) {
            end -= 1;
        }
        return TOOL_TRUNCATION_MARKER[..end].to_owned();
    }
    let mut end = output.len().min(limit - TOOL_TRUNCATION_MARKER.len());
    while end > 0 && !output.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{TOOL_TRUNCATION_MARKER}", &output[..end])
}

fn map_connector_initialization(error: crate::ConnectorError) -> BackendFailure {
    failure(BackendFailureKind::Initialization, error.to_string())
}

fn map_connector_turn(error: crate::ConnectorError) -> BackendFailure {
    failure(BackendFailureKind::Turn, error.to_string())
}

fn map_connector_cleanup(error: crate::ConnectorError) -> BackendFailure {
    failure(BackendFailureKind::Cleanup, error.to_string())
}

fn map_tool_turn(_error: crate::ToolExecutionError) -> BackendFailure {
    failure(BackendFailureKind::Turn, "tool execution failed")
}

fn map_tool_cleanup(_error: crate::ToolExecutionError) -> BackendFailure {
    failure(BackendFailureKind::Cleanup, "tool execution cleanup failed")
}

fn tool_validation_failure(kind: ToolValidationFailure, message: &str) -> Failure {
    Failure::new(message)
        .with_code(kind.code())
        .expect("tool validation codes are stable ASCII identifiers")
}

fn durable_tool_validation_message(kind: ToolValidationFailure) -> &'static str {
    match kind {
        ToolValidationFailure::InvalidIdentity => "tool identity is invalid",
        ToolValidationFailure::ArgumentLimit => "tool arguments exceed the admitted limit",
        ToolValidationFailure::InvalidJson => "tool arguments are not valid JSON",
        ToolValidationFailure::SchemaMismatch => "tool arguments do not match the admitted schema",
        ToolValidationFailure::UnknownTool => "the requested tool is not admitted",
        ToolValidationFailure::DuplicateIdentity => "tool call identity was already used",
        ToolValidationFailure::Unavailable => "the requested tool is unavailable",
        ToolValidationFailure::ApprovalMismatch => "tool approval does not match the request",
        ToolValidationFailure::SemanticAdmission => "tool semantic admission was rejected",
    }
}

fn failure(kind: BackendFailureKind, message: impl Into<String>) -> BackendFailure {
    BackendFailure::new(kind, message)
}

#[cfg(test)]
mod tests;
