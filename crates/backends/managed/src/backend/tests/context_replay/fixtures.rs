use std::{
    collections::VecDeque,
    num::NonZeroU64,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use yo_core::{
    AgentEvent, AgentIntent, AgentSession, CommandAdmission, ContextPolicyChanged, ContextStrategy,
    JournalSequence, ModelConnectorEvent, ModelReplayContract, ModelReplayDelta, ModelReplayItem,
    ModelReplayRole, ProviderPrivateReplayEnvelope, ToolExecution, ToolExecutionError,
    ToolExecutionHost, ToolExecutionRequest, ToolId, TranscriptReader, TranscriptRecord, TurnRef,
};

use super::super::support::{completed, turn};
use crate::backend::{NativeModelBackend, NativeModelBackendConfig};

pub(super) struct RecordingTokenCounter {
    pub(super) input_tokens: u64,
    pub(super) payloads: Arc<Mutex<Vec<serde_json::Value>>>,
}

pub(super) fn pressure_at_hard_limit_config() -> NativeModelBackendConfig {
    NativeModelBackendConfig {
        context_policy: ContextPolicyChanged::try_new(
            1,
            true,
            ContextStrategy::PortableSummaryV1Alpha1,
            99,
            100,
            Some(10),
            Some(65_536),
        )
        .unwrap(),
        ..NativeModelBackendConfig::default()
    }
}

pub(super) fn portable_summary() -> String {
    [
        "# Context Checkpoint",
        "## Current Objective\nContinue the current task.",
        "## Active Constraints\nNone.",
        "## Decisions\nPreserve exact retained history.",
        "## Verified Progress\nTwo prior turns completed.",
        "## Current State\nA new turn is ready.",
        "## Unknown or Unverified\nNone.",
        "## Next Actions\nAnswer the current user input.",
        "## Critical References\nNone.",
    ]
    .join("\n")
}

pub(super) fn completed_text_round(response_id: &str, text: &str) -> Vec<ModelConnectorEvent> {
    vec![
        ModelConnectorEvent::ResponseCreated {
            response_id: response_id.to_owned(),
        },
        ModelConnectorEvent::TextDelta {
            output_index: 0,
            item_id: format!("{response_id}-item"),
            content_index: 0,
            delta: text.to_owned(),
        },
        ModelConnectorEvent::MessageDone {
            output_index: 0,
            item_id: format!("{response_id}-item"),
        },
        completed(response_id),
    ]
}

pub(super) fn completed_summary_round(
    response_id: &str,
    text: &str,
    output_index: usize,
) -> Vec<ModelConnectorEvent> {
    let mut events = completed_text_round(response_id, text);
    for event in &mut events {
        match event {
            ModelConnectorEvent::TextDelta {
                output_index: index,
                ..
            }
            | ModelConnectorEvent::MessageDone {
                output_index: index,
                ..
            } => *index = output_index,
            _ => {},
        }
    }
    if output_index > 0 {
        events.insert(
            1,
            ModelConnectorEvent::ReasoningDelta {
                output_index: 0,
                item_id: format!("{response_id}-reasoning"),
                channel: yo_core::ReasoningChannel::Text,
                part_index: 0,
                delta: "Synthetic reasoning excluded from the checkpoint.".to_owned(),
            },
        );
    }
    let Some(ModelConnectorEvent::Terminal { usage, .. }) = events.last_mut() else {
        unreachable!("완료 summary round는 terminal event로 끝납니다")
    };
    *usage = yo_core::ResponsesUsage {
        input_tokens: Some(20),
        output_tokens: Some(10),
        total_tokens: Some(30),
        reasoning_tokens: Some(0),
        cache_read_input_tokens: yo_core::CacheReadInputTokens::Unsupported,
    };
    events
}

pub(super) fn private_summary_event() -> ModelConnectorEvent {
    ModelConnectorEvent::ProviderPrivateAssistant {
        output_index: 1,
        envelope: ProviderPrivateReplayEnvelope::new(
            "kimi.assistant-message/v1alpha1",
            br#"{"role":"assistant","reasoning_content":"private","content":null}"#.to_vec(),
        )
        .unwrap(),
        visible_projection: Vec::new(),
    }
}

pub(super) fn turn_number(number: u64) -> TurnRef {
    TurnRef::new(
        turn().session_id(),
        yo_core::TurnId::new(NonZeroU64::new(number).unwrap()),
    )
}

pub(super) fn wait_for_turn_finish(
    session: &mut AgentSession,
    transcript: &TranscriptReader,
    cursor: &mut Option<JournalSequence>,
    expected_turn_id: u64,
) {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut finished = false;
    loop {
        let _ = session.poll().expect("the Session remains healthy");
        let slice = transcript.read_after(*cursor);
        if let Some(last) = slice.entries().last() {
            *cursor = Some(last.sequence());
        }
        finished |= slice.entries().iter().any(|entry| {
            matches!(
                entry.record(),
                TranscriptRecord::EventCommitted(AgentEvent::TurnFinished { turn, .. })
                    if turn.turn_id().get().get() == expected_turn_id
            )
        });
        if finished && session.is_idle_for_new_conversation() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the expected Turn did not finish and settle idle"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

pub(super) fn queue_intent(session: &mut AgentSession, intent: AgentIntent) {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut admission = session.dispatch(intent).unwrap();
    loop {
        match admission {
            CommandAdmission::Queued => return,
            CommandAdmission::Backpressured(pending) => {
                let _ = session.poll().expect("the Session remains healthy");
                admission = session.retry(pending).unwrap();
            },
            CommandAdmission::Rejected { rejection, .. } => {
                panic!("the test intent was rejected: {rejection:?}")
            },
        }
        assert!(Instant::now() < deadline, "the test intent stayed queued");
        thread::sleep(Duration::from_millis(1));
    }
}

pub(super) struct FailingStartHost;

impl ToolExecutionHost for FailingStartHost {
    fn identity(&self) -> &str {
        "failing-start-host-v1"
    }

    fn is_available(&self, _tool: &ToolId) -> bool {
        true
    }

    fn start(
        &mut self,
        _request: ToolExecutionRequest,
    ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
        Err(ToolExecutionError::new("injected start failure"))
    }

    fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
        Ok(())
    }
}

pub(super) struct SequenceTokenCounter {
    input_tokens: Mutex<VecDeque<u64>>,
    payloads: Arc<Mutex<Vec<serde_json::Value>>>,
}

pub(super) struct CapSensitiveTokenCounter {
    pub(super) payloads: Arc<Mutex<Vec<serde_json::Value>>>,
}

impl yo_core::ModelTokenCounter for CapSensitiveTokenCounter {
    fn count_input_tokens(
        &self,
        _tokenizer_profile: &str,
        request: &serde_json::Value,
    ) -> Result<u64, yo_core::ModelTokenCounterError> {
        self.payloads.lock().unwrap().push(request.clone());
        let input_text = request["input"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item["content"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let output_cap = request["max_output_tokens"].as_u64();
        let tokens = if input_text.contains("Create a faithful context checkpoint") {
            20
        } else if input_text.contains("# Context Checkpoint") {
            30
        } else if input_text.contains("input-3") && input_text.contains("first answer") {
            match output_cap {
                Some(10) => 96,
                Some(4) => 82,
                _ => 90,
            }
        } else if input_text.contains("input-3") {
            match output_cap {
                Some(10) => 10,
                _ => 20,
            }
        } else {
            10
        };
        Ok(tokens)
    }
}

impl SequenceTokenCounter {
    pub(super) fn new(
        input_tokens: impl IntoIterator<Item = u64>,
        payloads: Arc<Mutex<Vec<serde_json::Value>>>,
    ) -> Self {
        Self {
            input_tokens: Mutex::new(input_tokens.into_iter().collect()),
            payloads,
        }
    }
}

impl yo_core::ModelTokenCounter for SequenceTokenCounter {
    fn count_input_tokens(
        &self,
        _tokenizer_profile: &str,
        request: &serde_json::Value,
    ) -> Result<u64, yo_core::ModelTokenCounterError> {
        self.payloads.lock().unwrap().push(request.clone());
        Ok(self
            .input_tokens
            .lock()
            .unwrap()
            .pop_front()
            .expect("the test declared every exact payload count"))
    }
}

pub(super) fn tool_call_round() -> Vec<ModelConnectorEvent> {
    vec![
        ModelConnectorEvent::ResponseCreated {
            response_id: "tool".to_owned(),
        },
        ModelConnectorEvent::FunctionCallStarted {
            output_index: 0,
            item_id: "item-1".to_owned(),
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
        },
        ModelConnectorEvent::FunctionCallDone {
            output_index: 0,
            item_id: "item-1".to_owned(),
            call_id: "call-1".to_owned(),
            name: "read_file".to_owned(),
            arguments: r#"{"path":"README.md"}"#.to_owned(),
        },
        completed("tool"),
    ]
}

pub(super) fn retain_empty_assistant_items(backend: &mut NativeModelBackend, count: usize) {
    backend
        .replay
        .apply(&ModelReplayDelta::new(
            Some(ModelReplayContract::new("system", Vec::new())),
            (0..count)
                .map(|_| ModelReplayItem::Message {
                    role: ModelReplayRole::Assistant,
                    content: String::new(),
                    refusal: None,
                })
                .collect(),
        ))
        .unwrap();
}

pub(super) fn fill_current_delta_to_item_limit(backend: &mut NativeModelBackend) {
    let delta = &mut backend
        .turn
        .as_mut()
        .expect("the first model request opened a Turn")
        .delta;
    assert_eq!(delta.len(), 1);
    delta.extend((1..4_095).map(|_| ModelReplayItem::Message {
        role: ModelReplayRole::Assistant,
        content: String::new(),
        refusal: None,
    }));
    assert_eq!(delta.len(), 4_095);
}

impl yo_core::ModelTokenCounter for RecordingTokenCounter {
    fn count_input_tokens(
        &self,
        _tokenizer_profile: &str,
        request: &serde_json::Value,
    ) -> Result<u64, yo_core::ModelTokenCounterError> {
        self.payloads.lock().unwrap().push(request.clone());
        Ok(self.input_tokens)
    }
}
