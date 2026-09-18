use std::{
    collections::{HashMap, HashSet, VecDeque},
    num::NonZeroU64,
};

use serde_json::Value;
use yo_core::{
    ActivityApproval, ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef,
    ActivityUpdate, BackendEvent, BackendFailure, QuestionChoice, RequestId, SessionId, TurnRef,
    interview::{Answer, AnswerResponse, Capture},
};

use crate::{client::AcpClient, protocol, transport::JsonPeer};

pub(super) const MAX_ACP_IDENTIFIER_BYTES: usize = 4096;

pub(super) struct SessionBinding {
    pub(super) yo: SessionId,
    pub(super) grok: String,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum MessageChannel {
    Agent,
    Thought,
    Plan,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct MessageKey {
    pub(super) channel: MessageChannel,
    pub(super) message_id: Option<String>,
}

pub(super) struct MessageBinding {
    pub(super) activity: ActivityRef,
    pub(super) reasoning: Option<String>,
}

#[derive(Default)]
pub(super) struct ToolIdentity {
    pub(super) title: Option<String>,
    pub(super) name: Option<String>,
    pub(super) raw_input: Option<String>,
}

pub(super) struct ToolBinding {
    pub(super) activity: ActivityRef,
    pub(super) file_change: bool,
    pub(super) result_activity: Option<ActivityRef>,
    pub(super) output: Value,
    pub(super) identity: ToolIdentity,
    pub(super) finished: bool,
}

#[derive(Clone)]
pub(super) struct ApprovalBinding {
    pub(super) wire_id: Value,
    pub(super) activity: ActivityRef,
    pub(super) allow_option: String,
    pub(super) reject_option: String,
    pub(super) offered: Vec<Option<String>>,
    pub(super) profile: ActivityApproval,
    pub(super) tool_call_id: Option<String>,
    pub(super) pending_display: Option<Value>,
}

#[derive(Clone)]
pub(super) struct InputQuestion {
    pub(super) id: String,
    pub(super) text: String,
    pub(super) choices: Vec<QuestionChoice>,
    pub(super) previews: Vec<Option<String>>,
}

#[derive(Clone)]
pub(super) struct InputAnswer {
    pub(super) label: String,
    pub(super) preview: Option<String>,
    pub(super) notes: Option<String>,
}

#[derive(Clone)]
pub(super) struct InputQuestions {
    pub(super) questions: Vec<InputQuestion>,
    pub(super) current: usize,
    pub(super) answers: Vec<Option<InputAnswer>>,
    pub(super) drafts: Vec<(Option<u32>, String)>,
    pub(super) captured_answers: Vec<Option<(Answer, AnswerResponse)>>,
    pub(super) capture: Option<Capture>,
}

#[derive(Clone)]
pub(super) struct InputBinding {
    pub(super) wire_id: Value,
    pub(super) tool_call_id: String,
    pub(super) activity: ActivityRef,
    pub(super) questions: InputQuestions,
}

pub(super) struct PromptBinding {
    pub(super) request_id: u64,
    pub(super) turn: TurnRef,
    pub(super) interrupt_requested: bool,
}

pub(super) struct Backend<P> {
    pub(super) client: AcpClient<P>,
    pub(super) initialized: bool,
    pub(super) backend_version: Option<String>,
    pub(super) load_session: bool,
    pub(super) cwd: String,
    pub(super) read_only_review: bool,
    pub(super) session: Option<SessionBinding>,
    pub(super) prompt: Option<PromptBinding>,
    pub(super) messages: HashMap<MessageKey, MessageBinding>,
    pub(super) tools: HashMap<String, ToolBinding>,
    pub(super) seen_tool_ids: HashSet<String>,
    pub(super) approvals: HashMap<ActivityRequestRef, ApprovalBinding>,
    pub(super) wire_approvals: HashMap<String, ActivityRequestRef>,
    pub(super) inputs: HashMap<ActivityRequestRef, InputBinding>,
    pub(super) wire_inputs: HashMap<String, ActivityRequestRef>,
    pub(super) input_tool_turns: HashMap<String, TurnRef>,
    pub(super) pending_events: VecDeque<BackendEvent>,
    pub(super) next_activity_id: u64,
    pub(super) next_request_id: u64,
}

impl<P: JsonPeer> Backend<P> {
    pub(super) const MAX_ACTIVE_ACTIVITIES: usize = 1024;
    pub(super) const MAX_SESSION_TOOL_IDS: usize = 4096;

    pub(super) fn new_uninitialized(
        client: AcpClient<P>,
        cwd: String,
        read_only_review: bool,
    ) -> Self {
        Self {
            client,
            initialized: false,
            backend_version: None,
            load_session: false,
            cwd,
            read_only_review,
            session: None,
            prompt: None,
            messages: HashMap::new(),
            tools: HashMap::new(),
            seen_tool_ids: HashSet::new(),
            approvals: HashMap::new(),
            wire_approvals: HashMap::new(),
            inputs: HashMap::new(),
            wire_inputs: HashMap::new(),
            input_tool_turns: HashMap::new(),
            pending_events: VecDeque::new(),
            next_activity_id: 1,
            next_request_id: 1,
        }
    }

    pub(super) fn session_id(&self, session_id: SessionId) -> Result<&str, BackendFailure> {
        self.session
            .as_ref()
            .filter(|binding| binding.yo == session_id)
            .map(|binding| binding.grok.as_str())
            .ok_or_else(|| protocol::protocol_failure("Grok ACP Session binding was not found"))
    }

    pub(super) fn active_turn(&self) -> Result<TurnRef, BackendFailure> {
        self.prompt
            .as_ref()
            .map(|prompt| prompt.turn)
            .ok_or_else(|| protocol::protocol_failure("Grok ACP update has no active Turn"))
    }

    pub(super) fn ensure_activity_capacity(&self) -> Result<(), BackendFailure> {
        let active = self
            .messages
            .len()
            .checked_add(
                self.tools
                    .values()
                    .map(|binding| {
                        usize::from(!binding.finished)
                            + usize::from(!binding.finished && binding.result_activity.is_some())
                    })
                    .sum::<usize>(),
            )
            .and_then(|count| count.checked_add(self.approvals.len()))
            .and_then(|count| count.checked_add(self.inputs.len()))
            .ok_or_else(|| protocol::protocol_failure("Grok active activity count overflowed"))?;
        if active >= Self::MAX_ACTIVE_ACTIVITIES {
            return Err(protocol::protocol_failure(format!(
                "Grok ACP exceeded the per-Turn active activity limit of {}",
                Self::MAX_ACTIVE_ACTIVITIES
            )));
        }
        Ok(())
    }

    pub(super) fn next_activity(&mut self, turn: TurnRef) -> Result<ActivityRef, BackendFailure> {
        let id = NonZeroU64::new(self.next_activity_id)
            .map(ActivityId::new)
            .ok_or_else(|| protocol::protocol_failure("Grok Activity id space was exhausted"))?;
        self.next_activity_id = self
            .next_activity_id
            .checked_add(1)
            .ok_or_else(|| protocol::protocol_failure("Grok Activity id space was exhausted"))?;
        Ok(ActivityRef::new(turn, id))
    }

    pub(super) fn next_request(&mut self) -> Result<RequestId, BackendFailure> {
        let id = NonZeroU64::new(self.next_request_id)
            .map(RequestId::new)
            .ok_or_else(|| protocol::protocol_failure("Grok request id space was exhausted"))?;
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .ok_or_else(|| protocol::protocol_failure("Grok request id space was exhausted"))?;
        Ok(id)
    }

    pub(super) fn queue_usage_activity(
        &mut self,
        turn: TurnRef,
        receipt: Value,
    ) -> Result<(), BackendFailure> {
        let activity = self.next_activity(turn)?;
        self.pending_events
            .push_back(BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::ModelWork,
            });
        self.pending_events
            .push_back(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(receipt.to_string()),
            });
        self.pending_events
            .push_back(BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            });
        Ok(())
    }
}

pub(super) fn identifier_at<'a>(value: &'a Value, field: &str) -> Result<&'a str, BackendFailure> {
    let identifier = protocol::string_at(value, &[field])?;
    if identifier.is_empty() || identifier.len() > MAX_ACP_IDENTIFIER_BYTES {
        return Err(protocol::protocol_failure(format!(
            "Grok ACP field `{field}` is not a bounded identifier"
        )));
    }
    Ok(identifier)
}

pub(super) fn optional_identifier(
    value: &Value,
    field: &str,
) -> Result<Option<String>, BackendFailure> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => identifier_at(value, field).map(|identifier| Some(identifier.to_owned())),
    }
}

pub(super) fn wire_key(value: &Value) -> Result<String, BackendFailure> {
    serde_json::to_string(value)
        .map_err(|error| protocol::protocol_failure(format!("invalid Grok request id: {error}")))
}

pub(super) fn format_tool_summary(name: Option<&str>, raw_input: Option<&str>) -> Option<String> {
    match (name, raw_input) {
        (Some(name), Some(input)) => Some(format!("{name}: {input}")),
        (Some(name), None) => Some(name.to_owned()),
        (None, Some(input)) => Some(input.to_string()),
        (None, None) => None,
    }
}

pub(super) fn raw_input_summary(value: &Value) -> Option<String> {
    meaningful_raw_input(value).then(|| match value {
        Value::String(input) => input.trim().to_owned(),
        value => value.to_string(),
    })
}

pub(super) fn non_empty_text<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn meaningful_raw_input(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(input) => !input.trim().is_empty(),
        Value::Array(items) => items.iter().any(meaningful_raw_input),
        Value::Object(fields) => fields.values().any(meaningful_raw_input),
        Value::Bool(_) | Value::Number(_) => true,
    }
}
