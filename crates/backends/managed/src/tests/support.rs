use std::{
    collections::VecDeque,
    num::NonZeroU64,
    sync::{Arc, Mutex},
};

use serde_json::json;
use yo_core::{
    AccountId, ApiDialect, ModelId, NormalizedEndpoint, ProviderId, ToolDefinition, ToolEffect,
    ToolExecutionError, ToolExecutionResult, ToolId, ToolRegistry,
};

use super::super::{
    AgentBackend, BackendEvent, BackendPoll, EffectiveModelBinding, FrozenToolRegistry,
    ModelConnector, ModelConnectorCancellation, ModelConnectorEvent, ModelConnectorInputItem,
    ModelConnectorPoll, ModelConnectorRequest, ModelConnectorStreamPort, ModelConnectorTerminal,
    NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices,
    ToolApprovalRequirement, ToolExecution, ToolExecutionHost, ToolExecutionOutcome,
    ToolExecutionPoll, ToolExecutionRequest, ToolSemanticAdmission, TurnRef,
};
use crate::fixture_session;

pub(super) struct MockConnector {
    pub(super) rounds: Arc<Mutex<VecDeque<VecDeque<ModelConnectorEvent>>>>,
    pub(super) requests: Arc<Mutex<Vec<ModelConnectorRequest>>>,
}

impl ModelConnector for MockConnector {
    fn request_url(&self) -> &str {
        "https://example.invalid/v1/responses"
    }

    fn tokenization_payload(
        &self,
        request: &ModelConnectorRequest,
    ) -> Result<serde_json::Value, yo_core::ConnectorError> {
        Ok(mock_tokenization_payload(request, "mock-model"))
    }

    fn start(
        &self,
        request: ModelConnectorRequest,
        _cancellation: ModelConnectorCancellation,
    ) -> Result<Box<dyn ModelConnectorStreamPort>, yo_core::ConnectorError> {
        self.requests.lock().unwrap().push(request);
        let events = self
            .rounds
            .lock()
            .unwrap()
            .pop_front()
            .expect("the test declared every model round");
        Ok(Box::new(MockStream { events }))
    }
}

pub(super) fn mock_tokenization_payload(
    request: &ModelConnectorRequest,
    model: &str,
) -> serde_json::Value {
    let input = request
        .input()
        .iter()
        .map(|item| match item {
            ModelConnectorInputItem::Message {
                role,
                content,
                refusal,
            } => {
                let mut visible = content.clone();
                if let Some(refusal) = refusal {
                    visible.push_str(refusal);
                }
                json!({"role": role.as_str(), "content": visible})
            },
            ModelConnectorInputItem::FunctionCall {
                call_id,
                name,
                arguments,
            } => json!({
                "type": "function_call", "call_id": call_id,
                "name": name, "arguments": arguments,
            }),
            ModelConnectorInputItem::FunctionCallOutput { call_id, output } => json!({
                "type": "function_call_output", "call_id": call_id, "output": output,
            }),
            ModelConnectorInputItem::ProviderPrivateAssistant { envelope } => json!({
                "type": "provider_private_assistant", "schema": envelope.schema(),
            }),
        })
        .collect::<Vec<_>>();
    let mut body = json!({"model": model, "input": input, "stream": true});
    if let Some(maximum) = request.max_output_tokens() {
        body["max_output_tokens"] = serde_json::Value::from(maximum);
    }
    if let Some(tools) = request.tools() {
        body["tools"] = serde_json::Value::Array(
            tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function", "name": tool.name(),
                        "description": tool.description(), "parameters": tool.parameters(),
                    })
                })
                .collect(),
        );
        body["tool_choice"] = serde_json::Value::String("auto".to_owned());
    }
    if let Some(effort) = request.reasoning_effort() {
        body["reasoning"] = json!({"effort": effort.as_str()});
    }
    body
}

struct MockStream {
    events: VecDeque<ModelConnectorEvent>,
}

impl ModelConnectorStreamPort for MockStream {
    fn poll(&mut self) -> Result<ModelConnectorPoll, yo_core::ConnectorError> {
        Ok(self
            .events
            .pop_front()
            .map_or(ModelConnectorPoll::Closed, ModelConnectorPoll::Event))
    }

    fn cancel(&self) {}

    fn shutdown(&mut self) -> Result<(), yo_core::ConnectorError> {
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct MockHost {
    starts: Arc<Mutex<usize>>,
}

impl ToolExecutionHost for MockHost {
    fn identity(&self) -> &str {
        "test-host-v1"
    }

    fn is_available(&self, _tool: &ToolId) -> bool {
        true
    }

    fn start(
        &mut self,
        _request: ToolExecutionRequest,
    ) -> Result<Box<dyn ToolExecution>, ToolExecutionError> {
        *self.starts.lock().unwrap() += 1;
        Ok(Box::new(MockExecution {
            result: Some(ToolExecutionResult::new(
                ToolExecutionOutcome::Completed,
                r#"{"contents":"ok"}"#,
                false,
            )),
        }))
    }

    fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
        Ok(())
    }
}

pub(super) struct MockExecution {
    pub(super) result: Option<ToolExecutionResult>,
}

impl ToolExecution for MockExecution {
    fn poll(&mut self) -> Result<ToolExecutionPoll, ToolExecutionError> {
        Ok(ToolExecutionPoll::Ready)
    }

    fn take_result(&mut self) -> Option<ToolExecutionResult> {
        self.result.take()
    }

    fn cancel(&self) {}

    fn shutdown(&mut self) -> Result<(), ToolExecutionError> {
        Ok(())
    }
}

pub(super) fn binding() -> EffectiveModelBinding {
    EffectiveModelBinding::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("default").unwrap(),
        ModelId::new("qwen3.8max").unwrap(),
        ApiDialect::OpenAiResponses,
        NormalizedEndpoint::parse("https://example.invalid/v1").unwrap(),
    )
}

pub(super) fn registry(approval: ToolApprovalRequirement) -> FrozenToolRegistry {
    ToolRegistry::new([ToolDefinition::new(
        ToolId::new("read-file").unwrap(),
        "read_file",
        "reads a UTF-8 file",
        yo_core::TOOL_SCHEMA_DIALECT,
        json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"],
            "additionalProperties": false
        }),
        ToolEffect::ReadOnly,
        approval,
    )
    .unwrap()])
    .unwrap()
    .freeze()
}

pub(super) struct ExactAdmission;

impl ToolSemanticAdmission for ExactAdmission {
    fn admit_arguments(
        &self,
        _definition: &yo_core::ToolDefinition,
        validated_argument_bytes: &str,
    ) -> Result<String, yo_core::ToolSemanticAdmissionError> {
        Ok(validated_argument_bytes.to_owned())
    }

    fn admit_output(
        &self,
        _definition: &yo_core::ToolDefinition,
        bounded_output: &str,
    ) -> Result<String, yo_core::ToolSemanticAdmissionError> {
        Ok(bounded_output.to_owned())
    }
}

pub(super) fn context_profile() -> yo_core::ModelContextProfile {
    yo_core::ModelContextProfile::new(1_000_000, 4_096, "test-tokenizer/v1").unwrap()
}

pub(super) struct FixedTokenCounter(pub(super) u64);

impl yo_core::ModelTokenCounter for FixedTokenCounter {
    fn count_input_tokens(
        &self,
        _tokenizer_profile: &str,
        _request: &serde_json::Value,
    ) -> Result<u64, yo_core::ModelTokenCounterError> {
        Ok(self.0)
    }
}

pub(super) fn event_rounds(
    rounds: Vec<Vec<ModelConnectorEvent>>,
) -> Arc<Mutex<VecDeque<VecDeque<ModelConnectorEvent>>>> {
    Arc::new(Mutex::new(
        rounds
            .into_iter()
            .map(VecDeque::from)
            .collect::<VecDeque<_>>(),
    ))
}

pub(super) fn backend(
    rounds: Vec<Vec<ModelConnectorEvent>>,
    approval: ToolApprovalRequirement,
    starts: Arc<Mutex<usize>>,
) -> NativeModelBackend {
    NativeModelBackend::with_connector(
        Box::new(MockConnector {
            rounds: event_rounds(rounds),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        binding(),
        registry(approval),
        NativeModelBackendServices::new(
            Some(Box::new(ExactAdmission)),
            Box::new(MockHost { starts }),
            Box::new(FixedTokenCounter(1)),
        ),
        context_profile(),
        NativeModelBackendConfig::default(),
    )
    .unwrap()
}

pub(super) fn turn() -> TurnRef {
    TurnRef::new(
        fixture_session(44),
        yo_core::TurnId::new(NonZeroU64::new(1).unwrap()),
    )
}

pub(super) fn completed(response_id: &str) -> ModelConnectorEvent {
    ModelConnectorEvent::Terminal {
        response_id: response_id.to_owned(),
        status: ModelConnectorTerminal::Completed,
        usage: yo_core::ResponsesUsage::default(),
    }
}

pub(super) fn drain_until_turn(backend: &mut NativeModelBackend) -> BackendEvent {
    for _ in 0..100 {
        match backend.poll_event().unwrap() {
            BackendPoll::Event(
                event @ (BackendEvent::TurnFinished { .. }
                | BackendEvent::ResumableTurnFinished { .. }),
            ) => return event,
            BackendPoll::Event(_) | BackendPoll::Pending => {},
            BackendPoll::Closed => panic!("backend closed before finishing the Turn"),
        }
    }
    panic!("backend did not finish within the deterministic poll budget")
}
