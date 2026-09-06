//! Managed binding state, host services, and backend construction.

mod adapter;
mod compaction;
mod context;
mod identity;
mod replay;
mod request;
mod response;
mod tools;
mod turn;

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    num::NonZeroU64,
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::Duration,
};

use serde_json::json;
use yo_core::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef,
    AdmittedReplayProfile, AdmittedToolPolicy, BackendBindingEvidence, BackendEvent,
    BackendFailure, BackendFailureKind, BackendIdentity, CompleteModelBinding,
    ContextPolicyChanged, ContextStrategy, ContinuationStrategy, EffectiveModelBinding,
    EffectiveModelProfile, Failure, FrozenToolRegistry, ModelBindingAdmission, ModelCatalogEntry,
    ModelConnector, ModelConnectorCancellation, ModelConnectorStreamPort, ModelContextProfile,
    ModelReplay, ModelReplayContract, ModelReplayItem, ModelTokenCounter, ReasoningEffort,
    ReplayExecutor, ReplayProfile, RequestId, SessionId, ToolApprovalBinding, ToolExecution,
    ToolExecutionHost, ToolSemanticAdmission, TurnRef, ValidatedToolCall,
};

use self::identity::native_binding_identity;

const BACKEND_KIND: &str = "yo-managed-model";
const BACKEND_VERSION: &str = "1";
const TOOL_TRUNCATION_MARKER: &str = "\n[yo: tool output truncated]";
const CONTEXT_EXHAUSTED_CODE: &str = "context_exhausted";

#[derive(Clone, Debug)]
pub struct NativeModelBackendConfig {
    pub system_prompt: String,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub maximum_model_rounds: usize,
    pub maximum_tool_argument_bytes: usize,
    pub maximum_tool_output_bytes: usize,
    pub absolute_tool_execution_timeout: Option<Duration>,
    pub context_policy: ContextPolicyChanged,
}

impl Default for NativeModelBackendConfig {
    fn default() -> Self {
        Self {
            system_prompt: "You are Yo, a careful software-engineering agent.".to_owned(),
            reasoning_effort: Some(ReasoningEffort::Medium),
            maximum_model_rounds: 32,
            maximum_tool_argument_bytes: 4 * 1024 * 1024,
            maximum_tool_output_bytes: 4 * 1024 * 1024,
            absolute_tool_execution_timeout: None,
            context_policy: ContextPolicyChanged::try_new(
                1,
                true,
                ContextStrategy::PortableSummaryV1Alpha1,
                85,
                90,
                Some(10),
                Some(65_536),
            )
            .expect("the built-in context policy is valid"),
        }
    }
}

/// Host-owned services used by the provider-neutral model loop.
pub struct NativeModelBackendServices {
    binding_admission: Box<dyn ModelBindingAdmission>,
    semantic_admission: Option<Box<dyn ToolSemanticAdmission>>,
    tool_host: Box<dyn ToolExecutionHost>,
    token_counter: Box<dyn ModelTokenCounter>,
    request_observer: Option<Box<dyn ModelRequestObserver>>,
}

/// Host-owned sink for one typed actual model-request outcome.
pub trait ModelRequestObserver: Send {
    fn observe(&mut self, outcome: yo_core::ModelRequestOutcome) -> Result<(), String>;
}

impl<F> ModelRequestObserver for F
where
    F: FnMut(yo_core::ModelRequestOutcome) -> Result<(), String> + Send,
{
    fn observe(&mut self, outcome: yo_core::ModelRequestOutcome) -> Result<(), String> {
        self(outcome)
    }
}

impl NativeModelBackendServices {
    pub fn new(
        binding_admission: Box<dyn ModelBindingAdmission>,
        semantic_admission: Option<Box<dyn ToolSemanticAdmission>>,
        tool_host: Box<dyn ToolExecutionHost>,
        token_counter: Box<dyn ModelTokenCounter>,
    ) -> Self {
        Self {
            binding_admission,
            semantic_admission,
            tool_host,
            token_counter,
            request_observer: None,
        }
    }

    #[must_use]
    pub fn with_model_request_observer(
        mut self,
        observer: impl ModelRequestObserver + 'static,
    ) -> Self {
        self.request_observer = Some(Box::new(observer));
        self
    }
}

#[derive(Default)]
struct SharedStop {
    requested: AtomicBool,
    response: Mutex<Option<ModelConnectorCancellation>>,
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

enum CompactionState {
    Summarizing {
        input_tokens_before: u64,
        summarized_groups: Vec<Vec<ModelReplayItem>>,
        retained_groups: Vec<Vec<ModelReplayItem>>,
        body: String,
        response_id: Option<String>,
        message_done: bool,
    },
    AwaitingCheckpoint {
        replay: ModelReplay,
    },
}

enum IdleCompactionState {
    Summarizing {
        input_tokens_before: u64,
        summarized_groups: Vec<Vec<ModelReplayItem>>,
        retained_groups: Vec<Vec<ModelReplayItem>>,
        body: String,
        response_id: Option<String>,
        message_done: bool,
        stream: Box<dyn ModelConnectorStreamPort>,
    },
    AwaitingCheckpoint {
        replay: ModelReplay,
    },
}

struct TurnState {
    turn: TurnRef,
    round: usize,
    delta: Vec<ModelReplayItem>,
    stream: Option<Box<dyn ModelConnectorStreamPort>>,
    response_id: Option<String>,
    assistant_activities: BTreeMap<usize, ActivityRef>,
    reasoning_activities: HashMap<(usize, usize), ActivityRef>,
    call_activities: HashMap<String, CallActivity>,
    seen_call_ids: HashSet<String>,
    round_message_items: BTreeSet<usize>,
    round_messages: BTreeMap<(usize, usize), String>,
    round_refusals: BTreeMap<(usize, usize), String>,
    round_replay: BTreeMap<usize, ModelReplayItem>,
    pending_calls: BTreeMap<usize, PendingCall>,
    active_tool: Option<ActiveTool>,
    ready_tool: Option<ValidatedToolCall>,
    dispatch_tool: Option<(ValidatedToolCall, ActivityRef)>,
    awaiting_approval: Option<(ActivityRequestRef, PendingCall)>,
    start_next_round: bool,
    compaction: Option<CompactionState>,
    compaction_attempted: bool,
}

pub struct NativeModelBackend {
    connector: Box<dyn ModelConnector>,
    binding: EffectiveModelBinding,
    binding_identity: BackendIdentity,
    registry: FrozenToolRegistry,
    tool_exposure_enabled: bool,
    semantic_admission: Option<Box<dyn ToolSemanticAdmission>>,
    tool_host: Box<dyn ToolExecutionHost>,
    config: NativeModelBackendConfig,
    model_context: ModelContextProfile,
    token_counter: Box<dyn ModelTokenCounter>,
    request_observer: Option<Box<dyn ModelRequestObserver>>,
    contract: ModelReplayContract,
    replay_profile: ReplayProfile,
    session: Option<SessionId>,
    replay: ModelReplay,
    replay_groups: Vec<Vec<ModelReplayItem>>,
    context_policy_active: bool,
    turn: Option<TurnState>,
    idle_compaction: Option<IdleCompactionState>,
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
        connector: Box<dyn ModelConnector>,
        registry: FrozenToolRegistry,
        services: NativeModelBackendServices,
        config: NativeModelBackendConfig,
    ) -> Result<Self, BackendFailure> {
        let binding = catalog_entry.binding().clone();
        Self::with_connector_and_profile(
            connector,
            binding,
            registry,
            services,
            catalog_entry.context().clone(),
            catalog_entry.explicit_profile().cloned(),
            config,
        )
    }

    #[cfg(test)]
    fn with_connector(
        connector: Box<dyn ModelConnector>,
        binding: EffectiveModelBinding,
        registry: FrozenToolRegistry,
        services: NativeModelBackendServices,
        model_context: ModelContextProfile,
        config: NativeModelBackendConfig,
    ) -> Result<Self, BackendFailure> {
        Self::with_connector_and_profile(
            connector,
            binding,
            registry,
            services,
            model_context,
            None,
            config,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn with_connector_and_profile(
        connector: Box<dyn ModelConnector>,
        binding: EffectiveModelBinding,
        registry: FrozenToolRegistry,
        services: NativeModelBackendServices,
        model_context: ModelContextProfile,
        explicit_profile: Option<EffectiveModelProfile>,
        mut config: NativeModelBackendConfig,
    ) -> Result<Self, BackendFailure> {
        let (tool_exposure_enabled, replay_profile) = if let Some(profile) =
            explicit_profile.as_ref()
        {
            let complete = CompleteModelBinding::new(binding.clone(), profile.clone())
                .map_err(|error| failure(BackendFailureKind::Initialization, error.to_string()))?;
            let admitted = services
                .binding_admission
                .admit(&complete)
                .map_err(|message| failure(BackendFailureKind::Initialization, message))?;
            config.reasoning_effort = admitted.profile().reasoning_effort();
            let tools = match admitted.profile().tool_policy() {
                AdmittedToolPolicy::LocalTools => !registry.is_empty(),
                AdmittedToolPolicy::NoTools => {
                    if !registry.is_empty() {
                        return Err(failure(
                            BackendFailureKind::Initialization,
                            "no-tools/v1 requires an empty frozen tool registry",
                        ));
                    }
                    false
                },
            };
            let replay = match admitted.replay_profile() {
                AdmittedReplayProfile::SemanticOnly => ReplayProfile::SemanticOnly,
                AdmittedReplayProfile::ProviderPrivateLocalPlaintext => {
                    ReplayProfile::ProviderPrivateLocalPlaintext
                },
            };
            (tools, replay)
        } else {
            (!registry.is_empty(), ReplayProfile::SemanticOnly)
        };
        if config.system_prompt.is_empty()
            || config.maximum_model_rounds == 0
            || config.maximum_tool_argument_bytes == 0
            || config.maximum_tool_output_bytes < TOOL_TRUNCATION_MARKER.len()
            || config
                .absolute_tool_execution_timeout
                .is_some_and(|timeout| timeout.is_zero())
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
        let binding_identity = native_binding_identity(&binding, explicit_profile.as_ref())?;
        Ok(Self {
            connector,
            binding,
            binding_identity,
            registry,
            tool_exposure_enabled,
            semantic_admission: services.semantic_admission,
            tool_host: services.tool_host,
            config,
            model_context,
            token_counter: services.token_counter,
            request_observer: services.request_observer,
            contract,
            replay_profile,
            session: None,
            replay: ModelReplay::default(),
            replay_groups: Vec::new(),
            context_policy_active: false,
            turn: None,
            idle_compaction: None,
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
        BackendBindingEvidence::new(
            BACKEND_KIND,
            BACKEND_VERSION,
            self.binding_identity.clone(),
            BackendIdentity::new("yo.model-id/v1", self.binding.model_id().as_str()),
            BackendIdentity::new("yo.session-id/v1", session_id.to_string()),
            ContinuationStrategy::ExactReplay {
                executor: ReplayExecutor::LocalClient,
                replay_profile: self.replay_profile,
            },
        )
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
                update: yo_core::ActivityUpdate::TextSnapshot(text),
            });
        }
        if let Some(outcome) = finish {
            self.events
                .push_back(BackendEvent::ActivityFinished { activity, outcome });
        }
    }

    fn observe_connector_failure(&mut self, turn: TurnRef, error: &yo_core::ConnectorError) {
        if let Some(kind) = error.request_failure_kind() {
            self.observe_model_request(turn, yo_core::ModelRequestOutcome::Failed(kind));
        }
    }

    fn observe_model_request(&mut self, turn: TurnRef, outcome: yo_core::ModelRequestOutcome) {
        let Some(observer) = self.request_observer.as_mut() else {
            return;
        };
        let Err(detail) = observer.observe(outcome) else {
            return;
        };
        let Ok(activity) = self.next_activity(turn) else {
            return;
        };
        self.queue_activity_text(
            activity,
            ActivityKind::ModelWork,
            json!({
                "warning": "model request status was not saved",
                "detail": detail,
            })
            .to_string(),
            Some(ActivityOutcome::Failed(Failure::new(
                "recording model request status failed",
            ))),
        );
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
                BackendEvent::ActivityUpdated { .. }
                | BackendEvent::ContextPolicyChanged { .. }
                | BackendEvent::ContextCheckpointPrepared { .. }
                | BackendEvent::ContextActiveSuffixCompleted { .. }
                | BackendEvent::ModelRequestAccepted { .. } => {},
            }
        }
        activities
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
            BackendEvent::ActivityUpdated { .. }
            | BackendEvent::ContextPolicyChanged { .. }
            | BackendEvent::ContextCheckpointPrepared { .. }
            | BackendEvent::ContextActiveSuffixCompleted { .. }
            | BackendEvent::ModelRequestAccepted { .. } => {},
        }
        Some(event)
    }
}

fn map_connector_turn(error: yo_core::ConnectorError) -> BackendFailure {
    failure(BackendFailureKind::Turn, error.to_string())
}

fn map_connector_cleanup(error: yo_core::ConnectorError) -> BackendFailure {
    failure(BackendFailureKind::Cleanup, error.to_string())
}

fn map_tool_cleanup(_error: yo_core::ToolExecutionError) -> BackendFailure {
    failure(BackendFailureKind::Cleanup, "tool execution cleanup failed")
}

fn failure(kind: BackendFailureKind, message: impl Into<String>) -> BackendFailure {
    BackendFailure::new(kind, message)
}

#[cfg(test)]
mod tests;
