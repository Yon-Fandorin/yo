/// Opaque provider-owned identity with an adapter-versioned interpretation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendIdentity {
    schema: String,
    value: String,
}

impl BackendIdentity {
    pub fn new(schema: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            schema: schema.into(),
            value: value.into(),
        }
    }

    pub fn schema(&self) -> &str {
        &self.schema
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub(crate) fn is_valid(&self) -> bool {
        valid_schema(&self.schema) && valid_value(&self.value)
    }
}

/// Adapter facts proving that a backend Session binding was created.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendBindingEvidence {
    backend_kind: String,
    backend_version: String,
    binding_identity: BackendIdentity,
    model_identity: BackendIdentity,
    session_locator: BackendIdentity,
    continuation_strategy: ContinuationStrategy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayExecutor {
    LocalClient,
    ManagedServer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContinuationStrategy {
    ExactReplay { executor: ReplayExecutor },
    BackendManagedState,
}

/// Durable provider-neutral coordinates required to reconnect one existing binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendResumeTarget {
    session_id: crate::SessionId,
    epoch: u64,
    binding: BackendBindingEvidence,
    model_replay: ModelReplay,
    source_anchor_sequence: crate::JournalSequence,
}

impl BackendResumeTarget {
    pub(crate) fn new(
        session_id: crate::SessionId,
        epoch: u64,
        binding: BackendBindingEvidence,
        source_anchor_sequence: crate::JournalSequence,
    ) -> Self {
        Self {
            session_id,
            epoch,
            binding,
            model_replay: ModelReplay::default(),
            source_anchor_sequence,
        }
    }

    #[must_use]
    pub const fn session_id(&self) -> crate::SessionId {
        self.session_id
    }

    #[must_use]
    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    #[must_use]
    pub const fn binding(&self) -> &BackendBindingEvidence {
        &self.binding
    }

    #[must_use]
    pub const fn model_replay(&self) -> &ModelReplay {
        &self.model_replay
    }

    #[must_use]
    pub const fn source_anchor_sequence(&self) -> crate::JournalSequence {
        self.source_anchor_sequence
    }

    pub(crate) fn with_model_replay(mut self, replay: ModelReplay) -> Self {
        self.model_replay = replay;
        self
    }
}

impl BackendBindingEvidence {
    pub fn new(
        backend_kind: impl Into<String>,
        backend_version: impl Into<String>,
        binding_identity: BackendIdentity,
        model_identity: BackendIdentity,
        session_locator: BackendIdentity,
        continuation_strategy: ContinuationStrategy,
    ) -> Self {
        Self {
            backend_kind: backend_kind.into(),
            backend_version: backend_version.into(),
            binding_identity,
            model_identity,
            session_locator,
            continuation_strategy,
        }
    }

    pub fn backend_kind(&self) -> &str {
        &self.backend_kind
    }

    pub fn backend_version(&self) -> &str {
        &self.backend_version
    }

    pub const fn binding_identity(&self) -> &BackendIdentity {
        &self.binding_identity
    }

    pub const fn model_identity(&self) -> &BackendIdentity {
        &self.model_identity
    }

    pub const fn session_locator(&self) -> &BackendIdentity {
        &self.session_locator
    }

    pub const fn continuation_strategy(&self) -> ContinuationStrategy {
        self.continuation_strategy
    }

    pub(crate) fn same_resume_identity(&self, other: &Self) -> bool {
        self.backend_kind == other.backend_kind
            && self.binding_identity == other.binding_identity
            && self.model_identity == other.model_identity
            && self.session_locator == other.session_locator
            && self.continuation_strategy == other.continuation_strategy
    }

    pub(crate) fn is_valid(&self) -> bool {
        valid_schema(&self.backend_kind)
            && valid_value(&self.backend_version)
            && self.binding_identity.is_valid()
            && self.model_identity.is_valid()
            && self.session_locator.is_valid()
    }
}

/// Adapter facts proving that one backend request was sent and accepted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendRequestEvidence {
    payload_schema: String,
    exchange_identity: BackendIdentity,
    request_identity: BackendIdentity,
}

impl BackendRequestEvidence {
    pub fn new(
        payload_schema: impl Into<String>,
        exchange_identity: BackendIdentity,
        request_identity: BackendIdentity,
    ) -> Self {
        Self {
            payload_schema: payload_schema.into(),
            exchange_identity,
            request_identity,
        }
    }

    pub fn payload_schema(&self) -> &str {
        &self.payload_schema
    }

    pub const fn exchange_identity(&self) -> &BackendIdentity {
        &self.exchange_identity
    }

    pub const fn request_identity(&self) -> &BackendIdentity {
        &self.request_identity
    }

    pub(crate) fn is_valid(&self) -> bool {
        valid_schema(&self.payload_schema)
            && self.exchange_identity.is_valid()
            && self.request_identity.is_valid()
    }
}

fn valid_schema(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && value.is_ascii()
}

fn valid_value(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4096
}

/// Provider-neutral evidence returned after command acceptance.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum BackendCommandEvidence {
    #[default]
    None,
    BindingOpened(BackendBindingEvidence),
    RequestAccepted(BackendRequestEvidence),
}

/// Provider evidence that a completed Turn is stable enough to resume.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BackendOutcomeEvidence {
    outcome_identity: Option<BackendIdentity>,
    model_replay: Option<ModelReplayDelta>,
}

impl BackendOutcomeEvidence {
    #[must_use]
    pub const fn without_identity() -> Self {
        Self {
            outcome_identity: None,
            model_replay: None,
        }
    }

    #[must_use]
    pub fn with_identity(identity: BackendIdentity) -> Self {
        Self {
            outcome_identity: Some(identity),
            model_replay: None,
        }
    }

    #[must_use]
    pub fn with_replay(mut self, replay: ModelReplayDelta) -> Self {
        self.model_replay = Some(replay);
        self
    }

    pub const fn outcome_identity(&self) -> Option<&BackendIdentity> {
        self.outcome_identity.as_ref()
    }

    pub const fn model_replay(&self) -> Option<&ModelReplayDelta> {
        self.model_replay.as_ref()
    }

    pub(crate) fn is_valid(&self) -> bool {
        self.outcome_identity
            .as_ref()
            .is_none_or(BackendIdentity::is_valid)
            && self
                .model_replay
                .as_ref()
                .is_none_or(ModelReplayDelta::is_valid)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelReplayRole {
    System,
    Developer,
    User,
    Assistant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelReplayTool {
    name: String,
    description: String,
    schema_version: String,
    parameters: Value,
}

impl ModelReplayTool {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        schema_version: impl Into<String>,
        parameters: Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            schema_version: schema_version.into(),
            parameters,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    pub const fn parameters(&self) -> &Value {
        &self.parameters
    }

    pub(crate) fn is_valid(&self) -> bool {
        valid_schema(&self.name)
            && valid_value(&self.description)
            && valid_schema(&self.schema_version)
            && self.parameters.is_object()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelReplayContract {
    system_prompt: String,
    tools: Vec<ModelReplayTool>,
}

impl ModelReplayContract {
    pub fn new(system_prompt: impl Into<String>, tools: Vec<ModelReplayTool>) -> Self {
        Self {
            system_prompt: system_prompt.into(),
            tools,
        }
    }

    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    pub fn tools(&self) -> &[ModelReplayTool] {
        &self.tools
    }

    pub(crate) fn is_valid(&self) -> bool {
        let mut names = std::collections::HashSet::new();
        !self.system_prompt.is_empty()
            && self.tools.len() <= 1_024
            && self.tools.iter().all(ModelReplayTool::is_valid)
            && self.tools.iter().all(|tool| names.insert(tool.name()))
            && encoded_contract_len(self) <= MAX_REPLAY_CONTRACT_BYTES
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelReplayItem {
    Message {
        role: ModelReplayRole,
        content: String,
    },
    FunctionCall {
        call_id: String,
        name: String,
        arguments: String,
    },
    FunctionCallOutput {
        call_id: String,
        output: String,
    },
}

impl ModelReplayItem {
    fn is_valid(&self) -> bool {
        match self {
            Self::Message { content, .. } => content.len() <= MAX_REPLAY_TEXT_BYTES,
            Self::FunctionCall {
                call_id,
                name,
                arguments,
            } => {
                valid_value(call_id)
                    && valid_schema(name)
                    && arguments.len() <= MAX_REPLAY_TEXT_BYTES
            },
            Self::FunctionCallOutput { call_id, output } => {
                valid_value(call_id) && output.len() <= MAX_REPLAY_TEXT_BYTES
            },
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModelReplayDelta {
    contract: Option<ModelReplayContract>,
    items: Vec<ModelReplayItem>,
}

impl ModelReplayDelta {
    pub fn new(contract: Option<ModelReplayContract>, items: Vec<ModelReplayItem>) -> Self {
        Self { contract, items }
    }

    pub const fn contract(&self) -> Option<&ModelReplayContract> {
        self.contract.as_ref()
    }

    pub fn items(&self) -> &[ModelReplayItem] {
        &self.items
    }

    pub(crate) fn is_valid(&self) -> bool {
        self.contract
            .as_ref()
            .is_none_or(ModelReplayContract::is_valid)
            && !self.items.is_empty()
            && self.items.len() <= MAX_REPLAY_ITEMS
            && self.items.iter().all(ModelReplayItem::is_valid)
            && encoded_delta_len(self) <= MAX_REPLAY_DELTA_BYTES
    }

    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        validate_replay_delta(self).map(|_| ())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModelReplay {
    contract: Option<ModelReplayContract>,
    items: Vec<ModelReplayItem>,
    encoded_prefix_bytes: usize,
    known_calls: std::collections::BTreeSet<String>,
    answered_calls: std::collections::BTreeSet<String>,
}

impl ModelReplay {
    pub const fn contract(&self) -> Option<&ModelReplayContract> {
        self.contract.as_ref()
    }

    pub fn items(&self) -> &[ModelReplayItem] {
        &self.items
    }

    pub(crate) fn apply(&mut self, delta: &ModelReplayDelta) -> Result<(), &'static str> {
        let (delta_calls, delta_answers) = validate_replay_delta(delta)?;
        match (self.contract.is_some(), delta.contract.is_some()) {
            (false, false) => return Err("first model replay delta requires its contract"),
            (true, true) => return Err("model replay contract was declared more than once"),
            (false, true) | (true, false) => {},
        }
        if self.items.len().saturating_add(delta.items.len()) > MAX_REPLAY_ITEMS {
            return Err("model replay item limit exceeded");
        }
        if delta_calls
            .iter()
            .any(|call_id| self.known_calls.contains(call_id))
        {
            return Err("model replay contains a duplicate function call identity");
        }
        if delta_answers
            .iter()
            .any(|call_id| self.answered_calls.contains(call_id))
        {
            return Err("model replay contains a duplicate function call output");
        }
        let encoded_prefix_bytes = if self.items.is_empty() {
            encoded_prefix_len(delta.contract.as_ref(), delta.items.iter())
        } else {
            self.encoded_prefix_bytes
                .saturating_add(encoded_items_len(&delta.items))
                .saturating_add(delta.items.len())
        };
        if encoded_prefix_bytes > MAX_REPLAY_PREFIX_BYTES {
            return Err("model replay prefix byte limit exceeded");
        }
        if let Some(contract) = &delta.contract {
            self.contract = Some(contract.clone());
        }
        self.items.extend(delta.items.iter().cloned());
        self.encoded_prefix_bytes = encoded_prefix_bytes;
        self.known_calls.extend(delta_calls);
        self.answered_calls.extend(delta_answers);
        Ok(())
    }
}

fn validate_replay_delta(
    delta: &ModelReplayDelta,
) -> Result<
    (
        std::collections::BTreeSet<String>,
        std::collections::BTreeSet<String>,
    ),
    &'static str,
> {
    if !delta.is_valid() {
        return Err("model replay delta is invalid or exceeds its bounds");
    }
    let mut known_calls = std::collections::BTreeSet::new();
    let mut answered_calls = std::collections::BTreeSet::new();
    for item in &delta.items {
        match item {
            ModelReplayItem::FunctionCall {
                call_id, arguments, ..
            } => {
                if serde_json::from_str::<Value>(arguments).is_err() {
                    return Err("model replay function arguments are not valid JSON");
                }
                if !known_calls.insert(call_id.clone()) {
                    return Err("model replay contains a duplicate function call identity");
                }
            },
            ModelReplayItem::FunctionCallOutput { call_id, .. } => {
                if !known_calls.contains(call_id) {
                    return Err("model replay output has no matching function call");
                }
                if !answered_calls.insert(call_id.clone()) {
                    return Err("model replay contains a duplicate function call output");
                }
            },
            ModelReplayItem::Message { .. } => {},
        }
    }
    if known_calls.difference(&answered_calls).next().is_some() {
        return Err("model replay delta ends with an unmatched function call");
    }
    Ok((known_calls, answered_calls))
}
use serde_json::{Value, json};

const MAX_REPLAY_ITEMS: usize = 4_096;
const MAX_REPLAY_TEXT_BYTES: usize = 16 * 1024 * 1024;
const MAX_REPLAY_CONTRACT_BYTES: usize = 1024 * 1024;
const MAX_REPLAY_DELTA_BYTES: usize = 16 * 1024 * 1024;
const MAX_REPLAY_PREFIX_BYTES: usize = 64 * 1024 * 1024;

fn encoded_contract_len(contract: &ModelReplayContract) -> usize {
    serde_json::to_vec(&contract_value(contract))
        .expect("a replay contract is always JSON serializable")
        .len()
}

fn encoded_delta_len(delta: &ModelReplayDelta) -> usize {
    let value = json!({
        "contract": delta.contract.as_ref().map(contract_value),
        "items": delta.items.iter().map(item_value).collect::<Vec<_>>(),
    });
    serde_json::to_vec(&value)
        .expect("a replay delta is always JSON serializable")
        .len()
}

fn encoded_prefix_len<'a>(
    contract: Option<&ModelReplayContract>,
    items: impl Iterator<Item = &'a ModelReplayItem>,
) -> usize {
    let value = json!({
        "contract": contract.map(contract_value),
        "items": items.map(item_value).collect::<Vec<_>>(),
    });
    serde_json::to_vec(&value)
        .expect("a replay prefix is always JSON serializable")
        .len()
}

fn encoded_items_len(items: &[ModelReplayItem]) -> usize {
    items
        .iter()
        .map(|item| {
            serde_json::to_vec(&item_value(item))
                .expect("a replay item is always JSON serializable")
                .len()
        })
        .sum()
}

fn contract_value(contract: &ModelReplayContract) -> Value {
    json!({
        "system_prompt": contract.system_prompt,
        "tools": contract.tools.iter().map(|tool| json!({
            "name": tool.name,
            "description": tool.description,
            "schema_version": tool.schema_version,
            "parameters": tool.parameters,
        })).collect::<Vec<_>>(),
    })
}

fn item_value(item: &ModelReplayItem) -> Value {
    match item {
        ModelReplayItem::Message { role, content } => json!({
            "kind": "message",
            "role": match role {
                ModelReplayRole::System => "system",
                ModelReplayRole::Developer => "developer",
                ModelReplayRole::User => "user",
                ModelReplayRole::Assistant => "assistant",
            },
            "content": content,
        }),
        ModelReplayItem::FunctionCall {
            call_id,
            name,
            arguments,
        } => json!({
            "kind": "function_call",
            "call_id": call_id,
            "name": name,
            "arguments": arguments,
        }),
        ModelReplayItem::FunctionCallOutput { call_id, output } => json!({
            "kind": "function_call_output",
            "call_id": call_id,
            "output": output,
        }),
    }
}

#[cfg(test)]
mod tests;
